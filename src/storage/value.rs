use foldhash::{HashMap, HashMapExt, HashSet};
use std::collections::VecDeque;

const COMPACT_THRESHOLD: usize = 64;
const SMALL_STR_CAP: usize = 15;

#[derive(Clone)]
#[repr(C)]
pub struct SmallStr {
    data: [u8; SMALL_STR_CAP],
    len: u8,
}

impl SmallStr {
    #[inline]
    fn store_heap_len(data: &mut [u8; SMALL_STR_CAP], len: usize) {
        let lb = (len as u64).to_ne_bytes();
        data[8..15].copy_from_slice(&lb[..7]);
    }

    #[inline(always)]
    fn read_heap_len(data: &[u8; SMALL_STR_CAP]) -> usize {
        let mut lb = [0u8; 8];
        lb[..7].copy_from_slice(&data[8..15]);
        u64::from_ne_bytes(lb) as usize
    }

    #[inline]
    pub fn new(s: &str) -> Self {
        if s.len() <= SMALL_STR_CAP {
            let mut data = [0u8; SMALL_STR_CAP];
            data[..s.len()].copy_from_slice(s.as_bytes());
            Self { data, len: s.len() as u8 }
        } else {
            let ptr = Box::into_raw(s.to_owned().into_boxed_str());
            let mut data = [0u8; SMALL_STR_CAP];
            let bytes = (ptr as *const u8 as usize).to_ne_bytes();
            data[..8].copy_from_slice(&bytes);
            Self::store_heap_len(&mut data, s.len());
            Self { data, len: 0xFF }
        }
    }

    #[inline]
    pub fn from_string(s: String) -> Self {
        if s.len() <= SMALL_STR_CAP {
            let mut data = [0u8; SMALL_STR_CAP];
            data[..s.len()].copy_from_slice(s.as_bytes());
            let len = s.len() as u8;
            drop(s);
            Self { data, len }
        } else {
            let ptr = Box::into_raw(s.into_boxed_str());
            let mut data = [0u8; SMALL_STR_CAP];
            let bytes = (ptr as *const u8 as usize).to_ne_bytes();
            data[..8].copy_from_slice(&bytes);
            Self::store_heap_len(&mut data, unsafe { &*ptr }.len());
            Self { data, len: 0xFF }
        }
    }

    #[inline(always)]
    pub fn as_str(&self) -> &str {
        if self.len != 0xFF {
            unsafe { std::str::from_utf8_unchecked(&self.data[..self.len as usize]) }
        } else {
            let ptr_val = usize::from_ne_bytes(self.data[..8].try_into().unwrap());
            let len_val = Self::read_heap_len(&self.data);
            unsafe {
                let slice = std::slice::from_raw_parts(ptr_val as *const u8, len_val);
                std::str::from_utf8_unchecked(slice)
            }
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        if self.len != 0xFF {
            self.len as usize
        } else {
            Self::read_heap_len(&self.data)
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn into_string(self) -> String {
        if self.len != 0xFF {
            let s = self.as_str().to_owned();
            std::mem::forget(self);
            s
        } else {
            let ptr_val = usize::from_ne_bytes(self.data[..8].try_into().unwrap());
            let len_val = Self::read_heap_len(&self.data);
            let boxed = unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr_val as *mut u8, len_val) as *mut str) };
            std::mem::forget(self);
            boxed.into()
        }
    }

    #[inline]
    pub fn push_str(&mut self, s: &str) {
        let mut owned = self.as_str().to_owned();
        owned.push_str(s);
        *self = Self::from_string(owned);
    }

    #[inline]
    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        if self.len != 0xFF {
            &mut self.data[..self.len as usize]
        } else {
            let ptr_val = usize::from_ne_bytes(self.data[..8].try_into().unwrap());
            let len_val = Self::read_heap_len(&self.data);
            unsafe { std::slice::from_raw_parts_mut(ptr_val as *mut u8, len_val) }
        }
    }

    #[inline]
    pub fn into_bytes(self) -> Vec<u8> {
        self.into_string().into_bytes()
    }

    #[inline]
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self::from_string(unsafe { String::from_utf8_unchecked(bytes) })
    }

    #[inline]
    pub fn ensure_len(&mut self, min_len: usize) {
        if self.len() < min_len {
            let mut owned = self.as_str().to_owned();
            owned.extend(std::iter::repeat_n('\0', min_len - owned.len()));
            *self = Self::from_string(owned);
        }
    }
}

