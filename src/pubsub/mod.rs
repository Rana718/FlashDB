use crossbeam_queue::SegQueue;
use dashmap::DashMap;
use mio::Waker;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use crate::utils::util::glob_match_bytes;

// ---------------------------------------------------------------------------
// WorkerNotifier — one per worker thread
// ---------------------------------------------------------------------------

/// Shared between every SubSlot owned by the worker and the worker's event
/// loop.  When a message is pushed to a subscriber, the token is enqueued
/// here so the worker knows exactly which connections need flushing — no
/// full-arena scan required.
pub struct WorkerNotifier {
    /// Tokens of connections that have at least one pending message.
    pub pending: SegQueue<usize>,
    /// Wakes the worker's epoll loop.
    pub waker: Arc<Waker>,
}

impl WorkerNotifier {
    pub fn new(waker: Arc<Waker>) -> Arc<Self> {
        Arc::new(Self {
            pending: SegQueue::new(),
            waker,
        })
    }
}

// ---------------------------------------------------------------------------
// SubSlot — one per subscribed connection
// ---------------------------------------------------------------------------

pub struct SubSlot {
    /// Connection token in the worker's arena.
    pub token: usize,
    /// Lock-free message queue (MPMC, wait-free push).
    pub queue: SegQueue<Arc<Vec<u8>>>,
    /// True once the token has been enqueued into the notifier's pending
    /// queue and not yet drained.  Prevents duplicate token enqueues.
    notify_pending: AtomicBool,
    /// Worker notifier — shared with all other slots on the same worker.
    notifier: Arc<WorkerNotifier>,
}

impl SubSlot {
    pub fn new(token: usize, notifier: Arc<WorkerNotifier>) -> Self {
        Self {
            token,
            queue: SegQueue::new(),
            notify_pending: AtomicBool::new(false),
            notifier,
        }
    }

    /// Push a message.  Enqueues the token into the worker notifier once
    /// per burst (not once per message) and wakes epoll once per burst.
    #[inline]
    pub fn push(&self, msg: Arc<Vec<u8>>) {
        self.queue.push(msg);
        // CAS false→true: only the first pusher of a burst does the work.
        if !self.notify_pending.swap(true, Ordering::AcqRel) {
            self.notifier.pending.push(self.token);
            let _ = self.notifier.waker.wake();
        }
    }

    /// Drain all pending messages into `out`.
    /// Resets notify_pending so the next push will re-enqueue the token.
    #[inline]
    pub fn drain_into(&self, out: &mut Vec<u8>) {
        // Reset before draining so a concurrent push after this point
        // correctly re-enqueues rather than getting lost.
        self.notify_pending.store(false, Ordering::Release);
        while let Some(msg) = self.queue.pop() {
            out.extend_from_slice(&msg);
        }
    }

    #[inline]
    pub fn has_pending(&self) -> bool {
        !self.queue.is_empty()
    }
}

// ---------------------------------------------------------------------------
// PatternEntry
// ---------------------------------------------------------------------------

struct PatternEntry {
    pattern: String,
    slot: Arc<SubSlot>,
}

// ---------------------------------------------------------------------------
// PubSub
// ---------------------------------------------------------------------------

pub struct PubSub {
    channels: DashMap<String, Vec<Arc<SubSlot>>>,
    patterns: RwLock<Vec<PatternEntry>>,
}

impl PubSub {
    pub fn new() -> Self {
        Self {
            channels: DashMap::new(),
            patterns: RwLock::new(Vec::new()),
        }
    }

    pub fn subscribe(&self, channel: &str, slot: Arc<SubSlot>) {
        self.channels
            .entry(channel.to_string())
            .or_default()
            .push(slot);
    }

    pub fn unsubscribe(&self, channel: &str, slot: &Arc<SubSlot>) {
        let mut entry = match self.channels.get_mut(channel) {
            Some(e) => e,
            None => return,
        };
        entry.retain(|s| !Arc::ptr_eq(s, slot));
        let empty = entry.is_empty();
        drop(entry);
        if empty {
            self.channels.remove_if(channel, |_, v| v.is_empty());
        }
    }

