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

func runKV(addr string) {
	fmt.Printf("── KV Benchmark (%s) ─────────────────────────\n", addr)
	fmt.Printf("clients=%d  ops/client=%d  pipeline_size=%d  total=%d\n\n",
		CLIENTS, OPS_CLIENT, PIPE_SIZE, CLIENTS*OPS_CLIENT)

	// ── Sequential SET (one op at a time per connection)
	var seqOps int64
	var wg sync.WaitGroup
	seqStart := time.Now()

	for i := 0; i < CLIENTS; i++ {
		wg.Add(1)
		go func(id int) {
			defer wg.Done()
			conn, err := net.Dial("tcp", addr)
			if err != nil {
				return
			}
			defer conn.Close()
			tcpConn := conn.(*net.TCPConn)
			tcpConn.SetNoDelay(true)

			w := bufio.NewWriterSize(conn, 65536)
			r := bufio.NewReaderSize(conn, 65536)
			var respBuf [64]byte

			base := id * OPS_CLIENT
			for j := 0; j < OPS_CLIENT; j++ {
				key := strconv.Itoa(base + j)
				writeSet(w, key, "value")
				w.Flush()
				// Read "+OK\r\n"
				readLine(r, respBuf[:])
				atomic.AddInt64(&seqOps, 1)
			}
		}(i)
	}
	wg.Wait()
	seqElapsed := time.Since(seqStart)
	printResult("Sequential SET", seqOps, seqElapsed)

	// ── Pipelined SET
	var pipeSetOps int64
	pipeSetStart := time.Now()

	for i := 0; i < CLIENTS; i++ {
		wg.Add(1)
		go func(id int) {
			defer wg.Done()
			conn, err := net.Dial("tcp", addr)
			if err != nil {
				return
			}
			defer conn.Close()
			tcpConn := conn.(*net.TCPConn)
			tcpConn.SetNoDelay(true)

			w := bufio.NewWriterSize(conn, 131072)
			r := bufio.NewReaderSize(conn, 65536)
			var respBuf [64]byte

			// Pre-build all keys for this client
			base := id * OPS_CLIENT
			keys := make([]string, OPS_CLIENT)
			for j := 0; j < OPS_CLIENT; j++ {
				keys[j] = "p:" + strconv.Itoa(base+j)
			}

			sent := 0
			for sent < OPS_CLIENT {
				batch := batchSize(sent, OPS_CLIENT, PIPE_SIZE)
				// Write batch
				for j := 0; j < batch; j++ {
					writeSet(w, keys[sent+j], "value")
				}
				w.Flush()
				// Read batch responses
				for j := 0; j < batch; j++ {
					readLine(r, respBuf[:])
				}
				atomic.AddInt64(&pipeSetOps, int64(batch))
				sent += batch
			}
		}(i)
	}
	wg.Wait()
	pipeSetElapsed := time.Since(pipeSetStart)
	printResult("Pipelined SET", pipeSetOps, pipeSetElapsed)

	// ── Pipelined GET
	var pipeGetOps int64
	pipeGetStart := time.Now()

	for i := 0; i < CLIENTS; i++ {
		wg.Add(1)
		go func(id int) {
			defer wg.Done()
			conn, err := net.Dial("tcp", addr)
			if err != nil {
				return
			}
			defer conn.Close()
			tcpConn := conn.(*net.TCPConn)
			tcpConn.SetNoDelay(true)

			w := bufio.NewWriterSize(conn, 131072)
			r := bufio.NewReaderSize(conn, 65536)
			var respBuf [64]byte

			// Pre-build all keys for this client
			base := id * OPS_CLIENT
			keys := make([]string, OPS_CLIENT)
			for j := 0; j < OPS_CLIENT; j++ {
				keys[j] = "p:" + strconv.Itoa(base+j)
			}

			sent := 0
			for sent < OPS_CLIENT {
				batch := batchSize(sent, OPS_CLIENT, PIPE_SIZE)
				// Write batch
				for j := 0; j < batch; j++ {
					writeGet(w, keys[sent+j])
				}
				w.Flush()
				// Read batch responses: "$5\r\nvalue\r\n" or "$-1\r\n"
				for j := 0; j < batch; j++ {
					line, _ := r.ReadBytes('\n')
					if len(line) > 0 && line[0] == '$' && line[1] != '-' {
						// Read the value line
						readLine(r, respBuf[:])
					}
				}
				atomic.AddInt64(&pipeGetOps, int64(batch))
				sent += batch
			}
		}(i)
	}
	wg.Wait()
	pipeGetElapsed := time.Since(pipeGetStart)
	printResult("Pipelined GET", pipeGetOps, pipeGetElapsed)

	seqRate := rate(seqOps, seqElapsed)
	setRate := rate(pipeSetOps, pipeSetElapsed)
	getRate := rate(pipeGetOps, pipeGetElapsed)

	fmt.Println("\n── KV Summary ──────────────────────────────────")
	fmt.Printf("sequential SET:   %s\n", fmtRate(seqRate))
	fmt.Printf("pipelined  SET:   %s\n", fmtRate(setRate))
	fmt.Printf("pipelined  GET:   %s\n", fmtRate(getRate))
	fmt.Printf("pipeline speedup: %.1fx\n", setRate/seqRate)
}

// ─── Raw RESP writers ───
// Pre-computed parts to minimize per-call work

var setPrefix = []byte("*3\r\n$3\r\nSET\r\n$")
var getPrefix = []byte("*2\r\n$3\r\nGET\r\n$")
var crlf = []byte("\r\n")
var valPart = []byte("\r\n$5\r\nvalue\r\n") // pre-computed for "value"

func writeSet(w *bufio.Writer, key, value string) {
	if value == "value" {
		// Ultra-fast path for the common benchmark value
		w.Write(setPrefix)
		w.WriteString(strconv.Itoa(len(key)))
		w.Write(crlf)
		w.WriteString(key)
		w.Write(valPart)
	} else {
		w.Write(setPrefix)
		w.WriteString(strconv.Itoa(len(key)))
		w.Write(crlf)
		w.WriteString(key)
		w.WriteString("\r\n$")
		w.WriteString(strconv.Itoa(len(value)))
		w.Write(crlf)
		w.WriteString(value)
		w.Write(crlf)
	}
}

func writeGet(w *bufio.Writer, key string) {
	w.Write(getPrefix)
	w.WriteString(strconv.Itoa(len(key)))
	w.Write(crlf)
	w.WriteString(key)
	w.Write(crlf)
}

func readLine(r *bufio.Reader, buf []byte) {
	r.ReadBytes('\n')
}

// ─── Helpers ───

func batchSize(sent, total, pipeSize int) int {
	if sent+pipeSize > total {
		return total - sent
	}
	return pipeSize
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
