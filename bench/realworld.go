package main

import (
	"bufio"
	"fmt"
	"net"
	"strconv"
	"sync"
	"time"
)

const (
	MIX_OPS_CLIENT = 10000
	MIX_PIPE       = 100
	MIX_HOTKEY_OPS = 50000
	MIX_QUEUE_OPS  = 10000
)

var (
	incrHdr     = []byte("*2\r\n$4\r\nINCR\r\n$")
	hsetHdr     = []byte("*4\r\n$4\r\nHSET\r\n$")
	hgetHdr     = []byte("*3\r\n$4\r\nHGET\r\n$")
	lpushHdr    = []byte("*3\r\n$5\r\nLPUSH\r\n$")
	rpopHdr     = []byte("*2\r\n$4\r\nRPOP\r\n$")
	saddHdr     = []byte("*3\r\n$4\r\nSADD\r\n$")
	zaddHdr     = []byte("*4\r\n$4\r\nZADD\r\n$")
	expireHdr   = []byte("*3\r\n$6\r\nEXPIRE\r\n$")
	fieldSuffix = []byte("\r\n$1\r\nf\r\n$1\r\nv\r\n")
	fieldGet    = []byte("\r\n$1\r\nf\r\n")
	lpushBody   = []byte("\r\n$7\r\npayload\r\n")
	expireTTL   = []byte("\r\n$3\r\n300\r\n")
	flushCmd    = []byte("*1\r\n$8\r\nFLUSHALL\r\n")
)

type benchResult struct {
	label   string
	ops     int64
	elapsed time.Duration
}

func (b benchResult) opsPerSec() float64 {
	return float64(b.ops) / b.elapsed.Seconds()
}

var mixResults []benchResult

func recordResult(label string, ops int64, elapsed time.Duration) {
	printResult(label, ops, elapsed)
	mixResults = append(mixResults, benchResult{label, ops, elapsed})
}

func flushServer() {
	for _, addr := range addrs {
		conn, err := net.Dial("tcp", addr)
		if err != nil {
			continue
		}
		conn.Write(flushCmd)
		conn.Write([]byte("*1\r\n$4\r\nPING\r\n"))
		r := bufio.NewReaderSize(conn, 256)
		r.ReadSlice('\n')
		r.ReadSlice('\n')
		conn.Close()
	}
}

func runMix() {
	if len(addrs) > 1 {
		runMixCluster()
		return
	}

	fmt.Printf("── Mixed Workload Benchmark (%s) ───────────────\n", addrs[0])
	fmt.Printf("clients=%d  ops/client=%d  pipeline=%d\n\n", CLIENTS, MIX_OPS_CLIENT, MIX_PIPE)

	runBenchMixed()
	runBenchIncr()
	runBenchHash()
	runBenchList()
	runBenchSet()
	runBenchZSet()
	runBenchJson()
	runBenchExpire()
	runBenchHotKey()
	runBenchQueue()
}

func runMixCluster() {
	fmt.Printf("── Mixed Workload Benchmark (cluster %d masters) ────\n", len(addrs))
	fmt.Printf("clients=%d  ops/client=%d  pipeline=%d\n\n", CLIENTS, MIX_OPS_CLIENT, MIX_PIPE)

	runBenchMixed()
	runBenchIncr()
	runBenchHash()
	runBenchList()
	runBenchSet()
	runBenchZSet()
	runBenchJson()
	runBenchExpire()
	runBenchHotKey()
	runBenchQueue()
}

func printSummaryTable() {
	fmt.Println()
	fmt.Println("── Summary ─────────────────────────────────────")
	fmt.Printf("  %-32s %10s %14s\n", "Benchmark", "Elapsed", "Throughput")
	fmt.Printf("  %-32s %10s %14s\n", "────────────────────────────────", "──────────", "──────────────")
	for _, r := range mixResults {
		fmt.Printf("  %-32s %10s %14s\n", r.label, r.elapsed.Round(time.Millisecond), fmtRate(r.opsPerSec()))
	}
	fmt.Println()
}

