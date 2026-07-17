use std::collections::HashMap;
use tokio::time::Instant;

#[derive(Clone)]
pub enum FlashDB {
    String(String),
    Hash(HashMap<String, String>),
}

impl FlashDB {
    pub fn type_name(&self) -> &'static str {
        match self {
            FlashDB::String(_) => "string",
            FlashDB::Hash(_) => "hash",
        }
    }

    pub fn as_string(&self) -> Option<&String> {
        match self {
            FlashDB::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_string_mut(&mut self) -> Option<&mut String> {
        match self {
            FlashDB::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_hash(&self) -> Option<&HashMap<String, String>> {
        match self {
            FlashDB::Hash(h) => Some(h),
            _ => None,
        }
    }

    pub fn as_hash_mut(&mut self) -> Option<&mut HashMap<String, String>> {
        match self {
            FlashDB::Hash(h) => Some(h),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct StoreValue {
    pub value: FlashDB,
    pub expires_at: Option<Instant>,
}

impl StoreValue {
    pub fn string(s: String) -> Self {
        Self {
            value: FlashDB::String(s),
            expires_at: None,
        }
    }

    pub fn string_with_expiry(s: String, expires_at: Option<Instant>) -> Self {
        Self {
            value: FlashDB::String(s),
            expires_at,
        }
    }

    pub fn hash(h: HashMap<String, String>) -> Self {
        Self {
            value: FlashDB::Hash(h),
            expires_at: None,
        }
    }

    pub fn is_expired(&self) -> bool {
        self.expires_at.map_or(false, |exp| Instant::now() >= exp)
    }
}
