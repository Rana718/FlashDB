use crate::storage::store::Store;

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

        let chunk: Vec<String> = all_keys[start..end]
            .iter()
            .filter(|k| match pattern {
                Some(pat) => glob_match(pat, k),
                None => true,
            })
            .cloned()
            .collect();

        let next_cursor = if end >= total { 0 } else { end };

        (next_cursor, chunk)
    }
}

fn glob_match(pattern: &str, input: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let s: Vec<char> = input.chars().collect();
    glob_recurse(&p, &s, 0, 0)
}

fn glob_recurse(p: &[char], s: &[char], pi: usize, si: usize) -> bool {
    if pi == p.len() {
        return si == s.len();
    }

    match p[pi] {
        '*' => {
            for i in si..=s.len() {
                if glob_recurse(p, s, pi + 1, i) {
                    return true;
                }
            }
            false
        }
        '?' => {
            if si < s.len() {
                glob_recurse(p, s, pi + 1, si + 1)
            } else {
                false
            }
        }
        c => {
            if si < s.len() && s[si] == c {
                glob_recurse(p, s, pi + 1, si + 1)
            } else {
                false
            }
        }
    }
}
