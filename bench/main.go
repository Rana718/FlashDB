package main

import (
	"flag"
	"fmt"
	"net"
	"os"
	"runtime"
	"strconv"
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
	cluster := flag.String("cluster", "", "comma-separated list of cluster master addrs")
	pid := flag.Int("pid", 0, "server PID for resource monitoring (auto-detect if 0)")
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

	serverPID := *pid
	if serverPID == 0 {
		serverPID = findServerPID(*port)
	}

	var clusterPIDs []int
	var dockerContainers []string
	useDocker := false

	if len(addrs) > 1 {

		dockerContainers = findRedisContainers()
		if len(dockerContainers) > 0 {
			useDocker = true
		} else {
			for _, addr := range addrs {
				parts := strings.Split(addr, ":")
				if len(parts) == 2 {
					p, _ := strconv.Atoi(parts[1])
					if rpid := findServerPID(p); rpid > 0 {
						clusterPIDs = append(clusterPIDs, rpid)
					}
				}
			}
		}
	}

	if serverPID > 0 {
		idle := sampleProc(serverPID)
		fmt.Printf("── Server Resource (idle, PID %d) ──────────────\n", serverPID)
		fmt.Printf("   RSS: %s\n\n", fmtBytes(idle.rssBytes))
	} else if useDocker {
		rss, cpu := sampleDocker(dockerContainers)
		fmt.Printf("── Cluster Resource (idle, %d containers) ──────\n", len(dockerContainers))
		fmt.Printf("   total RSS: %s  CPU: %.1f%%\n\n", fmtBytes(rss), cpu)
	} else if len(clusterPIDs) > 0 {
		var totalRSS int64
		for _, p := range clusterPIDs {
			s := sampleProc(p)
			totalRSS += s.rssBytes
		}
		fmt.Printf("── Cluster Resource (idle, %d nodes) ───────────\n", len(clusterPIDs))
		fmt.Printf("   total RSS: %s\n\n", fmtBytes(totalRSS))
	}

	var mon *monitor
	if useDocker {
		mon = startDockerMonitor(dockerContainers)
	} else if len(clusterPIDs) > 0 {
		mon = startMultiMonitor(clusterPIDs)
	} else {
		mon = startMonitor(serverPID)
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

	mon.stop()
	if serverPID > 0 || len(clusterPIDs) > 0 || useDocker {
		fmt.Println()
		fmt.Println("── Server Resource Usage ────────────────────────")
		if serverPID > 0 {
			fmt.Printf("   PID:         %d\n", serverPID)
		} else if useDocker {
			fmt.Printf("   containers:  %d\n", len(dockerContainers))
		} else {
			fmt.Printf("   nodes:       %d\n", len(clusterPIDs))
		}
		fmt.Printf("   peak RSS:    %s\n", fmtBytes(mon.peakRSS))
		fmt.Printf("   avg RSS:     %s\n", fmtBytes(mon.avgRSS))
		fmt.Printf("   peak CPU:    %.1f%%\n", mon.peakCPU)
		fmt.Printf("   avg CPU:     %.1f%%\n", mon.avgCPU)
	}
}
