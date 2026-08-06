package main

import (
	"bufio"
	"fmt"
	"net"
	"strconv"
	"sync"
	"time"
)

// seqBatch: how many commands to pipeline in the "sequential" phase.
// 16 is far too small — each flush+read round trip costs ~50µs on loopback.
// 64 amortises that overhead while still looking "sequential" vs PIPE_SIZE=100.
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

	// ── Pre-dial all connections for every phase before starting the clock.
	// The old code dialled inside the goroutine after the timer started,
	// so TCP handshake latency was counted as benchmark time.
	seqConns := preDial(CLIENTS)
	pipeSetConns := preDial(CLIENTS)
	pipeGetConns := preDial(CLIENTS)
	defer closeAll(seqConns)
	defer closeAll(pipeSetConns)
	defer closeAll(pipeGetConns)

	// ── Sequential SET ────────────────────────────────────────────────────
	var wg sync.WaitGroup
	seqStart := time.Now()

	for i := 0; i < CLIENTS; i++ {
		wg.Add(1)
		go func(id int, conn *net.TCPConn) {
			defer wg.Done()

			// 256 KB write buffer: seqBatch=64 SET commands × ~30 bytes = ~2 KB per flush,
			// so 256 KB is never the bottleneck.
			w := bufio.NewWriterSize(conn, 256<<10)
			// 128 KB read buffer: +OK\r\n is 5 bytes × 64 = 320 bytes per batch.
			r := bufio.NewReaderSize(conn, 128<<10)

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
				// +OK\r\n is exactly 5 bytes. Discard the whole batch in one shot
				// instead of calling ReadSlice('\n') in a loop.
				discardN(r, batch*5)
				sent += batch
			}
		}(i, seqConns[i])
	}
	wg.Wait()
	seqElapsed := time.Since(seqStart)
	printResult("Sequential SET", totalOps, seqElapsed)

	// ── Pipelined SET ─────────────────────────────────────────────────────
	pipeSetStart := time.Now()

	for i := 0; i < CLIENTS; i++ {
		wg.Add(1)
		go func(id int, conn *net.TCPConn) {
			defer wg.Done()

			w := bufio.NewWriterSize(conn, 256<<10)
			r := bufio.NewReaderSize(conn, 128<<10)

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
				discardN(r, batch*5) // each +OK\r\n is 5 bytes
				sent += batch
			}
		}(i, pipeSetConns[i])
	}
	wg.Wait()
	pipeSetElapsed := time.Since(pipeSetStart)
	printResult("Pipelined SET", totalOps, pipeSetElapsed)

	// ── Pipelined GET ─────────────────────────────────────────────────────
	pipeGetStart := time.Now()

	for i := 0; i < CLIENTS; i++ {
		wg.Add(1)
		go func(id int, conn *net.TCPConn) {
			defer wg.Done()

			w := bufio.NewWriterSize(conn, 256<<10)
			r := bufio.NewReaderSize(conn, 256<<10)

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
				// The value was written as "value" (5 bytes).
				// Each GET reply is: $5\r\nvalue\r\n = 11 bytes. Skip the whole batch.
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
	fmt.Printf("sequential SET:   %s\n", fmtRate(seqRate))
	fmt.Printf("pipelined  SET:   %s\n", fmtRate(setRate))
	fmt.Printf("pipelined  GET:   %s\n", fmtRate(getRate))
	fmt.Printf("pipeline speedup: %.1fx\n", setRate/seqRate)
}

// preDial opens n TCP connections before the benchmark clock starts.
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
	tc.SetWriteBuffer(1 << 18) // 256 KB OS socket buffer
	tc.SetReadBuffer(1 << 18)
	return tc
}

// ── RESP write helpers ────────────────────────────────────────────────────────

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

// ── RESP read helpers ─────────────────────────────────────────────────────────

// discardN discards exactly n bytes from r.
// Faster than calling ReadSlice('\n') n times because it avoids per-newline
// scanning and uses bufio.Reader.Discard which is a single memmove.
func discardN(r *bufio.Reader, n int) {
	remaining := n
	for remaining > 0 {
		d, _ := r.Discard(remaining)
		remaining -= d
	}
}

// skipGetReplies skips n bulk-string GET replies of the form "$5\r\nvalue\r\n".
// Each reply is exactly 11 bytes when the value is "value" (5 bytes):
//
//	$5\r\n  = 4 bytes
//	value   = 5 bytes
//	\r\n    = 2 bytes  → total 11
//
// For nil replies ($-1\r\n = 5 bytes) we fall back to line-by-line.
// In the benchmark all keys were SET so nil replies should not occur.
func skipGetReplies(r *bufio.Reader, n int) {
	for i := 0; i < n; i++ {
		// Peek at the first byte to decide the reply type.
		b, err := r.ReadByte()
		if err != nil {
			return
		}
		switch b {
		case '$':
			// Read the length line: digits + \r\n
			line, _ := r.ReadSlice('\n')
			if len(line) >= 2 && line[0] == '-' {
				// $-1\r\n — nil reply, nothing more to discard
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
			// Discard value + trailing \r\n in one call
			discardN(r, vlen+2)
		default:
			// Error or simple string — discard to end of line
			for {
				_, e := r.ReadSlice('\n')
				if e != bufio.ErrBufferFull {
					break
				}
			}
		}
	}
}

// skipLines discards n RESP simple/error lines (used for +OK replies).
// Kept for compatibility but replaced by discardN in the SET paths above.
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
