package main

import (
	"context"
	"fmt"
	"sync"
	"sync/atomic"
	"time"

	"github.com/redis/go-redis/v9"
)

const (
	ADDR       = "127.0.0.1:6379"
	CLIENTS    = 100
	OPS_CLIENT = 10000
)

func main() {
	ctx := context.Background()

	rdb := redis.NewClient(&redis.Options{
		Addr: ADDR,
	})

	var ops int64

	start := time.Now()

	var wg sync.WaitGroup

	for i := 0; i < CLIENTS; i++ {
		wg.Add(1)

		go func(id int) {
			defer wg.Done()

			for j := 0; j < OPS_CLIENT; j++ {
				key := fmt.Sprintf("%d", id*OPS_CLIENT+j)

				err := rdb.Set(
					ctx,
					key,
					"value",
					0,
				).Err()

				if err != nil {
					fmt.Println(err)
					return
				}

				atomic.AddInt64(&ops, 1)
			}
		}(i)
	}

	wg.Wait()

	elapsed := time.Since(start)

	fmt.Printf("Ops: %d\n", ops)
	fmt.Printf("Ops/sec: %.2f\n", float64(ops)/elapsed.Seconds())
}