impl Drop for SmallStr {
    fn drop(&mut self) {
        if self.len == 0xFF {
            let ptr_val = usize::from_ne_bytes(self.data[..8].try_into().unwrap());
            let len_val = Self::read_heap_len(&self.data);
            unsafe {
                drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr_val as *mut u8, len_val) as *mut str));
            }
        }
    }
}

impl std::ops::Deref for SmallStr {
    type Target = str;
    #[inline(always)]
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl Default for SmallStr {
    fn default() -> Self {
        Self { data: [0u8; SMALL_STR_CAP], len: 0 }
    }
}

impl From<String> for SmallStr {
    fn from(s: String) -> Self { Self::from_string(s) }
}

impl From<&str> for SmallStr {
    fn from(s: &str) -> Self { Self::new(s) }
}

impl std::fmt::Display for SmallStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::fmt::Debug for SmallStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "\"{}\"", self.as_str())
    }
}

impl PartialEq for SmallStr {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for SmallStr {}

impl std::hash::Hash for SmallStr {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl std::borrow::Borrow<str> for SmallStr {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq<str> for SmallStr {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for SmallStr {
    fn eq(&self, other: &&str) -> bool { self.as_str() == *other }
}

#[derive(Clone)]
pub enum FyroDB {
    String(SmallStr),
    Hash(Box<HashInner>),
    List(Box<ListInner>),
    Set(Box<SetInner>),
    ZSet(Box<ZSetData>),
    Json(SmallStr),
    Stream(Box<crate::storage::stream::StreamData>),
}

#[derive(Clone)]
pub enum HashInner {
    Compact(Vec<SmallStr>),
    Full(Box<HashMap<SmallStr, SmallStr>>),
}

#[derive(Clone)]
pub enum ListInner {
    Compact(VecDeque<SmallStr>),
    Full(Box<VecDeque<SmallStr>>),
}

#[derive(Clone)]
pub enum SetInner {
    Integers(Vec<i64>),
    Compact(Vec<SmallStr>),
    Full(Box<HashSet<SmallStr>>),
}

impl Default for HashInner {
    fn default() -> Self { Self::new() }
}

impl HashInner {
    #[inline]
    pub fn new() -> Self {
        Self::Compact(Vec::new())
    }

    #[inline]
    pub fn get(&self, field: &str) -> Option<&SmallStr> {
        match self {
            Self::Compact(v) => {
                let mut i = 0;
                while i < v.len() {
                    if v[i] == field {
                        return Some(&v[i + 1]);
                    }
                    i += 2;
                }
                None
            }
            Self::Full(m) => m.get(field),
        }
    }

    #[inline]
    pub fn insert(&mut self, field: String, value: String) -> bool {
        self.insert_small(SmallStr::from_string(field), SmallStr::from_string(value))
    }

    #[inline]
    pub fn insert_small(&mut self, field: SmallStr, value: SmallStr) -> bool {
        match self {
            Self::Compact(v) => {
                let mut i = 0;
                while i < v.len() {
                    if v[i] == field {
                        v[i + 1] = value;
                        return false;
                    }
                    i += 2;
                }
                v.push(field);
                v.push(value);
                if v.len() / 2 > COMPACT_THRESHOLD {
                    self.promote_hash();
                }
                true
            }
            Self::Full(m) => m.insert(field, value).is_none(),
        }
    }

