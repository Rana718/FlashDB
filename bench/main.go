package main

import (
	"flag"
	"fmt"
	"net"
	"os"
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

func main() {
	port := flag.Int("p", 8000, "server port")
	mode := flag.String("m", "all", "mode: all | key | pub")
	flag.Parse()

	addr := fmt.Sprintf("%s:%d", HOST, *port)

	conn, err := net.Dial("tcp", addr)
	if err != nil {
		fmt.Fprintf(os.Stderr, "could not connect to %s: %v\n", addr, err)
		os.Exit(1)
	}
	conn.Close()
	fmt.Printf("connected to %s\n\n", addr)

	switch *mode {
	case "key":
		runKV(addr)
	case "pub":
		runPubSub(addr)
	case "all":
		runKV(addr)
		fmt.Println()
		runPubSub(addr)
	default:
		fmt.Fprintf(os.Stderr, "unknown -m value %q (want: all | key | pub)\n", *mode)
		os.Exit(1)
	}
}
