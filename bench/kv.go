package main

import (
	"bufio"
	"fmt"
	"net"
	"strconv"
	"sync"
	"time"
)

const seqBatch = 64

func runKV() {
	label := addrs[0]
	if len(addrs) > 1 {
		label = fmt.Sprintf("cluster(%d masters)", len(addrs))
	}
	fmt.Printf("── KV Benchmark (%s) ─────────────────────────\n", label)
	fmt.Printf("clients=%d  ops/client=%d  pipeline_size=%d  total=%d\n\n",
		CLIENTS, OPS_CLIENT, PIPE_SIZE, CLIENTS*OPS_CLIENT)

	totalOps := int64(CLIENTS * OPS_CLIENT)

	seqConns := preDial(CLIENTS)
	pipeSetConns := preDial(CLIENTS)
	pipeGetConns := preDial(CLIENTS)
	defer closeAll(seqConns)
	defer closeAll(pipeSetConns)
	defer closeAll(pipeGetConns)

	var wg sync.WaitGroup
	seqStart := time.Now()

	for i := 0; i < CLIENTS; i++ {
		wg.Add(1)
		go func(id int, conn *net.TCPConn) {
			defer wg.Done()

			r := bufio.NewReaderSize(conn, 128<<10)

			var kb [32]byte
			requests := make([]byte, 0, seqBatch*40)
			base := id * OPS_CLIENT
			sent := 0
			for sent < OPS_CLIENT {
				batch := seqBatch
				if OPS_CLIENT-sent < batch {
					batch = OPS_CLIENT - sent
				}
				requests = requests[:0]
				for j := 0; j < batch; j++ {
					kn := strconv.AppendInt(kb[:0], int64(base+sent+j), 10)
					requests = appendSetBytes(requests, kn)
				}
				writeFull(conn, requests)
				discardN(r, batch*5)
				sent += batch
			}
		}(i, seqConns[i])
	}
	wg.Wait()
	seqElapsed := time.Since(seqStart)
	printResult("Pipeline-64 SET", totalOps, seqElapsed)

	pipeSetStart := time.Now()

	for i := 0; i < CLIENTS; i++ {
		wg.Add(1)
		go func(id int, conn *net.TCPConn) {
			defer wg.Done()

			r := bufio.NewReaderSize(conn, 128<<10)

			var kb [32]byte
			requests := make([]byte, 0, PIPE_SIZE*40)
			base := id * OPS_CLIENT
			sent := 0
			for sent < OPS_CLIENT {
				batch := PIPE_SIZE
				if OPS_CLIENT-sent < batch {
					batch = OPS_CLIENT - sent
				}
				requests = requests[:0]
				for j := 0; j < batch; j++ {
					kn := strconv.AppendInt(kb[:0], int64(base+sent+j), 10)
					requests = appendSetBytes(requests, kn)
				}
				writeFull(conn, requests)
				discardN(r, batch*5)
				sent += batch
			}
		}(i, pipeSetConns[i])
	}
	wg.Wait()
	pipeSetElapsed := time.Since(pipeSetStart)
	printResult("Pipelined SET", totalOps, pipeSetElapsed)

	pipeGetStart := time.Now()

	for i := 0; i < CLIENTS; i++ {
		wg.Add(1)
		go func(id int, conn *net.TCPConn) {
			defer wg.Done()

			r := bufio.NewReaderSize(conn, 256<<10)

			var kb [32]byte
			requests := make([]byte, 0, PIPE_SIZE*32)
			base := id * OPS_CLIENT
			sent := 0
			for sent < OPS_CLIENT {
				batch := PIPE_SIZE
				if OPS_CLIENT-sent < batch {
					batch = OPS_CLIENT - sent
				}
				requests = requests[:0]
				for j := 0; j < batch; j++ {
					kn := strconv.AppendInt(kb[:0], int64(base+sent+j), 10)
					requests = appendGetBytes(requests, kn)
				}
				writeFull(conn, requests)
				skipGetReplies(r, batch)
				sent += batch
			}
		}(i, pipeGetConns[i])
	}
	wg.Wait()
	pipeGetElapsed := time.Since(pipeGetStart)
	printResult("Pipelined GET", totalOps, pipeGetElapsed)

	seqRate := rate(totalOps, seqElapsed)
	setRate := rate(totalOps, pipeSetElapsed)
	getRate := rate(totalOps, pipeGetElapsed)

	fmt.Println("\n── KV Summary ──────────────────────────────────")
	fmt.Printf("pipeline-64 SET:   %s\n", fmtRate(seqRate))
	fmt.Printf("pipelined  SET:   %s\n", fmtRate(setRate))
	fmt.Printf("pipelined  GET:   %s\n", fmtRate(getRate))
	fmt.Printf("pipeline speedup: %.1fx\n", setRate/seqRate)
}

