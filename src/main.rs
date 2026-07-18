use flash_db::{handler::Conn, storage::store::Store};
use mio::net::TcpListener;
use mio::{Events, Interest, Poll, Token};
use socket2::{Domain, Protocol, Socket, Type};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

const LISTENER: Token = Token(0);

fn main() {
    let workers = num_cpus::get();
    let store = Arc::new(Store::new());

    println!("flashdb running on port 8000 ({workers} threads, mio/epoll)");

    {
        let store = Arc::clone(&store);
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(1));
                store.cleanup_expired();
            }
        });
    }

    let mut handles = Vec::with_capacity(workers);

    for _ in 0..workers {
        let store = Arc::clone(&store);
        let handle = std::thread::spawn(move || {
            run_worker(store);
        });
        handles.push(handle);
    }

    for h in handles {
        let _ = h.join();
    }
}

fn make_listener(addr: SocketAddr) -> TcpListener {
    let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP)).unwrap();
    socket.set_reuse_address(true).unwrap();
    socket.set_reuse_port(true).unwrap();
    socket.set_nonblocking(true).unwrap();
    socket.bind(&addr.into()).unwrap();
    socket.listen(4096).unwrap();
    TcpListener::from_std(std::net::TcpListener::from(socket))
}

fn run_worker(store: Arc<Store>) {
    let addr: SocketAddr = "0.0.0.0:8000".parse().unwrap();
    let mut listener = make_listener(addr);

    let mut poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);

    poll.registry()
        .register(&mut listener, LISTENER, Interest::READABLE)
        .unwrap();

    let mut conns: HashMap<usize, Conn> = HashMap::new();
    let mut next_token: usize = 1;

    loop {
        poll.poll(&mut events, None).unwrap();

        for event in events.iter() {
            match event.token() {
                LISTENER => loop {
                    match listener.accept() {
                        Ok((mut stream, _addr)) => {
                            let token = Token(next_token);
                            next_token = next_token.wrapping_add(1);
                            if next_token == 0 {
                                next_token = 1;
                            }

                            poll.registry()
                                .register(
                                    &mut stream,
                                    token,
                                    Interest::READABLE | Interest::WRITABLE,
                                )
                                .unwrap();

                            let conn = Conn::new(stream, Arc::clone(&store));
                            conns.insert(token.0, conn);
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(_) => break,
                    }
                },

                token => {
                    let id = token.0;
                    let mut close = false;

                    if let Some(conn) = conns.get_mut(&id) {
                        if event.is_readable() {
                            if !conn.do_read() {
                                close = true;
                            }
                        }
                        if !close && (event.is_writable() || conn.parser.has_buffered_input()) {
                            if !conn.do_write() {
                                close = true;
                            }
                        }
                    }

                    if close {
                        if let Some(mut conn) = conns.remove(&id) {
                            let _ = poll.registry().deregister(&mut conn.stream);
                        }
                    }
                }
            }
        }
    }
}
