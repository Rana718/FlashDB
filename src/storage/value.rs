use tokio::time::Instant;

#[derive(Clone)]
pub struct StoreValue {
    pub value: String,
    pub expires_at: Option<Instant>,
}