func runBenchMixed() {
	conns := preDial(CLIENTS)
	defer closeAll(conns)
	total := int64(CLIENTS * MIX_OPS_CLIENT)

	start := time.Now()
	var wg sync.WaitGroup
	for i := range conns {
		wg.Add(1)
		go func(conn *net.TCPConn, id int) {
			defer wg.Done()
			r := bufio.NewReaderSize(conn, 256<<10)
			buf := make([]byte, 0, MIX_PIPE*48)
			var kb [32]byte
			base := id * MIX_OPS_CLIENT

			for sent := 0; sent < MIX_OPS_CLIENT; {
				batch := MIX_PIPE
				if MIX_OPS_CLIENT-sent < batch {
					batch = MIX_OPS_CLIENT - sent
				}
				buf = buf[:0]
				sets, gets := 0, 0
				for j := 0; j < batch; j++ {
					kn := strconv.AppendInt(kb[:0], int64(base+sent+j), 10)
					if (sent+j)&1 == 0 {
						buf = appendSetBytes(buf, kn)
						sets++
					} else {
						buf = appendGetBytes(buf, kn)
						gets++
					}
				}
				writeFull(conn, buf)
				discardN(r, sets*5)
				skipGetReplies(r, gets)
				sent += batch
			}
		}(conns[i], i)
	}
	wg.Wait()
	recordResult("Mixed SET/GET (50/50)", total, time.Since(start))
}

func runBenchIncr() {
	conns := preDial(CLIENTS)
	defer closeAll(conns)
	total := int64(CLIENTS * MIX_OPS_CLIENT)

	start := time.Now()
	var wg sync.WaitGroup
	for i := range conns {
		wg.Add(1)
		go func(conn *net.TCPConn, id int) {
			defer wg.Done()
			r := bufio.NewReaderSize(conn, 64<<10)
			buf := make([]byte, 0, MIX_PIPE*28)
			var kb [32]byte
			key := strconv.AppendInt(kb[:0], int64(id%100), 10)

			for sent := 0; sent < MIX_OPS_CLIENT; {
				batch := MIX_PIPE
				if MIX_OPS_CLIENT-sent < batch {
					batch = MIX_OPS_CLIENT - sent
				}
				buf = buf[:0]
				for j := 0; j < batch; j++ {
					buf = append(buf, incrHdr...)
					buf = appendLen(buf, len(key))
					buf = append(buf, crlfB...)
					buf = append(buf, key...)
					buf = append(buf, crlfB...)
				}
				writeFull(conn, buf)
				skipLines(r, batch)
				sent += batch
			}
		}(conns[i], i)
	}
	wg.Wait()
	recordResult("INCR (atomic counters)", total, time.Since(start))
}

func runBenchHash() {
	conns := preDial(CLIENTS)
	defer closeAll(conns)
	total := int64(CLIENTS * MIX_OPS_CLIENT)

	start := time.Now()
	var wg sync.WaitGroup
	for i := range conns {
		wg.Add(1)
		go func(conn *net.TCPConn, id int) {
			defer wg.Done()
			r := bufio.NewReaderSize(conn, 128<<10)
			buf := make([]byte, 0, MIX_PIPE*64)
			var kb [32]byte
			key := append([]byte("sess:"), strconv.AppendInt(kb[:0], int64(id), 10)...)

			for sent := 0; sent < MIX_OPS_CLIENT; {
				batch := MIX_PIPE
				if MIX_OPS_CLIENT-sent < batch {
					batch = MIX_OPS_CLIENT - sent
				}
				buf = buf[:0]
				hsets, hgets := 0, 0
				for j := 0; j < batch; j++ {
					if (sent+j)&1 == 0 {
						buf = append(buf, hsetHdr...)
						buf = appendLen(buf, len(key))
						buf = append(buf, crlfB...)
						buf = append(buf, key...)
						buf = append(buf, fieldSuffix...)
						hsets++
					} else {
						buf = append(buf, hgetHdr...)
						buf = appendLen(buf, len(key))
						buf = append(buf, crlfB...)
						buf = append(buf, key...)
						buf = append(buf, fieldGet...)
						hgets++
					}
				}
				writeFull(conn, buf)
				skipLines(r, hsets)
				skipGetReplies(r, hgets)
				sent += batch
			}
		}(conns[i], i)
	}
	wg.Wait()
	recordResult("HSET/HGET (sessions)", total, time.Since(start))
}

