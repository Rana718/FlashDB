#[macro_export]
macro_rules! hash_read {
    ($self:expr, $key:expr, $default:expr, |$h:ident| $body:expr) => {{
        match $self.data.get($key) {
            None => Ok($default),
            Some(e) if e.is_expired() => Ok($default),
            Some(e) => match e.value.as_hash() {
                Some($h) => Ok($body),
                None => Err("WRONGTYPE"),
            },
        }
    }};
}

#[macro_export]
macro_rules! hash_write {
    ($self:expr, $key:expr, |$h:ident| $body:expr) => {{
        let mut entry = $self.data.entry($key.to_string()).or_insert_with(|| {
            crate::storage::value::StoreValue::hash(std::collections::HashMap::new())
        });
        match entry.value.as_hash_mut() {
            Some($h) => Ok($body),
            None => Err("WRONGTYPE"),
        }
    }};
}