    #[inline]
    pub fn remove(&mut self, field: &str) -> Option<String> {
        match self {
            Self::Compact(v) => {
                let mut i = 0;
                while i < v.len() {
                    if v[i] == field {
                        let len = v.len();
                        v.swap(i, len - 2);
                        v.swap(i + 1, len - 1);
                        v.pop();
                        let out = v.pop().map(|s| s.into_string());
                        if v.capacity() > v.len().saturating_mul(2).max(8) {
                            v.shrink_to_fit();
                        }
                        return out;
                    }
                    i += 2;
                }
                None
            }
            Self::Full(m) => {
                let out = m.remove(field).map(|s| s.into_string());
                if m.len() <= COMPACT_THRESHOLD / 2 {
                    self.demote_hash();
                }
                out
            }
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        match self {
            Self::Compact(v) => v.len() / 2,
            Self::Full(m) => m.len(),
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn contains_key(&self, field: &str) -> bool {
        self.get(field).is_some()
    }

    pub fn keys(&self) -> Vec<&SmallStr> {
        match self {
            Self::Compact(v) => v.iter().step_by(2).collect(),
            Self::Full(m) => m.keys().collect(),
        }
    }

    pub fn values(&self) -> Vec<&SmallStr> {
        match self {
            Self::Compact(v) => v.iter().skip(1).step_by(2).collect(),
            Self::Full(m) => m.values().collect(),
        }
    }

    pub fn iter(&self) -> HashIter<'_> {
        match self {
            Self::Compact(v) => HashIter::Compact(v, 0),
            Self::Full(m) => HashIter::Full(m.iter()),
        }
    }

    fn promote_hash(&mut self) {
        if let Self::Compact(v) = self {
            let mut map = HashMap::with_capacity(v.len() / 2);
            let mut i = 0;
            while i < v.len() {
                let val = std::mem::take(&mut v[i + 1]);
                let key = std::mem::take(&mut v[i]);
                map.insert(key, val);
                i += 2;
            }
            *self = Self::Full(Box::new(map));
        }
    }

    fn demote_hash(&mut self) {
        let Self::Full(m) = self else { return };
        if m.len() > COMPACT_THRESHOLD / 2 { return; }
        let mut v = Vec::with_capacity(m.len() * 2);
        for (k, val) in std::mem::take(m).into_iter() {
            v.push(k);
            v.push(val);
        }
        *self = Self::Compact(v);
    }
}

pub enum HashIter<'a> {
    Compact(&'a Vec<SmallStr>, usize),
    Full(std::collections::hash_map::Iter<'a, SmallStr, SmallStr>),
}

impl<'a> Iterator for HashIter<'a> {
    type Item = (&'a SmallStr, &'a SmallStr);
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Compact(v, i) => {
                if *i >= v.len() {
                    return None;
                }
                let pair = (&v[*i], &v[*i + 1]);
                *i += 2;
                Some(pair)
            }
            Self::Full(it) => it.next(),
        }
    }
}

impl Default for SetInner {
    fn default() -> Self { Self::new() }
}

impl SetInner {
    #[inline]
    pub fn new() -> Self {
        Self::Integers(Vec::new())
    }

    pub fn from_strings(values: impl IntoIterator<Item = String>) -> Self {
        let mut set = Self::new();
        for value in values { set.insert(value); }
        set
    }

    #[inline]
    pub fn insert(&mut self, member: String) -> bool {
        self.insert_small(SmallStr::from_string(member))
    }

    #[inline]
    pub fn insert_small(&mut self, member: SmallStr) -> bool {
        match self {
            Self::Integers(v) => {
                if let Some(n) = canonical_i64(member.as_str()) {
                    match v.binary_search(&n) {
                        Ok(_) => false,
                        Err(pos) => { v.insert(pos, n); true }
                    }
                } else {
                    let mut strings = Vec::with_capacity(v.len() + 1);
                    strings.extend(v.drain(..).map(|n| SmallStr::from_string(n.to_string())));
                    strings.push(member);
                    *self = Self::Compact(strings);
                    true
                }
            }
            Self::Compact(v) => {
                if v.contains(&member) {
                    return false;
                }
                v.push(member);
                if v.len() > COMPACT_THRESHOLD {
                    self.promote_set();
                }
                true
            }
            Self::Full(s) => s.insert(member),
        }
    }

    #[inline]
    pub fn remove(&mut self, member: &str) -> bool {
        match self {
            Self::Integers(v) => canonical_i64(member)
                .and_then(|n| v.binary_search(&n).ok())
                .map(|pos| { v.remove(pos); true })
                .unwrap_or(false),
            Self::Compact(v) => {
                if let Some(pos) = v.iter().position(|m| m == member) {
                    v.swap_remove(pos);
                    if v.capacity() > v.len().saturating_mul(2).max(8) {
                        v.shrink_to_fit();
                    }
                    true
                } else {
                    false
                }
            }
            Self::Full(s) => {
                let removed = s.remove(member);
                if s.len() <= COMPACT_THRESHOLD / 2 {
                    self.demote_set();
                }
                removed
            }
        }
    }