func runBenchList() {
	conns := preDial(CLIENTS)
	defer closeAll(conns)
	total := int64(CLIENTS * MIX_OPS_CLIENT)

	start := time.Now()
	var wg sync.WaitGroup
	for i := range conns {
		wg.Add(1)
		go func(conn *net.TCPConn, id int) {
			defer wg.Done()
			r := bufio.NewReaderSize(conn, 128<<10)
			buf := make([]byte, 0, MIX_PIPE*48)
			var kb [32]byte
			qkey := append([]byte("q:"), strconv.AppendInt(kb[:0], int64(id), 10)...)

			for sent := 0; sent < MIX_OPS_CLIENT; {
				batch := MIX_PIPE
				if MIX_OPS_CLIENT-sent < batch {
					batch = MIX_OPS_CLIENT - sent
				}
				buf = buf[:0]
				pushes, pops := 0, 0
				for j := 0; j < batch; j++ {
					if (sent+j)&1 == 0 {
						buf = append(buf, lpushHdr...)
						buf = appendLen(buf, len(qkey))
						buf = append(buf, crlfB...)
						buf = append(buf, qkey...)
						buf = append(buf, lpushBody...)
						pushes++
					} else {
						buf = append(buf, rpopHdr...)
						buf = appendLen(buf, len(qkey))
						buf = append(buf, crlfB...)
						buf = append(buf, qkey...)
						buf = append(buf, crlfB...)
						pops++
					}
				}
				writeFull(conn, buf)
				skipLines(r, pushes)
				skipGetReplies(r, pops)
				sent += batch
			}
		}(conns[i], i)
	}
	wg.Wait()
	recordResult("LPUSH/RPOP (queue)", total, time.Since(start))
}

func runBenchSet() {
	conns := preDial(CLIENTS)
	defer closeAll(conns)
	total := int64(CLIENTS * MIX_OPS_CLIENT)

	start := time.Now()
	var wg sync.WaitGroup
	for i := range conns {
		wg.Add(1)
		go func(conn *net.TCPConn, id int) {
			defer wg.Done()
			r := bufio.NewReaderSize(conn, 128<<10)
			buf := make([]byte, 0, MIX_PIPE*48)
			var kb, mb [32]byte
			setkey := append([]byte("s:"), strconv.AppendInt(kb[:0], int64(id), 10)...)

			for sent := 0; sent < MIX_OPS_CLIENT; {
				batch := MIX_PIPE
				if MIX_OPS_CLIENT-sent < batch {
					batch = MIX_OPS_CLIENT - sent
				}
				buf = buf[:0]
				for j := 0; j < batch; j++ {
					member := strconv.AppendInt(mb[:0], int64(sent+j), 10)
					buf = append(buf, saddHdr...)
					buf = appendLen(buf, len(setkey))
					buf = append(buf, crlfB...)
					buf = append(buf, setkey...)
					buf = append(buf, "\r\n$"...)
					buf = appendLen(buf, len(member))
					buf = append(buf, crlfB...)
					buf = append(buf, member...)
					buf = append(buf, crlfB...)
				}
				writeFull(conn, buf)
				skipLines(r, batch)
				sent += batch
			}
		}(conns[i], i)
	}
	wg.Wait()
	recordResult("SADD (per-client sets)", total, time.Since(start))
}

