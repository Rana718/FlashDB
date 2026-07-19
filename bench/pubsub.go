package main

import (
	"context"
	"fmt"
	"sync"
	"sync/atomic"
	"time"

	"github.com/redis/go-redis/v9"
)

func runPubSub(addr string) {
	ctx := context.Background()
	channel := "bench:pubsub"

	totalMsgs := int64(PUB_PUBLISHERS * PUB_MSGS_EACH)
	expectedPerSub := totalMsgs
	totalExpected := int64(PUB_SUBSCRIBERS) * expectedPerSub

	fmt.Printf("── Pub/Sub Benchmark (%s) ────────────────────\n", addr)
	fmt.Printf("subscribers=%d  publishers=%d  msgs/publisher=%d\n",
		PUB_SUBSCRIBERS, PUB_PUBLISHERS, PUB_MSGS_EACH)
	fmt.Printf("total publishes=%d  total deliveries expected=%d\n\n",
		totalMsgs, totalExpected)

	var received int64
	var subWg sync.WaitGroup
	var recvWg sync.WaitGroup
	ready := make(chan struct{})

	subClients := make([]*redis.Client, PUB_SUBSCRIBERS)
	subReadyCh := make(chan struct{}, PUB_SUBSCRIBERS)

	for i := 0; i < PUB_SUBSCRIBERS; i++ {
		subClients[i] = redis.NewClient(&redis.Options{
			Addr:     addr,
			PoolSize: 1,
		})
		subWg.Add(1)
		recvWg.Add(1)
		go func(c *redis.Client) {
			defer recvWg.Done()

			sub := c.Subscribe(ctx, channel)
			defer sub.Close()

			ch := sub.Channel()

			subReadyCh <- struct{}{}
			subWg.Done()

			<-ready

			var got int64
			for range ch {
				got++
				atomic.AddInt64(&received, 1)
				if got >= expectedPerSub {
					break
				}
			}
		}(subClients[i])
	}

	subWg.Wait()
	for i := 0; i < PUB_SUBSCRIBERS; i++ {
		<-subReadyCh
	}
	close(ready)

	time.Sleep(50 * time.Millisecond)

	pubClient := redis.NewClient(&redis.Options{
		Addr:     addr,
		PoolSize: PUB_PUBLISHERS + 2,
	})
	defer pubClient.Close()

	var published int64
	var pubWg sync.WaitGroup
	pubStart := time.Now()

	for i := 0; i < PUB_PUBLISHERS; i++ {
		pubWg.Add(1)
		go func(id int) {
			defer pubWg.Done()
			for j := 0; j < PUB_MSGS_EACH; j++ {
				msg := fmt.Sprintf("msg:%d:%d", id, j)
				if err := pubClient.Publish(ctx, channel, msg).Err(); err != nil {
					fmt.Printf("publish error: %v\n", err)
					return
				}
				atomic.AddInt64(&published, 1)
			}
		}(i)
	}
	pubWg.Wait()
	pubElapsed := time.Since(pubStart)

	recvWg.Wait()
	recvElapsed := time.Since(pubStart)

	for _, c := range subClients {
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