    #[inline]
    pub fn contains(&self, member: &str) -> bool {
        match self {
            Self::Integers(v) => canonical_i64(member).is_some_and(|n| v.binary_search(&n).is_ok()),
            Self::Compact(v) => v.iter().any(|m| m == member),
            Self::Full(s) => s.contains(member),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        match self {
            Self::Integers(v) => v.len(),
            Self::Compact(v) => v.len(),
            Self::Full(s) => s.len(),
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn iter(&self) -> SetIter<'_> {
        match self {
            Self::Integers(v) => SetIter::Integers(v.iter()),
            Self::Compact(v) => SetIter::Compact(v.iter()),
            Self::Full(s) => SetIter::Full(s.iter()),
        }
    }

    fn promote_set(&mut self) {
        if let Self::Compact(v) = self {
            let mut set: HashSet<SmallStr> = HashSet::with_capacity_and_hasher(v.len(), Default::default());
            for item in v.drain(..) {
                set.insert(item);
            }
            *self = Self::Full(Box::new(set));
        }
    }

    fn demote_set(&mut self) {
        let Self::Full(s) = self else { return };
        if s.len() > COMPACT_THRESHOLD / 2 { return; }
        let v = std::mem::take(s).into_iter().collect();
        *self = Self::Compact(v);
    }
}

pub enum SetIter<'a> {
    Integers(std::slice::Iter<'a, i64>),
    Compact(std::slice::Iter<'a, SmallStr>),
    Full(std::collections::hash_set::Iter<'a, SmallStr>),
}

impl<'a> Iterator for SetIter<'a> {
    type Item = SetMemberRef<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Integers(it) => it.next().copied().map(SetMemberRef::Integer),
            Self::Compact(it) => it.next().map(SetMemberRef::String),
            Self::Full(it) => it.next().map(SetMemberRef::String),
        }
    }
}

#[derive(Clone, Copy)]
pub enum SetMemberRef<'a> { Integer(i64), String(&'a SmallStr) }

impl SetMemberRef<'_> {
    pub fn len(self) -> usize { self.to_string().len() }
    pub fn matches(self, value: &str) -> bool {
        match self {
            Self::Integer(n) => canonical_i64(value) == Some(n),
            Self::String(s) => s.as_str() == value,
        }
    }
}

impl std::fmt::Display for SetMemberRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self { Self::Integer(n) => n.fmt(f), Self::String(s) => s.fmt(f) }
    }
}

fn canonical_i64(value: &str) -> Option<i64> {
    let n = value.parse::<i64>().ok()?;
    (n.to_string() == value).then_some(n)
}

impl Default for ListInner {
    fn default() -> Self { Self::new() }
}

impl ListInner {
    #[inline]
    pub fn new() -> Self {
        Self::Compact(VecDeque::new())
    }

    #[inline]
    pub fn push_front(&mut self, value: String) {
        let value = SmallStr::from_string(value);
        match self {
            Self::Compact(v) => {
                v.push_front(value);
                if v.len() > COMPACT_THRESHOLD {
                    self.promote_list();
                }
            }
            Self::Full(v) => v.push_front(value),
        }
    }

    #[inline]
    pub fn push_back(&mut self, value: String) {
        let value = SmallStr::from_string(value);
        match self {
            Self::Compact(v) => {
                v.push_back(value);
                if v.len() > COMPACT_THRESHOLD {
                    self.promote_list();
                }
            }
            Self::Full(v) => v.push_back(value),
        }
    }

    #[inline]
    pub fn pop_front(&mut self) -> Option<String> {
        match self {
            Self::Compact(v) => v.pop_front().map(SmallStr::into_string),
            Self::Full(v) => {
                let out = v.pop_front().map(SmallStr::into_string);
                if v.len() <= COMPACT_THRESHOLD / 2 { self.demote_list(); }
                out
            }
        }
    }

