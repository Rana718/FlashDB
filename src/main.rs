use flash_db::{
    pubsub::PubSub,
    storage::{rdb, store::Store},
    worker::run_worker,
};
use mimalloc::MiMalloc;
use std::sync::Arc;
use std::time::Duration;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

const PORT: u16 = 8000;
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
    let pubsub = Arc::new(PubSub::new());

    if let Err(e) = rdb::load(&store, RDB_PATH) {
        eprintln!("flashdb: failed to load snapshot: {e}");
    }

    println!("flashdb running on 0.0.0.0:{PORT} ({workers} threads)");

    spawn_expiry_thread(Arc::clone(&store));
    rdb::start_background_save(Arc::clone(&store), RDB_PATH.to_string(), RDB_SAVE_INTERVAL);
    spawn_signal_thread(Arc::clone(&store));

    let mut handles = Vec::with_capacity(workers);
    for _ in 0..workers {
        let store = Arc::clone(&store);
        let pubsub = Arc::clone(&pubsub);
        handles.push(std::thread::spawn(move || run_worker(store, pubsub, PORT)));
    }
    for h in handles {
        let _ = h.join();
    }
}

fn spawn_expiry_thread(store: Arc<Store>) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(1));
            store.cleanup_expired();
        }
    });
}

fn spawn_signal_thread(store: Arc<Store>) {
    std::thread::spawn(move || {
        let mut sig = 0i32;
        unsafe {
            let mut mask: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut mask);
            libc::sigaddset(&mut mask, libc::SIGTERM);
            libc::sigaddset(&mut mask, libc::SIGINT);
            libc::sigwait(&mask, &mut sig);
        }
        if let Err(e) = flash_db::storage::rdb::save(&store, RDB_PATH) {
            eprintln!("flashdb: save failed: {e}");
        }
        std::process::exit(0);
    });
}

mod libc {
    pub use ::libc::*;
}
