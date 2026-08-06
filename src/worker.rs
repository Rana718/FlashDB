// run_worker — the per-thread epoll event loop.
// Handles connections, pub/sub notification, max clients, and slow subscriber eviction.

use mio::net::TcpListener;
use mio::{Events, Interest, Poll, Token, Waker};
use socket2::{Domain, Protocol, Socket, Type};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crate::handler::Conn;
use crate::pubsub::{PubSub, WorkerNotifier};
use crate::storage::store::Store;

const LISTENER_TOKEN: Token = Token(0);
const WAKER_TOKEN: Token = Token(usize::MAX);

/// Global max clients limit (shared across all workers).
/// Default 10_000, configurable via FLASHDB_MAX_CLIENTS env.
static MAX_CLIENTS: AtomicUsize = AtomicUsize::new(10_000);

/// Slow subscriber message cap — disconnect subscribers with more than this many
/// pending messages to prevent unbounded memory growth.
const SLOW_SUB_MSG_CAP: usize = 65_536;

/// Maximum write buffer size before we consider a connection slow and close it.
const MAX_WRITE_BUF: usize = 64 * 1024 * 1024; // 64 MB

/// Buffer shrink threshold: if wbuf capacity > 4x its length after a flush, shrink it.
const SHRINK_THRESHOLD: usize = 256 * 1024;

pub fn set_max_clients(n: usize) {
    MAX_CLIENTS.store(n, Ordering::Relaxed);
}

pub fn run_worker(store: Arc<Store>, pubsub: Arc<PubSub>, port: u16) {
    let addr: SocketAddr = format!("0.0.0.0:{port}").parse().unwrap();
    let mut listener = make_listener(addr);

    let mut poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(4096);

    poll.registry()
        .register(&mut listener, LISTENER_TOKEN, Interest::READABLE)
        .unwrap();

    let waker = Arc::new(Waker::new(poll.registry(), WAKER_TOKEN).unwrap());
    let notifier = WorkerNotifier::new(waker);

    let mut conns: Vec<Option<Conn>> = Vec::with_capacity(4096);
    let mut next_token: usize = 1;
    let mut free: Vec<usize> = Vec::new();
    let mut dirty: Vec<usize> = Vec::with_capacity(256);

    loop {
        match poll.poll(&mut events, None) {
            Ok(_) => {}
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                eprintln!("flashdb: poll error: {e}");
                continue;
            }
        }

        for event in events.iter() {
            match event.token() {
                LISTENER_TOKEN => loop {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            // Phase D: max clients check
                            let current = store.connected_clients();
                            let max = MAX_CLIENTS.load(Ordering::Relaxed);
                            if current >= max {
                                // Reject: send error and close
                                let _ = std::io::Write::write_all(
                                    &mut stream,
                                    b"-ERR max number of clients reached\r\n",
                                );
                                drop(stream);
                                continue;
                            }

                            let _ = stream.set_nodelay(true);
                            let id = alloc_slot(&mut free, &mut next_token, &mut conns);
                            poll.registry()
                                .register(&mut stream, Token(id), Interest::READABLE)
                                .unwrap();
                            conns[id] = Some(Conn::new(
                                stream,
                                Arc::clone(&store),
                                Arc::clone(&pubsub),
                                id,
                                Arc::clone(&notifier),
                            ));
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(ref e) if is_accept_transient(e) => continue,
                        Err(_) => break,
                    }
                },

                WAKER_TOKEN => {
                    while let Some(id) = notifier.pending.pop() {
                        dirty.push(id);
                    }
                }

                token => {
                    let id = token.0;
                    let close = match conns.get_mut(id).and_then(|s| s.as_mut()) {
                        Some(conn) => !conn.do_read(),
                        None => false,
                    };
                    if close {
                        close_conn(&mut conns, &mut poll, &mut free, id);
                    } else {
                        dirty.push(id);
                    }
                }
            }
        }

        // Phase D: flush writes + slow subscriber eviction + buffer management
        for id in dirty.drain(..) {
            if let Some(Some(conn)) = conns.get_mut(id) {
                // Check slow subscriber cap
                if is_slow_subscriber(conn) {
                    close_conn(&mut conns, &mut poll, &mut free, id);
                    continue;
                }

                if !conn.do_write() {
                    close_conn(&mut conns, &mut poll, &mut free, id);
                    continue;
                }

                // Check write buffer overflow (connection writing too slow)
                if conn.parser.wbuf.len() > MAX_WRITE_BUF {
                    close_conn(&mut conns, &mut poll, &mut free, id);
                    continue;
                }

                // Phase D: shrink oversized write buffers after flush
                if conn.parser.wbuf.is_empty()
                    && conn.parser.wbuf.capacity() > SHRINK_THRESHOLD
                {
                    conn.parser.wbuf = Vec::with_capacity(256 * 1024);
                }

                // Toggle WRITABLE interest based on pending data
                if conn.has_pending_write() {
                    let _ = poll.registry().reregister(
                        &mut conn.stream,
                        Token(id),
                        Interest::READABLE | Interest::WRITABLE,
                    );
                } else {
                    let _ = poll.registry().reregister(
                        &mut conn.stream,
                        Token(id),
                        Interest::READABLE,
                    );
                }
            }
        }
    }
}

/// Check if a subscriber connection has too many pending messages.
#[inline]
fn is_slow_subscriber(conn: &Conn) -> bool {
    use crate::handler::conn::ConnMode;
    if let ConnMode::Subscribed { ref slot, .. } = conn.mode {
        slot.queue_len() > SLOW_SUB_MSG_CAP
    } else {
        false
    }
}

/// Transient accept errors that should be retried.
#[inline]
fn is_accept_transient(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::Interrupted
    )
}

fn make_listener(addr: SocketAddr) -> TcpListener {
    let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP)).unwrap();
    socket.set_reuse_address(true).unwrap();
    socket.set_reuse_port(true).unwrap();
    socket.set_nonblocking(true).unwrap();
    socket.bind(&addr.into()).unwrap();
    socket.listen(8192).unwrap();
    TcpListener::from_std(std::net::TcpListener::from(socket))
}

fn alloc_slot(
    free: &mut Vec<usize>,
    next_token: &mut usize,
    conns: &mut Vec<Option<Conn>>,
) -> usize {
    if let Some(id) = free.pop() {
        return id;
    }
    let id = *next_token;
    *next_token += 1;
    if id >= conns.len() {
        conns.resize_with(id + 1, || None);
    }
    id
}

fn close_conn(conns: &mut [Option<Conn>], poll: &mut Poll, free: &mut Vec<usize>, id: usize) {
    if let Some(slot) = conns.get_mut(id)
        && let Some(mut conn) = slot.take()
    {
        let _ = poll.registry().deregister(&mut conn.stream);
        free.push(id);
    }
}
