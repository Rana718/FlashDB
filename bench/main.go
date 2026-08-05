package main

import (
	"flag"
	"fmt"
	"net"
	"os"
	"runtime"
	"strings"
)

const (
	HOST    = "127.0.0.1"
	CLIENTS = 100

	OPS_CLIENT = 10000
	PIPE_SIZE  = 100

	PUB_SUBSCRIBERS = 50
	PUB_PUBLISHERS  = 10
	PUB_MSGS_EACH   = 20000
)

var addrs []string

func pickAddr(i int) string {
	return addrs[i%len(addrs)]
}

func main() {
	port := flag.Int("p", 8000, "server port (single-node mode)")
	mode := flag.String("m", "all", "mode: all | key | pub")
	cluster := flag.String("cluster", "", "comma-separated list of cluster master addrs, e.g. 127.0.0.1:7001,127.0.0.1:7002,127.0.0.1:7003")
	flag.Parse()

	runtime.GOMAXPROCS(runtime.NumCPU())

	if *cluster != "" {
		for _, a := range strings.Split(*cluster, ",") {
			a = strings.TrimSpace(a)
			if a != "" {
				addrs = append(addrs, a)
			}
		}
	} else {
		addrs = []string{fmt.Sprintf("%s:%d", HOST, *port)}
	}

	for _, addr := range addrs {
		conn, err := net.Dial("tcp", addr)
		if err != nil {
			fmt.Fprintf(os.Stderr, "could not connect to %s: %v\n", addr, err)
			os.Exit(1)
		}
		conn.Close()
	}

	if len(addrs) == 1 {
		fmt.Printf("connected to %s\n\n", addrs[0])
	} else {
		fmt.Printf("connected to Redis Cluster (%d masters): %s\n\n",
			len(addrs), strings.Join(addrs, ", "))
	}

	switch *mode {
	case "key":
		runKV()
	case "pub":
		runPubSub(addrs[0])
	case "all":
		runKV()
		fmt.Println()
		runPubSub(addrs[0])
	default:
		fmt.Fprintf(os.Stderr, "unknown -m value %q (want: all | key | pub)\n", *mode)
		os.Exit(1)
	}
}
