use crate::storage::store::Store;
use crate::utils::util::glob_match;

impl Store {
    pub fn scan(&self, cursor: usize, pattern: Option<&str>, count: usize) -> (usize, Vec<String>) {
        let mut all_keys: Vec<String> = self.data.iter().map(|e| e.key().clone()).collect();
        all_keys.sort();

        let total = all_keys.len();
        if total == 0 {
            return (0, vec![]);
        }

        let start = if cursor == 0 || cursor >= total {
            0
        } else {
            cursor
        };
        let end = (start + count).min(total);

        let chunk = all_keys[start..end]
            .iter()
            .filter(|k| pattern.map_or(true, |pat| glob_match(pat, k)))
            .cloned()
            .collect();

        let next_cursor = if end >= total { 0 } else { end };
        (next_cursor, chunk)
    }
}
