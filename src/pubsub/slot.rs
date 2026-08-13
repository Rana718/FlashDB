use crossbeam_queue::SegQueue;
use mio::Waker;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub struct WorkerNotifier {
    pub pending: SegQueue<usize>,
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

pub struct SubSlot {
    pub token: usize,
    pub queue: SegQueue<Arc<[u8]>>,
    notify_pending: AtomicBool,
    notifier: Arc<WorkerNotifier>,
    len: AtomicUsize,
}

impl SubSlot {
    pub fn new(token: usize, notifier: Arc<WorkerNotifier>) -> Self {
        Self {
            token,
            queue: SegQueue::new(),
            notify_pending: AtomicBool::new(false),
            notifier,
            len: AtomicUsize::new(0),
        }
    }

    #[inline]
    pub fn push(&self, msg: Arc<[u8]>) {
        // Publish the accounting first. If the consumer could pop before this
        // increment, its fetch_sub would wrap 0 to usize::MAX and the worker
        // would falsely classify this connection as a slow subscriber.
        self.len.fetch_add(1, Ordering::Relaxed);
        self.queue.push(msg);
        if !self.notify_pending.swap(true, Ordering::AcqRel) {
            self.notifier.pending.push(self.token);
            let _ = self.notifier.waker.wake();
        }
    }

    #[inline]
    pub fn drain_into(&self, out: &mut Vec<u8>) {
        self.drain_into_limit(out, usize::MAX);
    }

    pub fn drain_into_limit(&self, out: &mut Vec<u8>, max_bytes: usize) {
        self.notify_pending.store(false, Ordering::Release);
        while let Some(msg) = self.queue.pop() {
            out.extend_from_slice(&msg);
            self.len.fetch_sub(1, Ordering::Relaxed);
            if out.len() >= max_bytes {
                break;
            }
        }
        // A publisher may have raced with notify_pending=false, or bounded
        // draining may have left messages queued. Re-arm this slot exactly
        // once so the worker continues flushing without polling every slot.
        if !self.queue.is_empty() && !self.notify_pending.swap(true, Ordering::AcqRel) {
            self.notifier.pending.push(self.token);
            let _ = self.notifier.waker.wake();
        }
    }

    #[inline]
    pub fn has_pending(&self) -> bool {
        !self.queue.is_empty()
    }

    #[inline]
    pub fn queue_len(&self) -> usize {
        self.len.load(Ordering::Relaxed)
    }
}