    #[inline]
    pub fn pop_back(&mut self) -> Option<String> {
        match self {
            Self::Compact(v) => v.pop_back().map(SmallStr::into_string),
            Self::Full(v) => {
                let out = v.pop_back().map(SmallStr::into_string);
                if v.len() <= COMPACT_THRESHOLD / 2 { self.demote_list(); }
                out
            }
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        match self {
            Self::Compact(v) => v.len(),
            Self::Full(v) => v.len(),
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn deque(&self) -> &VecDeque<SmallStr> {
        match self {
            Self::Compact(v) => v,
            Self::Full(v) => v,
        }
    }

    pub fn deque_mut(&mut self) -> &mut VecDeque<SmallStr> {
        match self {
            Self::Compact(v) => v,
            Self::Full(v) => v,
        }
    }

    fn promote_list(&mut self) {
        if let Self::Compact(v) = self {
            let boxed = Box::new(std::mem::take(v));
            *self = Self::Full(boxed);
        }
    }

    fn demote_list(&mut self) {
        let Self::Full(v) = self else { return };
        if v.len() > COMPACT_THRESHOLD / 2 { return; }
        *self = Self::Compact(std::mem::take(v));
    }
}

#[derive(Clone)]
pub struct ZSetData {
    entries: Vec<ZEntry>,
    fingerprints: Vec<u64>,
}


#[derive(Clone)]
pub struct ZEntry {
    pub score: f64,
    pub member: SmallStr,
}

impl ZSetData {
    pub fn new() -> Self {
        Self { entries: Vec::new(), fingerprints: vec![0] }
    }

    pub fn with_capacity(cap: usize) -> Self {
        let words = (cap.saturating_mul(8).next_power_of_two().max(64) / 64).max(1);
        Self { entries: Vec::with_capacity(cap), fingerprints: vec![0; words] }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn insert(&mut self, score: f64, member: &str) -> bool {
        let fingerprint = member_fingerprint(member);
        if self.maybe_contains_fingerprint(fingerprint)
            && let Some(pos) = self.entries.iter().position(|e| e.member.as_str() == member)
        {
            let old_score = self.entries[pos].score;
            if (old_score - score).abs() > f64::EPSILON {
                self.entries.remove(pos);
                let insert_pos = self.find_insert_pos(score, member);
                self.entries.insert(insert_pos, ZEntry { score, member: SmallStr::new(member) });
            }
            false
        } else {
            // ZADD workloads commonly arrive in score order. Appending avoids the
            // O(n) Vec shift for that hot path while retaining sorted ordering.
            if self.entries.last().is_none_or(|last| {
                last.score < score || (last.score == score && last.member.as_str() <= member)
            }) {
                self.entries.push(ZEntry { score, member: SmallStr::new(member) });
            } else {
                let insert_pos = self.find_insert_pos(score, member);
                self.entries.insert(insert_pos, ZEntry { score, member: SmallStr::new(member) });
            }
            self.ensure_fingerprint_capacity();
            self.set_fingerprint(fingerprint);
            true
        }
    }

    pub fn remove(&mut self, member: &str) -> Option<f64> {
        let pos = self.entries.iter().position(|e| e.member.as_str() == member)?;
        let score = self.entries[pos].score;
        self.entries.remove(pos);
        self.rebuild_fingerprints();
        self.reclaim_capacity();
        Some(score)
    }

    #[inline]
    pub fn get_score(&self, member: &str) -> Option<f64> {
        self.entries.iter().find(|e| e.member.as_str() == member).map(|e| e.score)
    }

    pub fn rank(&self, member: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.member.as_str() == member)
    }

    pub fn rev_rank(&self, member: &str) -> Option<usize> {
        self.rank(member).map(|r| self.entries.len() - 1 - r)
    }

    pub fn range_by_rank(&self, start: usize, end: usize) -> &[ZEntry] {
        let end = end.min(self.entries.len());
        if start >= end {
            return &[];
        }
        &self.entries[start..end]
    }

    pub fn range_by_score(&self, min: f64, max: f64) -> Vec<&ZEntry> {
        self.entries.iter().filter(|e| e.score >= min && e.score <= max).collect()
    }

    pub fn rev_range_by_score(&self, min: f64, max: f64) -> Vec<&ZEntry> {
        let mut v: Vec<&ZEntry> = self.entries.iter().filter(|e| e.score >= min && e.score <= max).collect();
        v.reverse();
        v
    }

    pub fn count_in_score_range(&self, min: f64, max: f64) -> usize {
        self.entries.iter().filter(|e| e.score >= min && e.score <= max).count()
    }

    pub fn pop_min(&mut self) -> Option<ZEntry> {
        if self.entries.is_empty() { return None; }
        let entry = self.entries.remove(0);
        self.rebuild_fingerprints();
        self.reclaim_capacity();
        Some(entry)
    }

    pub fn pop_max(&mut self) -> Option<ZEntry> {
        let entry = self.entries.pop()?;
        self.rebuild_fingerprints();
        self.reclaim_capacity();
        Some(entry)
    }

    pub fn iter(&self) -> std::slice::Iter<'_, ZEntry> {
        self.entries.iter()
    }

    pub fn iter_rev(&self) -> std::iter::Rev<std::slice::Iter<'_, ZEntry>> {
        self.entries.iter().rev()
    }

    pub fn contains(&self, member: &str) -> bool {
        self.entries.iter().any(|e| e.member.as_str() == member)
    }

    pub fn members(&self) -> impl Iterator<Item = &SmallStr> {
        self.entries.iter().map(|e| &e.member)
    }

    pub fn random_members(&self, count: usize, allow_dup: bool) -> Vec<&ZEntry> {
        use std::collections::HashSet as StdSet;
        if self.entries.is_empty() {
            return vec![];
        }
        let seed = self.entries.len().wrapping_mul(6364136223846793005).wrapping_add(1);
        let mut rng = seed;
        let mut next = || -> usize {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng % self.entries.len()
        };
        if allow_dup {
            (0..count).map(|_| &self.entries[next()]).collect()
        } else {
            let count = count.min(self.entries.len());
            let mut seen = StdSet::with_capacity(count);
            let mut result = Vec::with_capacity(count);
            while result.len() < count {
                let idx = next();
                if seen.insert(idx) {
                    result.push(&self.entries[idx]);
                }
            }
            result
        }
    }

    pub fn lex_range(&self, min: &str, max: &str, min_exclusive: bool, max_exclusive: bool) -> Vec<&ZEntry> {
        self.entries.iter().filter(|e| {
            let above_min = if min_exclusive { e.member.as_str() > min } else { e.member.as_str() >= min };
            let below_max = if max == "+" { true } else if max_exclusive { e.member.as_str() < max } else { e.member.as_str() <= max };
            above_min && below_max
        }).collect()
    }

    pub fn count_lex_range(&self, min: &str, max: &str, min_exclusive: bool, max_exclusive: bool) -> usize {
        self.lex_range(min, max, min_exclusive, max_exclusive).len()
    }

    pub fn remove_range_by_rank(&mut self, start: usize, end: usize) -> usize {
        let end = end.min(self.entries.len());
        if start >= end {
            return 0;
        }
        self.entries.drain(start..end);
        self.rebuild_fingerprints();
        self.reclaim_capacity();
        end - start
    }

    pub fn remove_range_by_score(&mut self, min: f64, max: f64) -> usize {
        let before = self.entries.len();
        self.entries.retain(|e| e.score < min || e.score > max);
        self.rebuild_fingerprints();
        self.reclaim_capacity();
        before - self.entries.len()
    }

    pub fn remove_lex_range(&mut self, min: &str, max: &str, min_exclusive: bool, max_exclusive: bool) -> usize {
        let before = self.entries.len();
        self.entries.retain(|e| {
            let above_min = if min_exclusive { e.member.as_str() > min } else { e.member.as_str() >= min };
            let below_max = if max == "+" { true } else if max_exclusive { e.member.as_str() < max } else { e.member.as_str() <= max };
            !(above_min && below_max)
        });
        self.rebuild_fingerprints();
        self.reclaim_capacity();
        before - self.entries.len()
    }

    pub fn incr(&mut self, member: &str, increment: f64) -> f64 {
        if let Some(pos) = self.entries.iter().position(|e| e.member.as_str() == member) {
            let old_score = self.entries[pos].score;
            let new_score = old_score + increment;
            self.entries.remove(pos);
            let insert_pos = self.find_insert_pos(new_score, member);
            self.entries.insert(insert_pos, ZEntry { score: new_score, member: SmallStr::new(member) });
            new_score
        } else {
            let insert_pos = self.find_insert_pos(increment, member);
            self.entries.insert(insert_pos, ZEntry { score: increment, member: SmallStr::new(member) });
            self.ensure_fingerprint_capacity();
            self.set_fingerprint(member_fingerprint(member));
            increment
        }
    }

    pub fn shrink_to_fit(&mut self) {
        self.entries.shrink_to_fit();
        self.rebuild_fingerprints();
    }

    #[inline]
    fn reclaim_capacity(&mut self) {
        // Hysteresis avoids a shrink/grow cycle while retaining large vectors
        // only when the set has genuinely dropped below half its capacity.
        if self.entries.capacity() > self.entries.len().saturating_mul(2).max(64) {
            self.entries.shrink_to_fit();
        }
    }

    fn rebuild_fingerprints(&mut self) {
        let words = (self.entries.len().saturating_mul(8).next_power_of_two().max(64) / 64).max(1);
        self.fingerprints.clear();
        self.fingerprints.resize(words, 0);
        for i in 0..self.entries.len() {
            let fingerprint = member_fingerprint(self.entries[i].member.as_str());
            self.set_fingerprint(fingerprint);
        }
    }

    #[inline]
    fn maybe_contains_fingerprint(&self, fingerprint: u64) -> bool {
        let bits = self.fingerprints.len() * 64;
        let a = fingerprint as usize & (bits - 1);
        let b = fingerprint.rotate_left(31) as usize & (bits - 1);
        (self.fingerprints[a / 64] & (1u64 << (a % 64))) != 0
            && (self.fingerprints[b / 64] & (1u64 << (b % 64))) != 0
    }

    #[inline]
    fn set_fingerprint(&mut self, fingerprint: u64) {
        let bits = self.fingerprints.len() * 64;
        let a = fingerprint as usize & (bits - 1);
        let b = fingerprint.rotate_left(31) as usize & (bits - 1);
        self.fingerprints[a / 64] |= 1u64 << (a % 64);
        self.fingerprints[b / 64] |= 1u64 << (b % 64);
    }

    fn ensure_fingerprint_capacity(&mut self) {
        if self.entries.len().saturating_mul(8) <= self.fingerprints.len() * 64 {
            return;
        }
        self.rebuild_fingerprints();
    }


    fn find_insert_pos(&self, score: f64, member: &str) -> usize {
        self.entries.partition_point(|e| {
            e.score < score || (e.score == score && e.member.as_str() < member)
        })
    }
}

#[inline]
fn member_fingerprint(member: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for &byte in member.as_bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

impl Default for ZSetData {
    fn default() -> Self { Self::new() }
}

#[derive(Clone, Debug)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(f64),
    String(SmallStr),
    Array(Vec<JsonValue>),
    Object(Vec<(SmallStr, JsonValue)>),
}

#[inline]
fn write_json_string(out: &mut String, value: &str) {
    out.push('"');
    let mut start = 0;
    for (idx, byte) in value.bytes().enumerate() {
        let escaped = match byte {
            b'\\' | b'"' => Some(byte),
            _ => None,
        };
        if let Some(byte) = escaped {
            out.push_str(&value[start..idx]);
            out.push('\\');
            out.push(byte as char);
            start = idx + 1;
        }
    }
    out.push_str(&value[start..]);
    out.push('"');
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
        let mut out = String::new();
        self.write_json(&mut out);
        out
    }

