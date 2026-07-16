use dashmap::DashMap;
use std::sync::LazyLock;
use tokio::time::Instant;

#[derive(Clone)]
pub struct StoreValue {
    pub value: String,
    pub expires_at: Option<Instant>,
}

pub static DATAS: LazyLock<DashMap<i32, StoreValue>> = LazyLock::new(DashMap::new);

pub fn setval(key: i32, value: StoreValue) {
    DATAS.insert(key, value);
}

pub fn getval(key: i32) -> Option<String> {
    let data = DATAS.get(&key)?;

    if let Some(exp) = data.expires_at {
        if Instant::now() >= exp {
            drop(data);
            DATAS.remove(&key);
            return None;
        }
    }
    Some(data.value.clone())
}

pub fn delval(key: i32) -> bool {
    DATAS.remove(&key).is_some()
}
