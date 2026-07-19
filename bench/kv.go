package main

import (
	"context"
	"fmt"
	"sync"
	"sync/atomic"
	"time"

	"github.com/redis/go-redis/v9"
)

func runKV(rdb *redis.Client, addr string) {
	ctx := context.Background()

	fmt.Printf("── KV Benchmark (%s) ─────────────────────────\n", addr)
	fmt.Printf("clients=%d  ops/client=%d  pipeline_size=%d  total=%d\n\n",
		CLIENTS, OPS_CLIENT, PIPE_SIZE, CLIENTS*OPS_CLIENT)

	var seqOps int64
	var wg sync.WaitGroup
	seqStart := time.Now()

	for i := 0; i < CLIENTS; i++ {
		wg.Add(1)
		go func(id int) {
			defer wg.Done()
			for j := 0; j < OPS_CLIENT; j++ {
				key := fmt.Sprintf("seq:%d", id*OPS_CLIENT+j)
				if err := rdb.Set(ctx, key, "value", 0).Err(); err != nil {
					fmt.Println("seq error:", err)
					return
				}
				atomic.AddInt64(&seqOps, 1)
			}
		}(i)
	}
	wg.Wait()
	seqElapsed := time.Since(seqStart)

	printResult("Sequential SET", seqOps, seqElapsed)

	var pipeSetOps int64
	pipeSetStart := time.Now()

	for i := 0; i < CLIENTS; i++ {
		wg.Add(1)
		go func(id int) {
			defer wg.Done()
			sent := 0
			for sent < OPS_CLIENT {
				batch := batchSize(sent, OPS_CLIENT, PIPE_SIZE)
				pipe := rdb.Pipeline()
				for j := 0; j < batch; j++ {
					pipe.Set(ctx, fmt.Sprintf("pipe:%d", id*OPS_CLIENT+sent+j), "value", 0)
				}
				if _, err := pipe.Exec(ctx); err != nil {
					fmt.Println("pipe set error:", err)
					return
				}
				atomic.AddInt64(&pipeSetOps, int64(batch))
				sent += batch
			}
		}(i)
	}
	wg.Wait()
	pipeSetElapsed := time.Since(pipeSetStart)

	printResult("Pipelined SET", pipeSetOps, pipeSetElapsed)

	var pipeGetOps int64
	pipeGetStart := time.Now()

	for i := 0; i < CLIENTS; i++ {
		wg.Add(1)
		go func(id int) {
			defer wg.Done()
			sent := 0
			for sent < OPS_CLIENT {
				batch := batchSize(sent, OPS_CLIENT, PIPE_SIZE)
				pipe := rdb.Pipeline()
				for j := 0; j < batch; j++ {
					pipe.Get(ctx, fmt.Sprintf("pipe:%d", id*OPS_CLIENT+sent+j))
				}
				if _, err := pipe.Exec(ctx); err != nil && err != redis.Nil {
					fmt.Println("pipe get error:", err)
					return
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