    fn write_json(&self, out: &mut String) {
        match self {
            Self::Null => out.push_str("null"),
            Self::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Self::Number(n) => {
                if *n == (*n as i64) as f64 {
                    use std::fmt::Write;
                    let _ = write!(out, "{}", *n as i64);
                } else {
                    use std::fmt::Write;
                    let _ = write!(out, "{}", n);
                }
            }
            Self::String(s) => write_json_string(out, s),
            Self::Array(arr) => {
                out.push('[');
                for (i, value) in arr.iter().enumerate() {
                    if i != 0 { out.push(','); }
                    value.write_json(out);
                }
                out.push(']');
            }
            Self::Object(obj) => {
                out.push('{');
                for (i, (key, value)) in obj.iter().enumerate() {
                    if i != 0 { out.push(','); }
                    write_json_string(out, key);
                    out.push(':');
                    value.write_json(out);
                }
                out.push('}');
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
                        obj.push((SmallStr::new(last), value));
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
    let start = 1;
    let mut i = start;
    let mut has_escape = false;
    while i < input.len() {
        match input[i] {
            b'"' => {
                if !has_escape {
                    let s = unsafe { std::str::from_utf8_unchecked(&input[start..i]) };
                    return Some((JsonValue::String(SmallStr::new(s)), &input[i + 1..]));
                }
                break;
            }
            b'\\' => {
                has_escape = true;
                i += 2;
            }
            _ => i += 1,
        }
    }
    if !has_escape {
        return None;
    }
    i = start;
    let mut s = String::with_capacity(32);
    while i < input.len() {
        match input[i] {
            b'"' => return Some((JsonValue::String(SmallStr::from_string(s)), &input[i + 1..])),
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

    pub fn as_string(&self) -> Option<&SmallStr> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_string_mut(&mut self) -> Option<&mut SmallStr> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_hash(&self) -> Option<&HashInner> {
        match self {
            Self::Hash(h) => Some(h),
            _ => None,
        }
    }

    pub fn as_hash_mut(&mut self) -> Option<&mut HashInner> {
        match self {
            Self::Hash(h) => Some(h),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&VecDeque<SmallStr>> {
        match self {
            Self::List(l) => Some(l.deque()),
            _ => None,
        }
    }

    pub fn as_list_mut(&mut self) -> Option<&mut VecDeque<SmallStr>> {
        match self {
            Self::List(l) => Some(l.deque_mut()),
            _ => None,
        }
    }

    pub fn as_set(&self) -> Option<&SetInner> {
        match self {
            Self::Set(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_set_mut(&mut self) -> Option<&mut SetInner> {
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

    pub fn as_json_str(&self) -> Option<&str> {
        match self {
            Self::Json(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn parse_json(&self) -> Option<JsonValue> {
        match self {
            Self::Json(s) => JsonValue::parse(s.as_str()),
            _ => None,
        }
    }

    pub fn set_json(&mut self, val: JsonValue) {
        *self = Self::Json(SmallStr::from_string(val.to_resp_string()));
    }

    pub fn set_json_str(&mut self, s: String) {
        *self = Self::Json(SmallStr::from_string(s));
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
            Self::List(l) => l.deque().iter().map(|s| s.len()).sum::<usize>() + l.deque().len() * 24,
            Self::Set(s) => s.iter().map(|m| m.len() + 24).sum(),
            Self::ZSet(z) => z.iter().map(|e| e.member.len() + 32).sum(),
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
            value: FyroDB::String(SmallStr::from_string(s)),
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
            value: FyroDB::String(SmallStr::from_string(s)),
            expires_ms,
        }
    }

    #[inline]
    pub fn hash(h: HashMap<String, String>) -> Self {
        let h = h.into_iter()
            .map(|(k, v)| (SmallStr::from_string(k), SmallStr::from_string(v)))
            .collect();
        Self {
            value: FyroDB::Hash(Box::new(HashInner::Full(Box::new(h)))),
            expires_ms: 0,
        }
    }

    #[inline]
    pub fn list(l: VecDeque<String>) -> Self {
        let l = l.into_iter().map(SmallStr::from_string).collect();
        Self {
            value: FyroDB::List(Box::new(ListInner::Full(Box::new(l)))),
            expires_ms: 0,
        }
    }

    #[inline]
    pub fn set(s: HashSet<String>) -> Self {
        Self {
            value: FyroDB::Set(Box::new(SetInner::from_strings(s))),
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
            value: FyroDB::Json(SmallStr::from_string(j.to_resp_string())),
            expires_ms: 0,
        }
    }

    #[inline]
    pub fn json_raw(s: String) -> Self {
        Self {
            value: FyroDB::Json(SmallStr::from_string(s)),
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
