package main

import (
	"bufio"
	"bytes"
	"fmt"
	"net"
	"strconv"
	"sync"
	"sync/atomic"
	"time"
)

const PUB_PIPE_SIZE = 200

func runPubSub(addr string) {
	channel := "bench:pubsub"

	totalMsgs := int64(PUB_PUBLISHERS * PUB_MSGS_EACH)
	totalExpected := int64(PUB_SUBSCRIBERS) * totalMsgs

	fmt.Printf("── Pub/Sub Benchmark (%s) ────────────────────\n", addr)
	fmt.Printf("subscribers=%d  publishers=%d  msgs/publisher=%d  pipe_size=%d\n",
		PUB_SUBSCRIBERS, PUB_PUBLISHERS, PUB_MSGS_EACH, PUB_PIPE_SIZE)
	fmt.Printf("total publishes=%d  total deliveries expected=%d\n\n",
		totalMsgs, totalExpected)

	// ── Pre-build the message payload. Frame size is no longer needed
	// since we count markers directly on raw reads.
	msgPayload := "hello-pubsub-bench-msg"

	var received int64

	// startSignal gates all subscriber goroutines at once so they don't miss
	// early messages. Using a channel is cleaner than time.Sleep.
	startSignal := make(chan struct{})

	// subReady counts how many subscriber goroutines have confirmed subscription
	// and are blocked on startSignal — i.e. truly ready to receive.
	var subReady sync.WaitGroup
	var recvDone sync.WaitGroup

	subCmd := buildSubCmd(channel)

	// ── Connect and subscribe all subscribers BEFORE the benchmark clock.
	subConns := make([]net.Conn, PUB_SUBSCRIBERS)
	for i := 0; i < PUB_SUBSCRIBERS; i++ {
		conn, err := net.Dial("tcp", addr)
		if err != nil {
			fmt.Printf("sub connect error: %v\n", err)
			return
		}
		tc := conn.(*net.TCPConn)
		tc.SetNoDelay(true)
		tc.SetReadBuffer(1 << 20) // 1 MB: subscribers receive fan-out, needs headroom
		subConns[i] = conn

		conn.Write(subCmd)

		// Consume the SUBSCRIBE confirmation reply using a tiny bufio.Reader
		// with a buffer exactly the size of the confirm frame.
		// We use a 512-byte buffer so it cannot buffer-ahead into message data —
		// the confirm frame is at most ~50 bytes, well under 512.
		// After consumeSubConfirm the bufio reader is discarded and the goroutine
		// reads from the raw conn directly.
		r := bufio.NewReaderSize(conn, 512)
		consumeSubConfirm(r)
		// Drain any bytes consumeSubConfirm left in the bufio buffer back into
		// a scratch slice so the raw conn.Read in the goroutine starts clean.
		if r.Buffered() > 0 {
			tmp := make([]byte, r.Buffered())
			r.Read(tmp) // discard — these are stale bytes past the confirm frame
		}

		subReady.Add(1)
		recvDone.Add(1)
		go func(conn net.Conn) {
			defer recvDone.Done()
			subReady.Done()
			<-startSignal

			// Raw read loop: read whatever the kernel gives us into a fixed
			// stack-allocated buffer, then count "*3\r\n" markers with bytes.Count.
			// This is the fastest possible approach:
			//   - One Read() call drains the entire socket receive buffer
			//   - bytes.Count uses SIMD-accelerated memmem internally in Go
			//   - Zero per-message overhead — we never parse individual frames
			//   - No bufio overhead — direct net.Conn read
			var buf [1 << 17]byte // 128 KB — matches typical socket buffer fill
			marker := []byte("*3\r\n")
			var localCount int64
			for localCount < totalMsgs {
				n, err := conn.Read(buf[:])
				if n > 0 {
					localCount += int64(bytes.Count(buf[:n], marker))
				}
				if err != nil {
					break
				}
			}
			atomic.AddInt64(&received, localCount)
		}(conn)
	}

	// Wait until all subscriber goroutines are alive and blocked on startSignal.
	subReady.Wait()

	// ── Pre-dial all publisher connections before opening the clock.
	pubConns := make([]*net.TCPConn, PUB_PUBLISHERS)
	for i := 0; i < PUB_PUBLISHERS; i++ {
		conn, err := net.Dial("tcp", addr)
		if err != nil {
			fmt.Printf("pub connect error: %v\n", err)
			return
		}
		tc := conn.(*net.TCPConn)
		tc.SetNoDelay(true)
		tc.SetWriteBuffer(1 << 19) // 512 KB
		tc.SetReadBuffer(1 << 18)  // 256 KB
		pubConns[i] = tc
	}

	pubCmd := buildPubCmd(channel, msgPayload)

	// ── Release all subscribers and start the clock simultaneously.
	close(startSignal)
	pubStart := time.Now()

	var published int64
	var pubWg sync.WaitGroup

	for i := 0; i < PUB_PUBLISHERS; i++ {
		pubWg.Add(1)
		go func(conn *net.TCPConn) {
			defer pubWg.Done()
			defer conn.Close()

			w := bufio.NewWriterSize(conn, 256<<10)
			r := bufio.NewReaderSize(conn, 128<<10)

			// PUBLISH reply is ":N\r\n" (integer). Each one is small.
			// We discard them in bulk: ":50\r\n" = 6 bytes, ":5\r\n" = 5 bytes.
			// Use skipLines which handles variable-length integer lines correctly.
			sent := 0
			for sent < PUB_MSGS_EACH {
				batch := min(PUB_PIPE_SIZE, PUB_MSGS_EACH-sent)
				for j := 0; j < batch; j++ {
					w.Write(pubCmd)
				}
				w.Flush()
				skipLines(r, batch)
				sent += batch
			}
			atomic.AddInt64(&published, int64(PUB_MSGS_EACH))
		}(pubConns[i])
	}
	pubWg.Wait()
	pubElapsed := time.Since(pubStart)

	recvDone.Wait()
	recvElapsed := time.Since(pubStart)

	for _, c := range subConns {
		c.Close()
	}

	pubRate := rate(published, pubElapsed)
	deliveryRate := rate(atomic.LoadInt64(&received), recvElapsed)

	fmt.Printf("── Publish throughput\n")
	fmt.Printf("   published: %d\n", published)
	fmt.Printf("   elapsed:   %s\n", pubElapsed.Round(time.Millisecond))
	fmt.Printf("   ops/sec:   %s\n\n", fmtRate(pubRate))

	fmt.Printf("── End-to-end delivery\n")
	fmt.Printf("   delivered:      %d / %d\n", atomic.LoadInt64(&received), totalExpected)
	fmt.Printf("   e2e elapsed:    %s\n", recvElapsed.Round(time.Millisecond))
	fmt.Printf("   delivery rate:  %s\n\n", fmtRate(deliveryRate))

	fmt.Println("── Pub/Sub Summary ─────────────────────────────")
	fmt.Printf("publish throughput:   %s\n", fmtRate(pubRate))
	fmt.Printf("delivery throughput:  %s\n", fmtRate(deliveryRate))
	fmt.Printf("fan-out factor:       %dx  (%d subs × %d msgs)\n",
		PUB_SUBSCRIBERS, PUB_SUBSCRIBERS, totalMsgs)
}