    pub fn psubscribe(&self, pattern: &str, slot: Arc<SubSlot>) {
        self.patterns.write().unwrap().push(PatternEntry {
            pattern: pattern.to_string(),
            slot,
        });
    }

    pub fn punsubscribe(&self, pattern: &str, slot: &Arc<SubSlot>) {
        self.patterns
            .write()
            .unwrap()
            .retain(|e| !(e.pattern == pattern && Arc::ptr_eq(&e.slot, slot)));
    }

    /// Encode once, fan out via Arc.  Returns delivery count.
    pub fn publish(&self, channel: &str, message: &str) -> usize {
        let mut count = 0usize;

        if let Some(slots) = self.channels.get(channel) {
            let slen = slots.len();
            if slen > 0 {
                let frame = Arc::new(encode_message(channel, message));
                for slot in slots.iter() {
                    slot.push(Arc::clone(&frame));
                }
                count += slen;
            }
        }

        {
            let guard = self.patterns.read().unwrap();
            if !guard.is_empty() {
                let chan_b = channel.as_bytes();
                for entry in guard.iter() {
                    if glob_match_bytes(entry.pattern.as_bytes(), chan_b) {
                        let frame = Arc::new(encode_pmessage(&entry.pattern, channel, message));
                        entry.slot.push(frame);
                        count += 1;
                    }
                }
            }
        }

        count
    }

    pub fn active_channels(&self, pattern: Option<&str>) -> Vec<String> {
        self.channels
            .iter()
            .filter(|e| {
                !e.value().is_empty()
                    && pattern.map_or(true, |p| {
                        glob_match_bytes(p.as_bytes(), e.key().as_bytes())
                    })
            })
            .map(|e| e.key().clone())
            .collect()
    }

    pub fn numsub(&self, channels: &[&str]) -> Vec<(String, usize)> {
        channels
            .iter()
            .map(|&ch| {
                let n = self.channels.get(ch).map(|e| e.len()).unwrap_or(0);
                (ch.to_string(), n)
            })
            .collect()
    }

    pub fn numpat(&self) -> usize {
        self.patterns.read().unwrap().len()
    }
}

// ---------------------------------------------------------------------------
// RESP frame encoders
// ---------------------------------------------------------------------------

fn encode_message(channel: &str, message: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(32 + channel.len() + message.len());
    out.extend_from_slice(b"*3\r\n$7\r\nmessage\r\n");
    bulk_into(&mut out, channel.as_bytes());
    bulk_into(&mut out, message.as_bytes());
    out
}

fn encode_pmessage(pattern: &str, channel: &str, message: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(48 + pattern.len() + channel.len() + message.len());
    out.extend_from_slice(b"*4\r\n$8\r\npmessage\r\n");
    bulk_into(&mut out, pattern.as_bytes());
    bulk_into(&mut out, channel.as_bytes());
    bulk_into(&mut out, message.as_bytes());
    out
}

pub fn encode_sub_reply(kind: &str, channel: &str, count: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(32 + kind.len() + channel.len());
    out.extend_from_slice(b"*3\r\n");
    bulk_into(&mut out, kind.as_bytes());
    bulk_into(&mut out, channel.as_bytes());
    out.push(b':');
    push_usize(&mut out, count);
    out.extend_from_slice(b"\r\n");
    out
}

#[inline]
fn bulk_into(out: &mut Vec<u8>, b: &[u8]) {
    out.push(b'$');
    push_usize(out, b.len());
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(b);
    out.extend_from_slice(b"\r\n");
}

#[inline]
fn push_usize(out: &mut Vec<u8>, mut n: usize) {
    if n == 0 {
        out.push(b'0');
        return;
    }
    let start = out.len();
    while n > 0 {
        out.push(b'0' + (n % 10) as u8);
        n /= 10;
    }
    out[start..].reverse();
}
