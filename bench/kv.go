package main

import (
	"bufio"
	"fmt"
	"net"
	"strconv"
	"sync"
	"time"
)

const seqBatch = 16

func runKV() {
	label := addrs[0]
	if len(addrs) > 1 {
		label = fmt.Sprintf("cluster(%d masters)", len(addrs))
	}
	fmt.Printf("── KV Benchmark (%s) ─────────────────────────\n", label)
	fmt.Printf("clients=%d  ops/client=%d  pipeline_size=%d  total=%d\n\n",
		CLIENTS, OPS_CLIENT, PIPE_SIZE, CLIENTS*OPS_CLIENT)

	totalOps := int64(CLIENTS * OPS_CLIENT)


	var wg sync.WaitGroup
	seqStart := time.Now()

	for i := 0; i < CLIENTS; i++ {
		wg.Add(1)
		go func(id int) {
			defer wg.Done()
			conn := dialTCP(id)
			defer conn.Close()

			w := bufio.NewWriterSize(conn, 65536)
			r := bufio.NewReaderSize(conn, 65536)

			var kb [32]byte
			base := id * OPS_CLIENT
			sent := 0
			for sent < OPS_CLIENT {
				batch := min(seqBatch, OPS_CLIENT-sent)
				for j := 0; j < batch; j++ {
					kn := strconv.AppendInt(kb[:0], int64(base+sent+j), 10)
					writeSetBytes(w, kn)
				}
				w.Flush()
				skipLines(r, batch)
				sent += batch
			}
		}(i)
	}
	wg.Wait()
	seqElapsed := time.Since(seqStart)
	printResult("Sequential SET", totalOps, seqElapsed)


	pipeSetStart := time.Now()

	for i := 0; i < CLIENTS; i++ {
		wg.Add(1)
		go func(id int) {
			defer wg.Done()
			conn := dialTCP(id)
			defer conn.Close()

			w := bufio.NewWriterSize(conn, 262144)
			r := bufio.NewReaderSize(conn, 131072)

			var kb [32]byte
			base := id * OPS_CLIENT
			sent := 0
			for sent < OPS_CLIENT {
				batch := min(PIPE_SIZE, OPS_CLIENT-sent)
				for j := 0; j < batch; j++ {
					kn := strconv.AppendInt(kb[:0], int64(base+sent+j), 10)
					writeSetBytes(w, kn)
				}
				w.Flush()
				skipLines(r, batch)
				sent += batch
			}
		}(i)
	}
	wg.Wait()
	pipeSetElapsed := time.Since(pipeSetStart)
	printResult("Pipelined SET", totalOps, pipeSetElapsed)


	pipeGetStart := time.Now()

	for i := 0; i < CLIENTS; i++ {
		wg.Add(1)
		go func(id int) {
			defer wg.Done()
			conn := dialTCP(id)
			defer conn.Close()

			w := bufio.NewWriterSize(conn, 262144)
			r := bufio.NewReaderSize(conn, 131072)

			var kb [32]byte
			base := id * OPS_CLIENT
			sent := 0
			for sent < OPS_CLIENT {
				batch := min(PIPE_SIZE, OPS_CLIENT-sent)
				for j := 0; j < batch; j++ {
					kn := strconv.AppendInt(kb[:0], int64(base+sent+j), 10)
					writeGetBytes(w, kn)
				}
				w.Flush()
				readGetReplies(r, batch)
				sent += batch
			}
		}(i)
	}
	wg.Wait()
	pipeGetElapsed := time.Since(pipeGetStart)
	printResult("Pipelined GET", totalOps, pipeGetElapsed)

	seqRate := rate(totalOps, seqElapsed)
	setRate := rate(totalOps, pipeSetElapsed)
	getRate := rate(totalOps, pipeGetElapsed)

	fmt.Println("\n── KV Summary ──────────────────────────────────")
	fmt.Printf("sequential SET:   %s\n", fmtRate(seqRate))
	fmt.Printf("pipelined  SET:   %s\n", fmtRate(setRate))
	fmt.Printf("pipelined  GET:   %s\n", fmtRate(getRate))
	fmt.Printf("pipeline speedup: %.1fx\n", setRate/seqRate)
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

func writeSetBytes(w *bufio.Writer, key []byte) {
	w.Write(setHdr)
	writeLen(w, len(key))
	w.Write(crlfB)
	w.Write(key)
	w.Write(valPart)
}

func writeGetBytes(w *bufio.Writer, key []byte) {
	w.Write(getHdr)
	writeLen(w, len(key))
	w.Write(crlfB)
	w.Write(key)
	w.Write(crlfB)
}

func writeLen(w *bufio.Writer, n int) {
	if n < 10 {
		w.WriteByte(byte('0' + n))
		return
	}
	var buf [5]byte
	pos := len(buf)
	for n > 0 {
		pos--
		buf[pos] = byte('0' + n%10)
		n /= 10
	}
	w.Write(buf[pos:])
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

func readGetReplies(r *bufio.Reader, n int) {
	for i := 0; i < n; i++ {
	
		b, err := r.ReadByte()
		if err != nil {
			return
		}
		if b != '$' {
		
			for {
				_, e := r.ReadSlice('\n')
				if e != bufio.ErrBufferFull {
					break
				}
			}
			continue
		}
	
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
		r.Discard(vlen + 2)
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
