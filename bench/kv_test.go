package main

import (
	"bufio"
	"strings"
	"testing"
)

func TestHashTag(t *testing.T) {
	tests := []struct {
		key  string
		want string
	}{
		{"plain", ""},
		{"foo{bar}zap", "bar"},
		{"foo{}bar", ""},
		{"foo{bar", ""},
	}
	for _, tt := range tests {
		got := hashTag([]byte(tt.key))
		if string(got) != tt.want {
			t.Fatalf("hashTag(%q) = %q, want %q", tt.key, got, tt.want)
		}
	}
}

func TestKeySlotUsesHashTag(t *testing.T) {
	if got, want := keySlot([]byte("a{shared}1")), keySlot([]byte("b{shared}2")); got != want {
		t.Fatalf("tagged keys mapped to different slots: %d != %d", got, want)
	}
}

func TestClientKeyRoutesToClientNode(t *testing.T) {
	oldAddrs := addrs
	addrs = []string{"node0", "node1", "node2", "node3", "node4", "node5"}
	defer func() { addrs = oldAddrs }()

	var scratch [64]byte
	for id := 0; id < len(addrs); id++ {
		key := clientKey("test:", id, scratch[:])
		if got := nodeForKey(key); got != id {
			t.Fatalf("client %d key routed to node %d", id, got)
		}
	}
}

func TestSkipGetReplies(t *testing.T) {
	r := bufio.NewReader(strings.NewReader("$5\r\nvalue\r\n$-1\r\n:1\r\n"))
	skipGetReplies(r, 3)
	if r.Buffered() != 0 {
		t.Fatalf("reader has %d bytes left", r.Buffered())
	}
}

func TestSkipLinesRejectsRedisError(t *testing.T) {
	defer func() {
		if recover() == nil {
			t.Fatal("expected Redis error reply to panic")
		}
	}()
	skipLines(bufio.NewReader(strings.NewReader("-ERR unsupported\r\n")), 1)
}
