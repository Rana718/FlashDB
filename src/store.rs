use dashmap::DashMap;
use std::sync::LazyLock;

pub static USERS: LazyLock<DashMap<i32, String>> = LazyLock::new(DashMap::new);

pub fn setval(key: i32, value: String) {
    USERS.insert(key, value);
}

pub fn getval(key: i32) -> Option<String> {
    USERS.get(&key).map(|v| v.value().clone())
}

pub fn delval(key: i32) -> bool {
    USERS.remove(&key).is_some()
}