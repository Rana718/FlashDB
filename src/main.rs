use flash_db::{
    handler::Conn,
    storage::{rdb, store::Store},
};
use mimalloc::MiMalloc;
use mio::net::TcpListener;
use mio::{Events, Interest, Poll, Token};
use socket2::{Domain, Protocol, Socket, Type};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

const LISTENER: Token = Token(0);
const RDB_PATH: &str = "flashdb.rdb";
const RDB_SAVE_INTERVAL: Duration = Duration::from_secs(300);

fn main() {
    unsafe {
        let mut mask: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut mask);
        libc::sigaddset(&mut mask, libc::SIGTERM);
        libc::sigaddset(&mut mask, libc::SIGINT);
        libc::pthread_sigmask(libc::SIG_BLOCK, &mask, std::ptr::null_mut());
    }

    let workers = num_cpus::get();
    let store = Arc::new(Store::new());

    match rdb::load(&store, RDB_PATH) {
        Ok(0) => println!("flashdb: no snapshot found, starting empty"),
        Ok(n) => println!("flashdb: loaded {n} keys from {RDB_PATH}"),
        Err(e) => eprintln!("flashdb: failed to load snapshot: {e}"),
    }

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

    rdb::start_background_save(Arc::clone(&store), RDB_PATH.to_string(), RDB_SAVE_INTERVAL);

    {
        let store = Arc::clone(&store);
        std::thread::spawn(move || {
            let mut sig = 0i32;
            unsafe {
                let mut mask: libc::sigset_t = std::mem::zeroed();
                libc::sigemptyset(&mut mask);
                libc::sigaddset(&mut mask, libc::SIGTERM);
                libc::sigaddset(&mut mask, libc::SIGINT);
                libc::sigwait(&mask, &mut sig);
            }
            println!("\nflashdb: shutting down, saving...");
            match rdb::save(&store, RDB_PATH) {
                Ok(()) => println!("flashdb: saved to {RDB_PATH}. Bye!"),
                Err(e) => eprintln!("flashdb: save failed: {e}"),
            }
            std::process::exit(0);
        });
    }

    let mut handles = Vec::with_capacity(workers);
    for _ in 0..workers {
        let store = Arc::clone(&store);
        handles.push(std::thread::spawn(move || run_worker(store)));
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
                LISTENER => loop {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            let id = if let Some(id) = free.pop() {
                                id
                            } else {
                                let id = next_token;
                                next_token += 1;
                                if id >= conns.len() {
                                    conns.resize_with(id + 1, || None);
                                }
                                id
                            };
                            poll.registry()
                                .register(&mut stream, Token(id), Interest::READABLE)
                                .unwrap();
                            conns[id] = Some(Conn::new(stream, Arc::clone(&store)));
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(_) => break,
                    }
                },

                token => {
                    let id = token.0;
                    let close = if let Some(Some(conn)) = conns.get_mut(id) {
                        if !conn.do_read() {
                            true
                        } else {
                            !conn.do_write()
                        }
                    } else {
                        false
                    };

                    if close {
                        if let Some(slot) = conns.get_mut(id) {
                            if let Some(mut conn) = slot.take() {
                                let _ = poll.registry().deregister(&mut conn.stream);
                                free.push(id);
                            }
                        }
                    }
                }
            }
        }
    }
}

mod libc {
    pub use ::libc::*;
}