func preDial(n int) []*net.TCPConn {
	conns := make([]*net.TCPConn, n)
	var wg sync.WaitGroup
	for i := 0; i < n; i++ {
		wg.Add(1)
		go func(id int) {
			defer wg.Done()
			conns[id] = dialTCP(id)
		}(i)
	}
	wg.Wait()
	return conns
}

func closeAll(conns []*net.TCPConn) {
	for _, c := range conns {
		if c != nil {
			c.Close()
		}
	}
}

func dialTCP(id int) *net.TCPConn {
	c, err := net.Dial("tcp", pickAddr(id))
	if err != nil {
		panic(err)
	}
	tc := c.(*net.TCPConn)
	tc.SetNoDelay(true)
	tc.SetWriteBuffer(1 << 18)
	tc.SetReadBuffer(1 << 18)
	return tc
}

var (
	setHdr  = []byte("*3\r\n$3\r\nSET\r\n$")
	getHdr  = []byte("*2\r\n$3\r\nGET\r\n$")
	valPart = []byte("\r\n$5\r\nvalue\r\n")
	crlfB   = []byte("\r\n")
)

func appendSetBytes(out, key []byte) []byte {
	out = append(out, setHdr...)
	out = appendLen(out, len(key))
	out = append(out, crlfB...)
	out = append(out, key...)
	return append(out, valPart...)
}

func appendGetBytes(out, key []byte) []byte {
	out = append(out, getHdr...)
	out = appendLen(out, len(key))
	out = append(out, crlfB...)
	out = append(out, key...)
	return append(out, crlfB...)
}

func appendLen(out []byte, n int) []byte {
	if n < 10 {
		return append(out, byte('0'+n))
	}
	var buf [5]byte
	pos := len(buf)
	for n > 0 {
		pos--
		buf[pos] = byte('0' + n%10)
		n /= 10
	}
	return append(out, buf[pos:]...)
}

func writeFull(conn *net.TCPConn, p []byte) {
	for len(p) != 0 {
		n, err := conn.Write(p)
		if err != nil {
			panic(err)
		}
		p = p[n:]
	}
}

func discardN(r *bufio.Reader, n int) {
	remaining := n
	for remaining > 0 {
		d, _ := r.Discard(remaining)
		remaining -= d
	}
}

func skipGetReplies(r *bufio.Reader, n int) {
	for i := 0; i < n; i++ {
		b, err := r.ReadByte()
		if err != nil {
			return
		}
		switch b {
		case '$':
			line, _ := r.ReadSlice('\n')
			if len(line) >= 2 && line[0] == '-' {
				continue
			}
			vlen := 0
			for _, c := range line {
				if c >= '0' && c <= '9' {
					vlen = vlen*10 + int(c-'0')
				} else {
					break
				}
			}
			discardN(r, vlen+2)
		default:
			for {
				_, e := r.ReadSlice('\n')
				if e != bufio.ErrBufferFull {
					break
				}
			}
		}
	}
}

func skipLines(r *bufio.Reader, n int) {
	for i := 0; i < n; i++ {
		for {
			_, err := r.ReadSlice('\n')
			if err != bufio.ErrBufferFull {
				break
			}
		}
	}
}

func min(a, b int) int {
	if a < b {
		return a
	}
	return b
}

func rate(ops int64, d time.Duration) float64 {
	return float64(ops) / d.Seconds()
}

func fmtRate(r float64) string {
	switch {
	case r >= 1_000_000:
		return fmt.Sprintf("%.2fM ops/sec", r/1_000_000)
	case r >= 1_000:
		return fmt.Sprintf("%.1fk ops/sec", r/1_000)
	default:
		return fmt.Sprintf("%.0f ops/sec", r)
	}
}

func printResult(label string, ops int64, d time.Duration) {
	fmt.Printf("── %s\n", label)
	fmt.Printf("   ops:     %d\n", ops)
	fmt.Printf("   elapsed: %s\n", d.Round(time.Millisecond))
	fmt.Printf("   ops/sec: %s\n\n", fmtRate(rate(ops, d)))
}
