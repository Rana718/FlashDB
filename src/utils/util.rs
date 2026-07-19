pub fn format_float(f: f64) -> String {
    let s = format!("{:.17}", f);
    let s = s.trim_end_matches('0');
    let s = s.trim_end_matches('.');
    s.to_string()
}

pub fn glob_match(pattern: &str, input: &str) -> bool {
    glob_match_bytes(pattern.as_bytes(), input.as_bytes())
}

pub fn glob_match_bytes(p: &[u8], s: &[u8]) -> bool {
    glob_recurse(p, s, 0, 0)
}

fn glob_recurse(p: &[u8], s: &[u8], pi: usize, si: usize) -> bool {
    if pi == p.len() {
        return si == s.len();
    }
    match p[pi] {
        b'*' => (si..=s.len()).any(|i| glob_recurse(p, s, pi + 1, i)),
        b'?' => si < s.len() && glob_recurse(p, s, pi + 1, si + 1),
        c => si < s.len() && s[si] == c && glob_recurse(p, s, pi + 1, si + 1),
    }
}
