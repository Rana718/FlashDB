use crossbeam_queue::SegQueue;
use mio::Waker;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

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

    #[inline]
    pub fn push(&self, msg: Arc<[u8]>) {
        self.queue.push(msg);
        if !self.notify_pending.swap(true, Ordering::AcqRel) {
            self.notifier.pending.push(self.token);
            let _ = self.notifier.waker.wake();
        }
    }

    #[inline]
    pub fn drain_into(&self, out: &mut Vec<u8>) {
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
