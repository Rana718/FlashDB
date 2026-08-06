use mio::net::TcpListener;
use mio::{Events, Interest, Poll, Token, Waker};
use socket2::{Domain, Protocol, Socket, Type};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::handler::Conn;
use crate::handler::conn::ConnMode;
use crate::pubsub::{PubSub, WorkerNotifier};
use crate::storage::store::Store;

const LISTENER_TOKEN: Token = Token(0);
const WAKER_TOKEN: Token = Token(usize::MAX);

static MAX_CLIENTS: AtomicUsize = AtomicUsize::new(10_000);

const SLOW_SUB_MSG_CAP: usize = 65_536;

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
    let mut has_subscribers = false;

    loop {
        let timeout = if has_subscribers {
            Some(std::time::Duration::from_micros(100))
        } else {
            None
        };

        match poll.poll(&mut events, timeout) {
            Ok(_) => {}
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => continue,
        }

        for event in events.iter() {
            match event.token() {
                LISTENER_TOKEN => loop {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            let current = store.connected_clients();
                            let max = MAX_CLIENTS.load(Ordering::Relaxed);
                            if current >= max {
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

        for id in dirty.drain(..) {
            if let Some(Some(conn)) = conns.get_mut(id) {
                if is_slow_subscriber(conn) {
                    close_conn(&mut conns, &mut poll, &mut free, id);
                    continue;
                }
                if !conn.do_write() {
                    close_conn(&mut conns, &mut poll, &mut free, id);
                }
            }
        }

        has_subscribers = false;
        for slot in conns.iter() {
            if let Some(conn) = slot {
                if matches!(&conn.mode, ConnMode::Subscribed { slot, .. } if slot.has_pending()) {
                    has_subscribers = true;
                    break;
                }
                if !conn.parser.wbuf.is_empty() {
                    has_subscribers = true;
                    break;
                }
            }
        }

        if has_subscribers {
            while let Some(id) = notifier.pending.pop() {
                dirty.push(id);
            }
            for id in dirty.drain(..) {
                if let Some(Some(conn)) = conns.get_mut(id) {
                    if !conn.do_write() {
                        close_conn(&mut conns, &mut poll, &mut free, id);
                    }
                }
            }
            for id in 1..conns.len() {
                if let Some(conn) = conns[id].as_mut() {
                    if conn.has_pending_write() {
                        if !conn.do_write() {
                            close_conn(&mut conns, &mut poll, &mut free, id);
                        }
                    }
                }
            }
        }
    }
}

#[inline]
fn is_slow_subscriber(conn: &Conn) -> bool {
    if let ConnMode::Subscribed { ref slot, .. } = conn.mode {
        slot.queue_len() > SLOW_SUB_MSG_CAP
    } else {
        false
    }
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
