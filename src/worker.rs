// run_worker — the per-thread epoll event loop.
// its WorkerNotifier, and the arena of Conn slots.

use mio::net::TcpListener;
use mio::{Events, Interest, Poll, Token, Waker};
use socket2::{Domain, Protocol, Socket, Type};
use std::net::SocketAddr;
use std::sync::Arc;

use crate::handler::Conn;
use crate::pubsub::{PubSub, WorkerNotifier};
use crate::storage::store::Store;

const LISTENER_TOKEN: Token = Token(0);
const WAKER_TOKEN: Token = Token(usize::MAX);

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

    loop {
        match poll.poll(&mut events, None) {
            Ok(_) => {}
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => panic!("poll error: {e}"),
        }

        for event in events.iter() {
            match event.token() {
                LISTENER_TOKEN => loop {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
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
                        if let Some(Some(conn)) = conns.get_mut(id) {
                            if !conn.do_write() {
                                close_conn(&mut conns, &mut poll, &mut free, id);
                            }
                        }
                    }
                }

                token => {
                    let id = token.0;
                    let close = match conns.get_mut(id).and_then(|s| s.as_mut()) {
                        Some(conn) => !conn.do_read() || !conn.do_write(),
                        None => false,
                    };
                    if close {
                        close_conn(&mut conns, &mut poll, &mut free, id);
                    }
                }
            }
        }
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

fn close_conn(conns: &mut Vec<Option<Conn>>, poll: &mut Poll, free: &mut Vec<usize>, id: usize) {
    if let Some(slot) = conns.get_mut(id) {
        if let Some(mut conn) = slot.take() {
            let _ = poll.registry().deregister(&mut conn.stream);
            free.push(id);
        }
    }
}
