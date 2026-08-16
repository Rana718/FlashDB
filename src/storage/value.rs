use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

#[derive(Clone)]
pub enum FyroDB {
    String(String),
    Hash(Box<HashMap<String, String>>),
    List(Box<VecDeque<String>>),
    Set(Box<HashSet<String>>),
    ZSet(Box<ZSetData>),
    Json(Box<JsonValue>),
    Stream(Box<crate::storage::stream::StreamData>),
}

#[derive(Clone)]
pub struct ZSetData {
    pub dict: HashMap<String, f64>,
    pub tree: BTreeMap<ScoreKey, ()>,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ScoreKey {
    pub score_bits: i64,
    pub member: String,
}

impl ScoreKey {
    #[inline]
    pub fn new(score: f64, member: String) -> Self {
        Self {
            score_bits: f64_to_sorted_i64(score),
            member,
        }
    }

    #[inline]
    pub fn score(&self) -> f64 {
        sorted_i64_to_f64(self.score_bits)
    }
}

#[inline]
fn f64_to_sorted_i64(f: f64) -> i64 {
    let bits = f.to_bits() as i64;
    if bits < 0 {
        !bits
    } else {
        bits ^ (1i64 << 63)
    }
}

#[inline]
fn sorted_i64_to_f64(i: i64) -> f64 {
    let bits = if i < 0 { i ^ (1i64 << 63) } else { !i };
    f64::from_bits(bits as u64)
}

impl Default for ZSetData {
    fn default() -> Self {
        Self::new()
    }
}

impl ZSetData {
    pub fn new() -> Self {
        Self {
            dict: HashMap::new(),
            tree: BTreeMap::new(),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.dict.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.dict.is_empty()
    }

    #[inline]
    pub fn insert(&mut self, member: String, score: f64) -> bool {
        if let Some(&old_score) = self.dict.get(&member) {
            if (old_score - score).abs() < f64::EPSILON {
                return false;
            }
            self.tree.remove(&ScoreKey::new(old_score, member.clone()));
        }
        self.tree.insert(ScoreKey::new(score, member.clone()), ());
        
        self.dict.insert(member, score).is_none()
    }

    #[inline]
    pub fn remove(&mut self, member: &str) -> bool {
        if let Some(score) = self.dict.remove(member) {
            self.tree
                .remove(&ScoreKey::new(score, member.to_string()));
            true
        } else {
            false
        }
    }

    #[inline]
    pub fn score(&self, member: &str) -> Option<f64> {
        self.dict.get(member).copied()
    }

    pub fn rank(&self, member: &str) -> Option<usize> {
        let score = *self.dict.get(member)?;
        let target = ScoreKey::new(score, member.to_string());
        Some(self.tree.range(..&target).count())
    }

    pub fn rev_rank(&self, member: &str) -> Option<usize> {
        let score = *self.dict.get(member)?;
        let target = ScoreKey::new(score, member.to_string());
        Some(self.tree.range(&target..).count().saturating_sub(1))
    }
}

#[derive(Clone, Debug)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<JsonValue>),
    Object(Vec<(String, JsonValue)>),
}

impl JsonValue {
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "boolean",
            Self::Number(_) => "number",
            Self::String(_) => "string",
            Self::Array(_) => "array",
            Self::Object(_) => "object",
        }
    }

    pub fn to_resp_string(&self) -> String {
        match self {
            Self::Null => "null".to_string(),
            Self::Bool(b) => if *b { "true" } else { "false" }.to_string(),
            Self::Number(n) => {
                if *n == (*n as i64) as f64 {
                    format!("{}", *n as i64)
                } else {
                    format!("{}", n)
                }
            }
            Self::String(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
            Self::Array(arr) => {
                let items: Vec<String> = arr.iter().map(|v| v.to_resp_string()).collect();
                format!("[{}]", items.join(","))
            }
            Self::Object(obj) => {
                let items: Vec<String> = obj
                    .iter()
                    .map(|(k, v)| {
                        format!(
                            "\"{}\":{}",
                            k.replace('\\', "\\\\").replace('"', "\\\""),
                            v.to_resp_string()
                        )
                    })
                    .collect();
                format!("{{{}}}", items.join(","))
            }
        }
    }

    pub fn get_path(&self, path: &str) -> Option<&JsonValue> {
        if path == "." || path == "$" || path.is_empty() {
            return Some(self);
        }
        let normalized = path.strip_prefix("$.").or_else(|| path.strip_prefix('.'))?;
        let mut current = self;
        for part in normalized.split('.') {
            if let Some(idx_str) = part.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                if let Ok(idx) = idx_str.parse::<usize>() {
                    match current {
                        Self::Array(arr) => current = arr.get(idx)?,
                        _ => return None,
                    }
                } else {
                    return None;
                }
            } else {
                match current {
                    Self::Object(obj) => {
                        current = obj.iter().find(|(k, _)| k == part).map(|(_, v)| v)?;
                    }
                    _ => return None,
                }
            }
        }
        Some(current)
    }

    pub fn get_path_mut(&mut self, path: &str) -> Option<&mut JsonValue> {
        if path == "." || path == "$" || path.is_empty() {
            return Some(self);
        }
        let normalized = path.strip_prefix("$.").or_else(|| path.strip_prefix('.'))?;
        let mut current = self;
        for part in normalized.split('.') {
            if let Some(idx_str) = part.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                if let Ok(idx) = idx_str.parse::<usize>() {
                    match current {
                        Self::Array(arr) => current = arr.get_mut(idx)?,
                        _ => return None,
                    }
                } else {
                    return None;
                }
            } else {
                match current {
                    Self::Object(obj) => {
                        current = obj.iter_mut().find(|(k, _)| k == part).map(|(_, v)| v)?;
                    }
                    _ => return None,
                }
            }
        }
        Some(current)
    }

    pub fn set_path(&mut self, path: &str, value: JsonValue) -> bool {
        if path == "." || path == "$" || path.is_empty() {
            *self = value;
            return true;
        }
        let normalized = match path.strip_prefix("$.").or_else(|| path.strip_prefix('.')) {
            Some(n) => n,
            None => return false,
        };
        let parts: Vec<&str> = normalized.split('.').collect();
        if parts.is_empty() {
            return false;
        }
        let (parent_parts, last) = parts.split_at(parts.len() - 1);
        let last = last[0];

        let mut current = self;
        for part in parent_parts {
            if let Some(idx_str) = part.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                if let Ok(idx) = idx_str.parse::<usize>() {
                    match current {
                        Self::Array(arr) => match arr.get_mut(idx) {
                            Some(v) => current = v,
                            None => return false,
                        },
                        _ => return false,
                    }
                } else {
                    return false;
                }
            } else {
                match current {
                    Self::Object(obj) => {
                        match obj.iter_mut().find(|(k, _)| k == part).map(|(_, v)| v) {
                            Some(v) => current = v,
                            None => return false,
                        }
                    }
                    _ => return false,
                }
            }
        }

        if let Some(idx_str) = last.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            if let Ok(idx) = idx_str.parse::<usize>() {
                match current {
                    Self::Array(arr) if idx < arr.len() => {
                        arr[idx] = value;
                        true
                    }
                    _ => false,
                }
            } else {
                false
            }
        } else {
            match current {
                Self::Object(obj) => {
                    if let Some((_, v)) = obj.iter_mut().find(|(k, _)| k == last) {
                        *v = value;
                    } else {
                        obj.push((last.to_string(), value));
                    }
                    true
                }
                _ => false,
            }
        }
    }

    pub fn del_path(&mut self, path: &str) -> bool {
        if path == "." || path == "$" || path.is_empty() {
            return false;
        }
        let normalized = match path.strip_prefix("$.").or_else(|| path.strip_prefix('.')) {
            Some(n) => n,
            None => return false,
        };
        let parts: Vec<&str> = normalized.split('.').collect();
        if parts.is_empty() {
            return false;
        }
        let (parent_parts, last) = parts.split_at(parts.len() - 1);
        let last = last[0];

        let mut current = self;
        for part in parent_parts {
            match current {
                Self::Object(obj) => {
                    match obj.iter_mut().find(|(k, _)| k == part).map(|(_, v)| v) {
                        Some(v) => current = v,
                        None => return false,
                    }
                }
                _ => return false,
            }
        }

        match current {
            Self::Object(obj) => {
                let before = obj.len();
                obj.retain(|(k, _)| k != last);
                obj.len() < before
            }
            Self::Array(arr) => {
                if let Ok(idx) = last.parse::<usize>() {
                    if idx < arr.len() {
                        arr.remove(idx);
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    pub fn parse(input: &str) -> Option<JsonValue> {
        let input = input.trim();
        if input.is_empty() {
            return None;
        }
        let (val, rest) = parse_json_value(input.as_bytes())?;
        if rest.iter().all(|b| b.is_ascii_whitespace()) {
            Some(val)
        } else {
            None
        }
    }
}

fn parse_json_value(input: &[u8]) -> Option<(JsonValue, &[u8])> {
    let input = skip_ws(input);
    if input.is_empty() {
        return None;
    }
    match input[0] {
        b'"' => parse_json_string(input),
        b'{' => parse_json_object(input),
        b'[' => parse_json_array(input),
        b't' if input.starts_with(b"true") => Some((JsonValue::Bool(true), &input[4..])),
        b'f' if input.starts_with(b"false") => Some((JsonValue::Bool(false), &input[5..])),
        b'n' if input.starts_with(b"null") => Some((JsonValue::Null, &input[4..])),
        _ => parse_json_number(input),
    }
}

fn skip_ws(input: &[u8]) -> &[u8] {
    let mut i = 0;
    while i < input.len() && input[i].is_ascii_whitespace() {
        i += 1;
    }
    &input[i..]
}

fn parse_json_string(input: &[u8]) -> Option<(JsonValue, &[u8])> {
    if input.first() != Some(&b'"') {
        return None;
    }
    let mut i = 1;
    let mut s = String::new();
    while i < input.len() {
        match input[i] {
            b'"' => return Some((JsonValue::String(s), &input[i + 1..])),
            b'\\' => {
                i += 1;
                if i >= input.len() {
                    return None;
                }
                match input[i] {
                    b'"' => s.push('"'),
                    b'\\' => s.push('\\'),
                    b'/' => s.push('/'),
                    b'n' => s.push('\n'),
                    b'r' => s.push('\r'),
                    b't' => s.push('\t'),
                    b'b' => s.push('\x08'),
                    b'f' => s.push('\x0c'),
                    b'u' => {
                        if i + 4 >= input.len() {
                            return None;
                        }
                        let hex = std::str::from_utf8(&input[i + 1..i + 5]).ok()?;
                        let cp = u32::from_str_radix(hex, 16).ok()?;
                        s.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                        i += 4;
                    }
                    _ => return None,
                }
            }
            b => s.push(b as char),
        }
        i += 1;
    }
    None
}

fn parse_json_number(input: &[u8]) -> Option<(JsonValue, &[u8])> {
    let mut i = 0;
    if i < input.len() && input[i] == b'-' {
        i += 1;
    }
    while i < input.len() && input[i].is_ascii_digit() {
        i += 1;
    }
    if i < input.len() && input[i] == b'.' {
        i += 1;
        while i < input.len() && input[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i < input.len() && (input[i] == b'e' || input[i] == b'E') {
        i += 1;
        if i < input.len() && (input[i] == b'+' || input[i] == b'-') {
            i += 1;
        }
        while i < input.len() && input[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i == 0 {
        return None;
    }
    let s = std::str::from_utf8(&input[..i]).ok()?;
    let n = s.parse::<f64>().ok()?;
    Some((JsonValue::Number(n), &input[i..]))
}

fn parse_json_object(input: &[u8]) -> Option<(JsonValue, &[u8])> {
    let mut rest = skip_ws(&input[1..]);
    let mut obj = Vec::new();
    if rest.first() == Some(&b'}') {
        return Some((JsonValue::Object(obj), &rest[1..]));
    }
    loop {
        rest = skip_ws(rest);
        let (key_val, r) = parse_json_string(rest)?;
        let key = match key_val {
            JsonValue::String(s) => s,
            _ => return None,
        };
        rest = skip_ws(r);
        if rest.first() != Some(&b':') {
            return None;
        }
        rest = skip_ws(&rest[1..]);
        let (val, r) = parse_json_value(rest)?;
        obj.push((key, val));
        rest = skip_ws(r);
        match rest.first() {
            Some(&b',') => rest = &rest[1..],
            Some(&b'}') => return Some((JsonValue::Object(obj), &rest[1..])),
            _ => return None,
        }
    }
}

fn parse_json_array(input: &[u8]) -> Option<(JsonValue, &[u8])> {
    let mut rest = skip_ws(&input[1..]);
    let mut arr = Vec::new();
    if rest.first() == Some(&b']') {
        return Some((JsonValue::Array(arr), &rest[1..]));
    }
    loop {
        rest = skip_ws(rest);
        let (val, r) = parse_json_value(rest)?;
        arr.push(val);
        rest = skip_ws(r);
        match rest.first() {
            Some(&b',') => rest = &rest[1..],
            Some(&b']') => return Some((JsonValue::Array(arr), &rest[1..])),
            _ => return None,
        }
    }
}

impl FyroDB {
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::String(_) => "string",
            Self::Hash(_) => "hash",
            Self::List(_) => "list",
            Self::Set(_) => "set",
            Self::ZSet(_) => "zset",
            Self::Json(_) => "ReJSON-RL",
            Self::Stream(_) => "stream",
        }
    }

    pub fn as_string(&self) -> Option<&String> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_string_mut(&mut self) -> Option<&mut String> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_hash(&self) -> Option<&HashMap<String, String>> {
        match self {
            Self::Hash(h) => Some(h),
            _ => None,
        }
    }

    pub fn as_hash_mut(&mut self) -> Option<&mut HashMap<String, String>> {
        match self {
            Self::Hash(h) => Some(h),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&VecDeque<String>> {
        match self {
            Self::List(l) => Some(l),
            _ => None,
        }
    }

    pub fn as_list_mut(&mut self) -> Option<&mut VecDeque<String>> {
        match self {
            Self::List(l) => Some(l),
            _ => None,
        }
    }

    pub fn as_set(&self) -> Option<&HashSet<String>> {
        match self {
            Self::Set(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_set_mut(&mut self) -> Option<&mut HashSet<String>> {
        match self {
            Self::Set(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_zset(&self) -> Option<&ZSetData> {
        match self {
            Self::ZSet(z) => Some(z),
            _ => None,
        }
    }

    pub fn as_zset_mut(&mut self) -> Option<&mut ZSetData> {
        match self {
            Self::ZSet(z) => Some(z),
            _ => None,
        }
    }

    pub fn as_json(&self) -> Option<&JsonValue> {
        match self {
            Self::Json(j) => Some(j),
            _ => None,
        }
    }

    pub fn as_json_mut(&mut self) -> Option<&mut JsonValue> {
        match self {
            Self::Json(j) => Some(j),
            _ => None,
        }
    }

    pub fn as_stream(&self) -> Option<&crate::storage::stream::StreamData> {
        match self {
            Self::Stream(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_stream_mut(&mut self) -> Option<&mut crate::storage::stream::StreamData> {
        match self {
            Self::Stream(s) => Some(s),
            _ => None,
        }
    }

    pub fn mem_size(&self) -> usize {
        match self {
            Self::String(s) => s.len(),
            Self::Hash(h) => h.iter().map(|(k, v)| k.len() + v.len()).sum(),
            Self::List(l) => l.iter().map(|s| s.len()).sum::<usize>() + l.len() * 24,
            Self::Set(s) => s.iter().map(|m| m.len() + 24).sum(),
            Self::ZSet(z) => z.dict.keys().map(|k| k.len() + 32).sum(),
            Self::Json(_) => 256,
            Self::Stream(s) => s.entries.len() * 128,
        }
    }
}

#[derive(Clone)]
pub struct StoreValue {
    pub value: FyroDB,
    pub expires_ms: u64,
}

static UNIX_MS_CACHE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[inline(always)]
pub fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[inline]
pub fn tick_clock() {
    UNIX_MS_CACHE.store(now_ms(), std::sync::atomic::Ordering::Relaxed);
}

#[inline]
pub fn expiry_from_secs(secs: u64) -> Option<u64> {
    approx_now_ms().checked_add(secs.checked_mul(1000)?)
}

#[inline]
pub fn expiry_from_ms(ms: u64) -> Option<u64> {
    approx_now_ms().checked_add(ms)
}

#[inline]
pub fn expiry_from_unix_secs(unix_secs: u64) -> Option<u64> {
    unix_secs.checked_mul(1000)
}

#[inline(always)]
pub fn approx_now_ms() -> u64 {
    let cached = UNIX_MS_CACHE.load(std::sync::atomic::Ordering::Relaxed);
    if cached == 0 { now_ms() } else { cached }
}

impl StoreValue {
    #[inline]
    pub fn string(s: String) -> Self {
        Self {
            value: FyroDB::String(s),
            expires_ms: 0,
        }
    }

    #[inline]
    pub fn string_with_expiry(s: String, expires_at: Option<std::time::Instant>) -> Self {
        let expires_ms = match expires_at {
            None => 0,
            Some(exp) => {
                let remaining = exp.saturating_duration_since(std::time::Instant::now());
                now_ms().saturating_add(remaining.as_millis() as u64)
            }
        };
        Self {
            value: FyroDB::String(s),
            expires_ms,
        }
    }

    #[inline]
    pub fn hash(h: HashMap<String, String>) -> Self {
        Self {
            value: FyroDB::Hash(Box::new(h)),
            expires_ms: 0,
        }
    }

    #[inline]
    pub fn list(l: VecDeque<String>) -> Self {
        Self {
            value: FyroDB::List(Box::new(l)),
            expires_ms: 0,
        }
    }

    #[inline]
    pub fn set(s: HashSet<String>) -> Self {
        Self {
            value: FyroDB::Set(Box::new(s)),
            expires_ms: 0,
        }
    }

    #[inline]
    pub fn zset(z: ZSetData) -> Self {
        Self {
            value: FyroDB::ZSet(Box::new(z)),
            expires_ms: 0,
        }
    }

    #[inline]
    pub fn json(j: JsonValue) -> Self {
        Self {
            value: FyroDB::Json(Box::new(j)),
            expires_ms: 0,
        }
    }

    #[inline]
    pub fn stream(s: crate::storage::stream::StreamData) -> Self {
        Self {
            value: FyroDB::Stream(Box::new(s)),
            expires_ms: 0,
        }
    }

    #[inline(always)]
    pub fn is_expired(&self) -> bool {
        self.expires_ms != 0 && approx_now_ms() >= self.expires_ms
    }

    #[inline]
    pub fn is_expired_precise(&self) -> bool {
        self.expires_ms != 0 && now_ms() >= self.expires_ms
    }

    #[inline]
    pub fn ttl_ms(&self) -> Option<u64> {
        if self.expires_ms == 0 {
            return None;
        }
        let now = now_ms();
        if now >= self.expires_ms {
            None
        } else {
            Some(self.expires_ms - now)
        }
    }
}