func runBenchZSet() {
	conns := preDial(CLIENTS)
	defer closeAll(conns)
	total := int64(CLIENTS * MIX_OPS_CLIENT)

	start := time.Now()
	var wg sync.WaitGroup
	for i := range conns {
		wg.Add(1)
		go func(conn *net.TCPConn, id int) {
			defer wg.Done()
			r := bufio.NewReaderSize(conn, 128<<10)
			buf := make([]byte, 0, MIX_PIPE*64)
			var kb, mb [32]byte
			lbkey := append([]byte("lb:"), strconv.AppendInt(kb[:0], int64(id), 10)...)

			for sent := 0; sent < MIX_OPS_CLIENT; {
				batch := MIX_PIPE
				if MIX_OPS_CLIENT-sent < batch {
					batch = MIX_OPS_CLIENT - sent
				}
				buf = buf[:0]
				for j := 0; j < batch; j++ {
					member := strconv.AppendInt(mb[:0], int64(sent+j), 10)
					buf = append(buf, zaddHdr...)
					buf = appendLen(buf, len(lbkey))
					buf = append(buf, crlfB...)
					buf = append(buf, lbkey...)
					buf = append(buf, "\r\n$"...)
					buf = appendLen(buf, len(member))
					buf = append(buf, crlfB...)
					buf = append(buf, member...)
					buf = append(buf, "\r\n$"...)
					buf = appendLen(buf, len(member))
					buf = append(buf, crlfB...)
					buf = append(buf, member...)
					buf = append(buf, crlfB...)
				}
				writeFull(conn, buf)
				skipLines(r, batch)
				sent += batch
			}
		}(conns[i], i)
	}
	wg.Wait()
	recordResult("ZADD (per-client zsets)", total, time.Since(start))
}

func runBenchExpire() {
	conns := preDial(CLIENTS)
	defer closeAll(conns)
	total := int64(CLIENTS * MIX_OPS_CLIENT)

	start := time.Now()
	var wg sync.WaitGroup
	for i := range conns {
		wg.Add(1)
		go func(conn *net.TCPConn, id int) {
			defer wg.Done()
			r := bufio.NewReaderSize(conn, 128<<10)
			buf := make([]byte, 0, MIX_PIPE*80)
			var kb [32]byte
			base := id * MIX_OPS_CLIENT

			for sent := 0; sent < MIX_OPS_CLIENT; {
				batch := MIX_PIPE
				if MIX_OPS_CLIENT-sent < batch {
					batch = MIX_OPS_CLIENT - sent
				}
				buf = buf[:0]
				for j := 0; j < batch; j++ {
					kn := strconv.AppendInt(kb[:0], int64(base+sent+j), 10)
					buf = appendSetBytes(buf, kn)
					buf = append(buf, expireHdr...)
					buf = appendLen(buf, len(kn))
					buf = append(buf, crlfB...)
					buf = append(buf, kn...)
					buf = append(buf, expireTTL...)
				}
				writeFull(conn, buf)
				skipLines(r, batch*2)
				sent += batch
			}
		}(conns[i], i)
	}
	wg.Wait()
	recordResult("SET+EXPIRE (cache TTL)", total, time.Since(start))
}

func runBenchHotKey() {
	conns := preDial(CLIENTS)
	defer closeAll(conns)
	total := int64(CLIENTS * MIX_HOTKEY_OPS)

	start := time.Now()
	var wg sync.WaitGroup
	hotkey := []byte("hot")
	for i := range conns {
		wg.Add(1)
		go func(conn *net.TCPConn) {
			defer wg.Done()
			r := bufio.NewReaderSize(conn, 64<<10)
			buf := make([]byte, 0, MIX_PIPE*32)

			for sent := 0; sent < MIX_HOTKEY_OPS; {
				batch := MIX_PIPE
				if MIX_HOTKEY_OPS-sent < batch {
					batch = MIX_HOTKEY_OPS - sent
				}
				buf = buf[:0]
				sets, gets := 0, 0
				for j := 0; j < batch; j++ {
					if (sent+j)&1 == 0 {
						buf = appendSetBytes(buf, hotkey)
						sets++
					} else {
						buf = appendGetBytes(buf, hotkey)
						gets++
					}
				}
				writeFull(conn, buf)
				discardN(r, sets*5)
				skipGetReplies(r, gets)
				sent += batch
			}
		}(conns[i])
	}
	wg.Wait()
	recordResult("Hot Key (1 key, contention)", total, time.Since(start))
}

