use fyro_db::{
    pubsub::PubSub,
    storage::{rdb, store::Store},
    worker::{initiate_shutdown, run_worker, set_max_clients},
};
use jemallocator::Jemalloc;
use std::env;
use std::sync::Arc;
use std::time::Duration;

#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

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
        eprintln!("fyrodb: failed to load snapshot: {e}");
    }

    println!(
        "fyrodb running on {}:{} ({workers} workers)",
        config.bind, config.port
    );
    println!(
        "  max_keys={} shards={} max_clients={} rdb_path={} rdb_interval={}s",
        config.max_keys,
        config.shards,
        config.max_clients,
        config.rdb_path,
        config.rdb_interval.as_secs()
    );
    if config.auth.is_some() {
        println!("  auth=enabled");
    }

    spawn_expiry_thread(Arc::clone(&store));
    rdb::start_background_save(
        Arc::clone(&store),
        config.rdb_path.clone(),
        config.rdb_interval,
    );
    spawn_signal_thread(Arc::clone(&store), config.rdb_path.clone());

    let mut handles = Vec::with_capacity(workers);
    let auth: Option<Arc<String>> = config.auth.map(Arc::new);
    for _ in 0..workers {
        let store = Arc::clone(&store);
        let pubsub = Arc::clone(&pubsub);
        let port = config.port;
        let bind = config.bind.clone();
        let auth = auth.clone();
        handles.push(std::thread::spawn(move || run_worker(store, pubsub, port, bind, auth)));
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
    auth: Option<String>,
    bind: String,
}

impl Config {
    fn from_env() -> Self {
        let workers = env_usize("FYRODB_WORKERS", 0);
        let workers = if workers == 0 {
            num_cpus::get()
        } else {
            workers
        };
        let shards = env_usize("FYRODB_SHARDS", 0);
        let shards = if shards == 0 {
            (workers * 4).next_power_of_two()
        } else {
            shards.next_power_of_two()
        };
        Config {
            port: env_u16("FYRODB_PORT", 8000),
            workers,
            shards,
            max_keys: env_usize("FYRODB_MAX_KEYS", 1_000_000),
            max_clients: env_usize("FYRODB_MAX_CLIENTS", 10_000),
            rdb_path: env::var("FYRODB_RDB_PATH").unwrap_or_else(|_| "fyrodb.rdb".to_string()),
            rdb_interval: Duration::from_secs(env_u64("FYRODB_RDB_INTERVAL", 300)),
            auth: env::var("FYRODB_AUTH").ok().filter(|s| !s.is_empty()),
            bind: env::var("FYRODB_BIND").unwrap_or_else(|_| "0.0.0.0".to_string()),
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
    const SCAN_SLOTS_PER_TICK: usize = 262_144;

    std::thread::Builder::new()
        .name("fyrodb-expiry".into())
        .spawn(move || {
            let shards = store.map_shard_count();
            let mut shard = 0usize;
            let mut slot = 0usize;
            let mut live_ttls = 0usize;
            let mut generation = store.ttl_generation();
            let mut capacities = vec![0usize; shards];
            let mut collect_tick = 0u8;
            let mut last_key_count = store.dbsize();
            loop {
                std::thread::sleep(Duration::from_secs(1));
                collect_tick += 1;
                if collect_tick >= 10 {
                    collect_tick = 0;
                    customhash::force_collect();
                }
                if store.has_ttl_keys() {
                    let (chunk_live_ttls, next_slot, capacity) =
                        store.cleanup_expired_shard(shard, slot, SCAN_SLOTS_PER_TICK);
                    if slot == 0 {
                        capacities[shard] = capacity;
                    } else if capacities[shard] != capacity {
                        shard = 0;
                        slot = 0;
                        live_ttls = 0;
                        generation = store.ttl_generation();
                        continue;
                    }
                    live_ttls += chunk_live_ttls;
                    if next_slot < capacity {
                        slot = next_slot;
                        continue;
                    }

                    slot = 0;
                    shard += 1;
                    if shard >= shards {
                        shard = 0;
                        if store.map_shard_layout_matches(&capacities) {
                            store.finish_ttl_scan(generation, live_ttls);
                        }
                        let cur_keys = store.dbsize();
                        if cur_keys < last_key_count {
                            for s in 0..shards {
                                store.compact_shard(s);
                            }
                            customhash::force_collect();
                        }
                        last_key_count = cur_keys;
                        live_ttls = 0;
                        generation = store.ttl_generation();
                    }
                } else {
                    shard = 0;
                    slot = 0;
                    live_ttls = 0;
                    generation = store.ttl_generation();
                    let cur_keys = store.dbsize();
                    if cur_keys < last_key_count {
                        for s in 0..shards {
                            store.compact_shard(s);
                        }
                        customhash::force_collect();
                    }
                    last_key_count = cur_keys;
                }
            }
        })
        .expect("failed to spawn expiry thread");
}

fn spawn_signal_thread(store: Arc<Store>, rdb_path: String) {
    std::thread::Builder::new()
        .name("fyrodb-signal".into())
        .spawn(move || {
            let mut sig = 0i32;
            unsafe {
                let mut mask: libc::sigset_t = std::mem::zeroed();
                libc::sigemptyset(&mut mask);
                libc::sigaddset(&mut mask, libc::SIGTERM);
                libc::sigaddset(&mut mask, libc::SIGINT);
                libc::sigwait(&mask, &mut sig);
            }
            eprintln!("fyrodb: received signal {sig}, shutting down...");
            initiate_shutdown();
            std::thread::sleep(Duration::from_millis(100));
            if let Err(e) = fyro_db::storage::rdb::save(&store, &rdb_path) {
                eprintln!("fyrodb: save failed: {e}");
            }
            eprintln!("fyrodb: shutdown complete");
            std::process::exit(0);
        })
        .expect("failed to spawn signal thread");
}

mod libc {
    pub use ::libc::*;
}