// consumeSubConfirm reads and discards a single SUBSCRIBE confirmation frame.
// This is more robust than counting 6 raw lines because it parses the array
// header and uses exact bulk-string lengths, so it works regardless of channel
// name length.
func consumeSubConfirm(r *bufio.Reader) {
	// *3\r\n
	r.ReadSlice('\n')
	// $9\r\nsubscribe\r\n
	r.ReadSlice('\n') // $9\r\n
	r.ReadSlice('\n') // subscribe\r\n
	// $<n>\r\n<channel>\r\n — read length, then discard exact bytes
	line, _ := r.ReadSlice('\n') // $<n>\r\n
	chanLen := 0
	for _, c := range line {
		if c >= '0' && c <= '9' {
			chanLen = chanLen*10 + int(c-'0')
		} else if c == '$' {
			// skip
		} else {
			break
		}
	}
	discardN(r, chanLen+2) // channel bytes + \r\n
	// :<count>\r\n
	r.ReadSlice('\n')
}

func buildSubCmd(channel string) []byte {
	b := make([]byte, 0, 64)
	b = append(b, "*2\r\n$9\r\nSUBSCRIBE\r\n$"...)
	b = strconv.AppendInt(b, int64(len(channel)), 10)
	b = append(b, "\r\n"...)
	b = append(b, channel...)
	b = append(b, "\r\n"...)
	return b
}

func buildPubCmd(channel, message string) []byte {
	b := make([]byte, 0, 64+len(channel)+len(message))
	b = append(b, "*3\r\n$7\r\nPUBLISH\r\n$"...)
	b = strconv.AppendInt(b, int64(len(channel)), 10)
	b = append(b, "\r\n"...)
	b = append(b, channel...)
	b = append(b, "\r\n$"...)
	b = strconv.AppendInt(b, int64(len(message)), 10)
	b = append(b, "\r\n"...)
	b = append(b, message...)
	b = append(b, "\r\n"...)
	return b
}
