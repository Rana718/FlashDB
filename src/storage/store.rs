use super::value::StoreValue;
use dashmap::DashMap;

#[derive(Clone)]
pub struct Store {
    pub(crate) data: DashMap<String, StoreValue>,
}

impl Store {
    pub fn new() -> Self {
        Self {
            data: DashMap::new(),
        }
    }
}
