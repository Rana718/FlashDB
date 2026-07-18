package main

import (
	"context"
	"flag"
	"fmt"
	"sync"
	"sync/atomic"
	"time"

	"github.com/redis/go-redis/v9"
)

const (
	HOST        = "127.0.0.1"
	CLIENTS     = 100
	OPS_CLIENT  = 10000
	PIPE_SIZE   = 100
)

func main() {
	ctx := context.Background()
 
	port := flag.Int("p", 8000, "port")
	flag.Parse()
	addr := fmt.Sprintf("%s:%d", HOST, *port)

	rdb := redis.NewClient(&redis.Options{
		Addr:         addr,
		PoolSize:     CLIENTS,
		MinIdleConns: CLIENTS,
	})

	// warm up
	if err := rdb.Ping(ctx).Err(); err != nil {
		fmt.Printf("could not connect: %v\n", err)
		return
	}

	fmt.Printf("connected to %s\n", addr)
	fmt.Printf("clients=%d  ops/client=%d  pipeline_size=%d  total=%d\n\n",
		CLIENTS, OPS_CLIENT, PIPE_SIZE, CLIENTS*OPS_CLIENT)

	// Sequential
	var seqOps int64
	seqStart := time.Now()
	var wg sync.WaitGroup

	for i := 0; i < CLIENTS; i++ {
		wg.Add(1)
		go func(id int) {
			defer wg.Done()
			for j := 0; j < OPS_CLIENT; j++ {
				key := fmt.Sprintf("%d", id*OPS_CLIENT+j)
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

	fmt.Println("── Sequential SET ──────────────────────────────")
	fmt.Printf("ops:     %d\n", seqOps)
	fmt.Printf("elapsed: %s\n", seqElapsed.Round(time.Millisecond))
	fmt.Printf("ops/sec: %.0f\n\n", float64(seqOps)/seqElapsed.Seconds())

	// Pipelined (stresses multi-thread model)
	var pipeOps int64
	pipeStart := time.Now()

	for i := 0; i < CLIENTS; i++ {
		wg.Add(1)
		go func(id int) {
			defer wg.Done()
			sent := 0
			for sent < OPS_CLIENT {
				batch := PIPE_SIZE
				if sent+batch > OPS_CLIENT {
					batch = OPS_CLIENT - sent
				}

				pipe := rdb.Pipeline()
				for j := 0; j < batch; j++ {
					key := fmt.Sprintf("p:%d", id*OPS_CLIENT+sent+j)
					pipe.Set(ctx, key, "value", 0)
				}
				if _, err := pipe.Exec(ctx); err != nil {
					fmt.Println("pipe error:", err)
					return
				}
				atomic.AddInt64(&pipeOps, int64(batch))
				sent += batch
			}
		}(i)
	}
	wg.Wait()
	pipeElapsed := time.Since(pipeStart)

	fmt.Println("── Pipelined SET ───────────────────────────────")
	fmt.Printf("ops:     %d\n", pipeOps)
	fmt.Printf("elapsed: %s\n", pipeElapsed.Round(time.Millisecond))
	fmt.Printf("ops/sec: %.0f\n\n", float64(pipeOps)/pipeElapsed.Seconds())

	// Pipelined GET
	var getOps int64
	getStart := time.Now()

	for i := 0; i < CLIENTS; i++ {
		wg.Add(1)
		go func(id int) {
			defer wg.Done()
			sent := 0
			for sent < OPS_CLIENT {
				batch := PIPE_SIZE
				if sent+batch > OPS_CLIENT {
					batch = OPS_CLIENT - sent
				}

				pipe := rdb.Pipeline()
				for j := 0; j < batch; j++ {
					key := fmt.Sprintf("p:%d", id*OPS_CLIENT+sent+j)
					pipe.Get(ctx, key)
				}
				if _, err := pipe.Exec(ctx); err != nil && err != redis.Nil {
					fmt.Println("get pipe error:", err)
					return
				}
				atomic.AddInt64(&getOps, int64(batch))
				sent += batch
			}
		}(i)
	}
	wg.Wait()
	getElapsed := time.Since(getStart)

	fmt.Println("── Pipelined GET ───────────────────────────────")
	fmt.Printf("ops:     %d\n", getOps)
	fmt.Printf("elapsed: %s\n", getElapsed.Round(time.Millisecond))
	fmt.Printf("ops/sec: %.0f\n\n", float64(getOps)/getElapsed.Seconds())

	
	// Summary
	fmt.Println("── Summary ─────────────────────────────────────")
	fmt.Printf("sequential SET:  %.0f ops/sec\n", float64(seqOps)/seqElapsed.Seconds())
	fmt.Printf("pipelined  SET:  %.0f ops/sec\n", float64(pipeOps)/pipeElapsed.Seconds())
	fmt.Printf("pipelined  GET:  %.0f ops/sec\n", float64(getOps)/getElapsed.Seconds())
	fmt.Printf("pipeline speedup: %.1fx\n", float64(pipeOps)/pipeElapsed.Seconds()/(float64(seqOps)/seqElapsed.Seconds()))
}