func runBenchQueue() {
	half := CLIENTS / 2
	if half < 1 {
		half = 1
	}
	producers := preDial(half)
	consumers := preDial(half)
	defer closeAll(producers)
	defer closeAll(consumers)
	total := int64(half * MIX_QUEUE_OPS * 2)
	qkey := []byte("wq")

	start := time.Now()
	var wg sync.WaitGroup

	for i := 0; i < half; i++ {
		wg.Add(1)
		go func(conn *net.TCPConn) {
			defer wg.Done()
			r := bufio.NewReaderSize(conn, 64<<10)
			buf := make([]byte, 0, MIX_PIPE*40)

			for sent := 0; sent < MIX_QUEUE_OPS; {
				batch := MIX_PIPE
				if MIX_QUEUE_OPS-sent < batch {
					batch = MIX_QUEUE_OPS - sent
				}
				buf = buf[:0]
				for j := 0; j < batch; j++ {
					buf = append(buf, lpushHdr...)
					buf = appendLen(buf, len(qkey))
					buf = append(buf, crlfB...)
					buf = append(buf, qkey...)
					buf = append(buf, lpushBody...)
				}
				writeFull(conn, buf)
				skipLines(r, batch)
				sent += batch
			}
		}(producers[i])
	}

	for i := 0; i < half; i++ {
		wg.Add(1)
		go func(conn *net.TCPConn) {
			defer wg.Done()
			r := bufio.NewReaderSize(conn, 64<<10)
			buf := make([]byte, 0, MIX_PIPE*28)

			for sent := 0; sent < MIX_QUEUE_OPS; {
				batch := MIX_PIPE
				if MIX_QUEUE_OPS-sent < batch {
					batch = MIX_QUEUE_OPS - sent
				}
				buf = buf[:0]
				for j := 0; j < batch; j++ {
					buf = append(buf, rpopHdr...)
					buf = appendLen(buf, len(qkey))
					buf = append(buf, crlfB...)
					buf = append(buf, qkey...)
					buf = append(buf, crlfB...)
				}
				writeFull(conn, buf)
				skipGetReplies(r, batch)
				sent += batch
			}
		}(consumers[i])
	}

	wg.Wait()
	recordResult("Producer/Consumer (50+50)", total, time.Since(start))
}

func runBenchJson() {
	conns := preDial(CLIENTS)
	defer closeAll(conns)
	total := int64(CLIENTS * MIX_OPS_CLIENT)

	jsonSetHdr := []byte("*4\r\n$8\r\nJSON.SET\r\n$")
	jsonGetHdr := []byte("*3\r\n$8\r\nJSON.GET\r\n$")
	pathDot := []byte("\r\n$1\r\n.\r\n")
	pathDotVal := []byte("\r\n$1\r\n.\r\n$27\r\n{\"name\":\"test\",\"score\":100}\r\n")

	start := time.Now()
	var wg sync.WaitGroup
	for i := range conns {
		wg.Add(1)
		go func(conn *net.TCPConn, id int) {
			defer wg.Done()
			r := bufio.NewReaderSize(conn, 128<<10)
			buf := make([]byte, 0, MIX_PIPE*96)
			var kb [32]byte
			base := id * MIX_OPS_CLIENT

			for sent := 0; sent < MIX_OPS_CLIENT; {
				batch := MIX_PIPE
				if MIX_OPS_CLIENT-sent < batch {
					batch = MIX_OPS_CLIENT - sent
				}
				buf = buf[:0]
				sets, gets := 0, 0
				for j := 0; j < batch; j++ {
					kn := strconv.AppendInt(kb[:0], int64(base+sent+j), 10)
					if (sent+j)&1 == 0 {
						buf = append(buf, jsonSetHdr...)
						buf = appendLen(buf, len(kn))
						buf = append(buf, crlfB...)
						buf = append(buf, kn...)
						buf = append(buf, pathDotVal...)
						sets++
					} else {
						buf = append(buf, jsonGetHdr...)
						buf = appendLen(buf, len(kn))
						buf = append(buf, crlfB...)
						buf = append(buf, kn...)
						buf = append(buf, pathDot...)
						gets++
					}
				}
				writeFull(conn, buf)
				skipLines(r, sets)
				skipGetReplies(r, gets)
				sent += batch
			}
		}(conns[i], i)
	}
	wg.Wait()
	recordResult("JSON.SET/GET (documents)", total, time.Since(start))
}
