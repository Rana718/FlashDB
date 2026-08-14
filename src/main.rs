use flash_db::{
    pubsub::PubSub,
    storage::{rdb, store::Store},
    worker::{run_worker, set_max_clients},
};
use mimalloc::MiMalloc;
use std::env;
use std::sync::Arc;
use std::time::Duration;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

fn main() {
    let config = Config::from_env();

    unsafe {
        let mut mask: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut mask);
        libc::sigaddset(&mut mask, libc::SIGTERM);
        libc::sigaddset(&mut mask, libc::SIGINT);
        libc::pthread_sigmask(libc::SIG_BLOCK, &mask, std::ptr::null_mut());
    }

    let workers = config.workers;
    let store = Arc::new(Store::with_config(config.shards, config.max_keys));
    let pubsub = Arc::new(PubSub::new());

    set_max_clients(config.max_clients);

    if let Err(e) = rdb::load(&store, &config.rdb_path) {
        eprintln!("flashdb: failed to load snapshot: {e}");
    }

    println!(
        "flashdb running on 0.0.0.0:{} ({workers} workers)",
        config.port
    );
    println!(
        "  max_keys={} shards={} max_clients={} rdb_path={} rdb_interval={}s",
        config.max_keys,
        config.shards,
        config.max_clients,
        config.rdb_path,
        config.rdb_interval.as_secs()
    );

    spawn_expiry_thread(Arc::clone(&store));
    rdb::start_background_save(
        Arc::clone(&store),
        config.rdb_path.clone(),
        config.rdb_interval,
    );
    spawn_signal_thread(Arc::clone(&store), config.rdb_path.clone());

    let mut handles = Vec::with_capacity(workers);
    for _ in 0..workers {
        let store = Arc::clone(&store);
        let pubsub = Arc::clone(&pubsub);
        let port = config.port;
        handles.push(std::thread::spawn(move || run_worker(store, pubsub, port)));
    }
    for h in handles {
        let _ = h.join();
    }
}

struct Config {
    port: u16,
    workers: usize,
    shards: usize,
    max_keys: usize,
    max_clients: usize,
    rdb_path: String,
    rdb_interval: Duration,
}

impl Config {
    fn from_env() -> Self {
        let workers = env_usize("FLASHDB_WORKERS", 0);
        let workers = if workers == 0 {
            num_cpus::get()
        } else {
            workers
        };
        let shards = env_usize("FLASHDB_SHARDS", 0);
        let shards = if shards == 0 {
            (workers * 4).next_power_of_two()
        } else {
            shards.next_power_of_two()
        };
        Config {
            port: env_u16("FLASHDB_PORT", 8000),
            workers,
            shards,
            max_keys: env_usize("FLASHDB_MAX_KEYS", 1_000_000),
            max_clients: env_usize("FLASHDB_MAX_CLIENTS", 10_000),
            rdb_path: env::var("FLASHDB_RDB_PATH").unwrap_or_else(|_| "flashdb.rdb".to_string()),
            rdb_interval: Duration::from_secs(env_u64("FLASHDB_RDB_INTERVAL", 300)),
        }
    }
}

fn env_u16(key: &str, default: u16) -> u16 {
    env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_usize(key: &str, default: usize) -> usize {
    env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_u64(key: &str, default: u64) -> u64 {
    env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn spawn_expiry_thread(store: Arc<Store>) {
    std::thread::Builder::new()
        .name("flashdb-expiry".into())
        .spawn(move || {
            let mut shard = 0usize;
            loop {
                std::thread::sleep(Duration::from_millis(100));
                store.cleanup_expired_shard(shard);
                shard += 1;
                if shard == store.map_shard_count() {
                    shard = 0;
                }
            }
        })
        .expect("failed to spawn expiry thread");
}

fn spawn_signal_thread(store: Arc<Store>, rdb_path: String) {
    std::thread::Builder::new()
        .name("flashdb-signal".into())
        .spawn(move || {
            let mut sig = 0i32;
            unsafe {
                let mut mask: libc::sigset_t = std::mem::zeroed();
                libc::sigemptyset(&mut mask);
                libc::sigaddset(&mut mask, libc::SIGTERM);
                libc::sigaddset(&mut mask, libc::SIGINT);
                libc::sigwait(&mask, &mut sig);
            }
            eprintln!("flashdb: received signal {sig}, saving...");
            if let Err(e) = flash_db::storage::rdb::save(&store, &rdb_path) {
                eprintln!("flashdb: save failed: {e}");
            }
            std::process::exit(0);
        })
        .expect("failed to spawn signal thread");
}

mod libc {
    pub use ::libc::*;
}
