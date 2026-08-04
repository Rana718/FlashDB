package main

import (
	"bufio"
	"fmt"
	"net"
	"strconv"
	"sync"
	"sync/atomic"
	"time"
)

const PUB_PIPE_SIZE = 100

func runPubSub(addr string) {
	channel := "bench:pubsub"

	totalMsgs := int64(PUB_PUBLISHERS * PUB_MSGS_EACH)
	totalExpected := int64(PUB_SUBSCRIBERS) * totalMsgs

	fmt.Printf("── Pub/Sub Benchmark (%s) ────────────────────\n", addr)
	fmt.Printf("subscribers=%d  publishers=%d  msgs/publisher=%d  pipe_size=%d\n",
		PUB_SUBSCRIBERS, PUB_PUBLISHERS, PUB_MSGS_EACH, PUB_PIPE_SIZE)
	fmt.Printf("total publishes=%d  total deliveries expected=%d\n\n",
		totalMsgs, totalExpected)

	var received int64
	var subReady sync.WaitGroup
	var recvDone sync.WaitGroup
	startSignal := make(chan struct{})

	subConns := make([]net.Conn, PUB_SUBSCRIBERS)
	for i := 0; i < PUB_SUBSCRIBERS; i++ {
		conn, err := net.Dial("tcp", addr)
		if err != nil {
			fmt.Printf("sub connect error: %v\n", err)
			return
		}
		conn.(*net.TCPConn).SetNoDelay(true)
		subConns[i] = conn

		w := bufio.NewWriter(conn)
		w.WriteString("*2\r\n$9\r\nSUBSCRIBE\r\n$")
		w.WriteString(strconv.Itoa(len(channel)))
		w.WriteString("\r\n")
		w.WriteString(channel)
		w.WriteString("\r\n")
		w.Flush()

		r := bufio.NewReaderSize(conn, 65536)
		for j := 0; j < 3; j++ {
			r.ReadBytes('\n')
		}
		r.ReadBytes('\n')
		r.ReadBytes('\n')
		r.ReadBytes('\n')

		subReady.Add(1)
		recvDone.Add(1)
		go func(conn net.Conn, r *bufio.Reader) {
			defer recvDone.Done()
			subReady.Done()
			<-startSignal

			var got int64
			for got < totalMsgs {
				line, err := r.ReadBytes('\n')
				if err != nil {
					break
				}
				if len(line) >= 2 && line[0] == '*' {
					r.ReadBytes('\n')
					r.ReadBytes('\n')
					r.ReadBytes('\n')
					r.ReadBytes('\n')
					r.ReadBytes('\n')
					r.ReadBytes('\n')

					got++
					atomic.AddInt64(&received, 1)
				}
			}
		}(conn, r)
	}

	subReady.Wait()
	time.Sleep(20 * time.Millisecond)
	close(startSignal)

	var published int64
	var pubWg sync.WaitGroup
	pubStart := time.Now()

	msg := "hello-pubsub-bench"

	for i := 0; i < PUB_PUBLISHERS; i++ {
		pubWg.Add(1)
		go func(id int) {
			defer pubWg.Done()
			conn, err := net.Dial("tcp", addr)
			if err != nil {
				return
			}
			defer conn.Close()
			conn.(*net.TCPConn).SetNoDelay(true)

			w := bufio.NewWriterSize(conn, 131072)
			r := bufio.NewReaderSize(conn, 65536)

			sent := 0
			for sent < PUB_MSGS_EACH {
				batch := PUB_MSGS_EACH - sent
				if batch > PUB_PIPE_SIZE {
					batch = PUB_PIPE_SIZE
				}
				for j := 0; j < batch; j++ {
					writePublish(w, channel, msg)
				}
				w.Flush()
				for j := 0; j < batch; j++ {
					r.ReadBytes('\n')
				}
				atomic.AddInt64(&published, int64(batch))
				sent += batch
			}
		}(i)
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

func writePublish(w *bufio.Writer, channel, message string) {
	w.WriteString("*3\r\n$7\r\nPUBLISH\r\n$")
	w.WriteString(strconv.Itoa(len(channel)))
	w.WriteString("\r\n")
	w.WriteString(channel)
	w.WriteString("\r\n$")
	w.WriteString(strconv.Itoa(len(message)))
	w.WriteString("\r\n")
	w.WriteString(message)
	w.WriteString("\r\n")
}
