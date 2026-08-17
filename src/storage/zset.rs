use crate::storage::store::Store;
use crate::storage::value::{FyroDB, ScoreKey, StoreValue, ZSetData};

impl Store {
    #[allow(clippy::too_many_arguments)]
    pub fn zadd(
        &self,
        key: &str,
        members: &[(f64, String)],
        nx: bool,
        xx: bool,
        gt: bool,
        lt: bool,
        ch: bool,
    ) -> Result<usize, &'static str> {
        let result = self.data.update_with(key, |val| {
            if val.is_expired() {
                let mut z = ZSetData::new();
                let mut added = 0;
                for (score, member) in members {
                    if !xx {
                        z.insert(member.clone(), *score);
                        added += 1;
                    }
                }
                val.value = FyroDB::ZSet(Box::new(z));
                val.expires_ms = 0;
                return Ok(added);
            }
            match val.value.as_zset_mut() {
                Some(z) => {
                    let mut added = 0usize;
                    let mut changed = 0usize;
                    for (score, member) in members {
                        let existing = z.score(member);
                        match existing {
                            Some(old) => {
                                if nx {
                                    continue;
                                }
                                let should_update = if gt && lt {
                                    *score != old
                                } else if gt {
                                    *score > old
                                } else if lt {
                                    *score < old
                                } else {
                                    true
                                };
                                if should_update {
                                    z.insert(member.clone(), *score);
                                    changed += 1;
                                }
                            }
                            None => {
                                if xx {
                                    continue;
                                }
                                z.insert(member.clone(), *score);
                                added += 1;
                            }
                        }
                    }
                    Ok(if ch { added + changed } else { added })
                }
                None => Err("WRONGTYPE"),
            }
        });

        match result {
            Some(r) => r,
            None => {
                if xx {
                    return Ok(0);
                }
                let mut z = ZSetData::new();
                let mut added = 0;
                for (score, member) in members {
                    z.insert(member.clone(), *score);
                    added += 1;
                }
                self.data.insert(key.to_string(), StoreValue::zset(z));
                Ok(added)
            }
        }
    }

    pub fn zrem(&self, key: &str, members: &[&str]) -> Result<usize, &'static str> {
        let result = self.data.update_with(key, |val| {
            if val.is_expired() {
                return Ok(0);
            }
            match val.value.as_zset_mut() {
                Some(z) => Ok(members.iter().filter(|m| z.remove(m)).count()),
                None => Err("WRONGTYPE"),
            }
        });

        match result {
            Some(r) => r,
            None => Ok(0),
        }
    }

    pub fn zscore(&self, key: &str, member: &str) -> Result<Option<f64>, &'static str> {
        match self.data.get_ref(key) {
            None => Ok(None),
            Some(e) if e.is_expired() => Ok(None),
            Some(e) => match e.value.as_zset() {
                Some(z) => Ok(z.score(member)),
                None => Err("WRONGTYPE"),
            },
        }
    }

    pub fn zmscore(&self, key: &str, members: &[&str]) -> Result<Vec<Option<f64>>, &'static str> {
        match self.data.get_ref(key) {
            None => Ok(vec![None; members.len()]),
            Some(e) if e.is_expired() => Ok(vec![None; members.len()]),
            Some(e) => match e.value.as_zset() {
                Some(z) => Ok(members.iter().map(|m| z.score(m)).collect()),
                None => Err("WRONGTYPE"),
            },
        }
    }

    pub fn zrank(&self, key: &str, member: &str) -> Result<Option<usize>, &'static str> {
        match self.data.get_ref(key) {
            None => Ok(None),
            Some(e) if e.is_expired() => Ok(None),
            Some(e) => match e.value.as_zset() {
                Some(z) => Ok(z.rank(member)),
                None => Err("WRONGTYPE"),
            },
        }
    }

    pub fn zrevrank(&self, key: &str, member: &str) -> Result<Option<usize>, &'static str> {
        match self.data.get_ref(key) {
            None => Ok(None),
            Some(e) if e.is_expired() => Ok(None),
            Some(e) => match e.value.as_zset() {
                Some(z) => Ok(z.rev_rank(member)),
                None => Err("WRONGTYPE"),
            },
        }
    }

    pub fn zcard(&self, key: &str) -> Result<usize, &'static str> {
        match self.data.get_ref(key) {
            None => Ok(0),
            Some(e) if e.is_expired() => Ok(0),
            Some(e) => match e.value.as_zset() {
                Some(z) => Ok(z.len()),
                None => Err("WRONGTYPE"),
            },
        }
    }

    pub fn zcount(&self, key: &str, min: f64, max: f64) -> Result<usize, &'static str> {
        match self.data.get_ref(key) {
            None => Ok(0),
            Some(e) if e.is_expired() => Ok(0),
            Some(e) => match e.value.as_zset() {
                Some(z) => {
                    let count = z.dict.values().filter(|&&s| s >= min && s <= max).count();
                    Ok(count)
                }
                None => Err("WRONGTYPE"),
            },
        }
    }

    pub fn zincrby(&self, key: &str, increment: f64, member: &str) -> Result<f64, &'static str> {
        let result = self.data.update_with(key, |val| {
            if val.is_expired() {
                let mut z = ZSetData::new();
                z.insert(member.to_string(), increment);
                val.value = FyroDB::ZSet(Box::new(z));
                val.expires_ms = 0;
                return Ok(increment);
            }
            match val.value.as_zset_mut() {
                Some(z) => {
                    let new_score = z.score(member).unwrap_or(0.0) + increment;
                    z.insert(member.to_string(), new_score);
                    Ok(new_score)
                }
                None => Err("WRONGTYPE"),
            }
        });

        match result {
            Some(r) => r,
            None => {
                let mut z = ZSetData::new();
                z.insert(member.to_string(), increment);
                self.data.insert(key.to_string(), StoreValue::zset(z));
                Ok(increment)
            }
        }
    }

    pub fn zrange(
        &self,
        key: &str,
        start: i64,
        stop: i64,
        _withscores: bool,
    ) -> Result<Vec<(String, f64)>, &'static str> {
        match self.data.get_ref(key) {
            None => Ok(vec![]),
            Some(e) if e.is_expired() => Ok(vec![]),
            Some(e) => match e.value.as_zset() {
                Some(z) => {
                    let len = z.len() as i64;
                    let s = normalize_zset_index(start, len);
                    let e_idx = normalize_zset_index(stop, len);
                    if s > e_idx {
                        return Ok(vec![]);
                    }
                    let items: Vec<(String, f64)> = z
                        .tree
                        .keys()
                        .skip(s)
                        .take(e_idx - s + 1)
                        .map(|sk| (sk.member.clone(), sk.score()))
                        .collect();
                    Ok(items)
                }
                None => Err("WRONGTYPE"),
            },
        }
    }

    pub fn zrevrange(
        &self,
        key: &str,
        start: i64,
        stop: i64,
    ) -> Result<Vec<(String, f64)>, &'static str> {
        match self.data.get_ref(key) {
            None => Ok(vec![]),
            Some(e) if e.is_expired() => Ok(vec![]),
            Some(e) => match e.value.as_zset() {
                Some(z) => {
                    let len = z.len() as i64;
                    let s = normalize_zset_index(start, len);
                    let e_idx = normalize_zset_index(stop, len);
                    if s > e_idx {
                        return Ok(vec![]);
                    }
                    let items: Vec<(String, f64)> = z
                        .tree
                        .keys()
                        .rev()
                        .skip(s)
                        .take(e_idx - s + 1)
                        .map(|sk| (sk.member.clone(), sk.score()))
                        .collect();
                    Ok(items)
                }
                None => Err("WRONGTYPE"),
            },
        }
    }

    pub fn zrangebyscore(
        &self,
        key: &str,
        min: f64,
        max: f64,
        offset: usize,
        count: usize,
    ) -> Result<Vec<(String, f64)>, &'static str> {
        match self.data.get_ref(key) {
            None => Ok(vec![]),
            Some(e) if e.is_expired() => Ok(vec![]),
            Some(e) => match e.value.as_zset() {
                Some(z) => {
                    let low = ScoreKey::new(min, String::new());
                    let items: Vec<(String, f64)> = z
                        .tree
                        .range(&low..)
                        .map(|(sk, _)| (sk.member.clone(), sk.score()))
                        .filter(|(_, s)| *s <= max)
                        .skip(offset)
                        .take(if count == 0 { usize::MAX } else { count })
                        .collect();
                    Ok(items)
                }
                None => Err("WRONGTYPE"),
            },
        }
    }

    pub fn zrevrangebyscore(
        &self,
        key: &str,
        max: f64,
        min: f64,
        offset: usize,
        count: usize,
    ) -> Result<Vec<(String, f64)>, &'static str> {
        match self.data.get_ref(key) {
            None => Ok(vec![]),
            Some(e) if e.is_expired() => Ok(vec![]),
            Some(e) => match e.value.as_zset() {
                Some(z) => {
                    let items: Vec<(String, f64)> = z
                        .tree
                        .keys()
                        .rev()
                        .map(|sk| (sk.member.clone(), sk.score()))
                        .filter(|(_, s)| *s >= min && *s <= max)
                        .skip(offset)
                        .take(if count == 0 { usize::MAX } else { count })
                        .collect();
                    Ok(items)
                }
                None => Err("WRONGTYPE"),
            },
        }
    }

    pub fn zpopmin(&self, key: &str, count: usize) -> Result<Vec<(String, f64)>, &'static str> {
        let result = self.data.update_with(key, |val| {
            if val.is_expired() {
                return Ok(vec![]);
            }
            match val.value.as_zset_mut() {
                Some(z) => {
                    let n = count.min(z.len());
                    let mut out = Vec::with_capacity(n);
                    for _ in 0..n {
                        let first = z.tree.keys().next().cloned();
                        if let Some(sk) = first {
                            let score = sk.score();
                            z.tree.remove(&sk);
                            z.dict.remove(&sk.member);
                            out.push((sk.member, score));
                        }
                    }
                    Ok(out)
                }
                None => Err("WRONGTYPE"),
            }
        });

        match result {
            Some(r) => r,
            None => Ok(vec![]),
        }
    }

    pub fn zpopmax(&self, key: &str, count: usize) -> Result<Vec<(String, f64)>, &'static str> {
        let result = self.data.update_with(key, |val| {
            if val.is_expired() {
                return Ok(vec![]);
            }
            match val.value.as_zset_mut() {
                Some(z) => {
                    let n = count.min(z.len());
                    let mut out = Vec::with_capacity(n);
                    for _ in 0..n {
                        let last = z.tree.keys().next_back().cloned();
                        if let Some(sk) = last {
                            let score = sk.score();
                            z.tree.remove(&sk);
                            z.dict.remove(&sk.member);
                            out.push((sk.member, score));
                        }
                    }
                    Ok(out)
                }
                None => Err("WRONGTYPE"),
            }
        });

        match result {
            Some(r) => r,
            None => Ok(vec![]),
        }
    }

    pub fn zunionstore(
        &self,
        dst: &str,
        keys: &[&str],
        weights: &[f64],
        aggregate: ZAggregate,
    ) -> Result<usize, &'static str> {
        let mut result = ZSetData::new();
        for (i, &k) in keys.iter().enumerate() {
            let weight = weights.get(i).copied().unwrap_or(1.0);
            match self.data.get_ref(k) {
                None => {}
                Some(e) if e.is_expired() => {}
                Some(e) => match e.value.as_zset() {
                    Some(z) => {
                        for (member, &score) in z.dict.iter() {
                            let weighted = score * weight;
                            let new_score = match result.score(member) {
                                Some(existing) => aggregate.apply(existing, weighted),
                                None => weighted,
                            };
                            result.insert(member.clone(), new_score);
                        }
                    }
                    None => return Err("WRONGTYPE"),
                },
            }
        }
        let count = result.len();
        self.data.insert(dst.to_string(), StoreValue::zset(result));
        Ok(count)
    }

    pub fn zinterstore(
        &self,
        dst: &str,
        keys: &[&str],
        weights: &[f64],
        aggregate: ZAggregate,
    ) -> Result<usize, &'static str> {
        if keys.is_empty() {
            self.data.insert(dst.to_string(), StoreValue::zset(ZSetData::new()));
            return Ok(0);
        }
        let first = match self.data.get_ref(keys[0]) {
            None => {
                self.data.insert(dst.to_string(), StoreValue::zset(ZSetData::new()));
                return Ok(0);
            }
            Some(e) if e.is_expired() => {
                self.data.insert(dst.to_string(), StoreValue::zset(ZSetData::new()));
                return Ok(0);
            }
            Some(e) => match e.value.as_zset() {
                Some(z) => z.clone(),
                None => return Err("WRONGTYPE"),
            },
        };

        let w0 = weights.first().copied().unwrap_or(1.0);
        let mut result = ZSetData::new();
        for (member, &score) in first.dict.iter() {
            result.insert(member.clone(), score * w0);
        }

        for (i, &k) in keys[1..].iter().enumerate() {
            let weight = weights.get(i + 1).copied().unwrap_or(1.0);
            let other = match self.data.get_ref(k) {
                None => {
                    self.data.insert(dst.to_string(), StoreValue::zset(ZSetData::new()));
                    return Ok(0);
                }
                Some(e) if e.is_expired() => {
                    self.data.insert(dst.to_string(), StoreValue::zset(ZSetData::new()));
                    return Ok(0);
                }
                Some(e) => match e.value.as_zset() {
                    Some(z) => z.clone(),
                    None => return Err("WRONGTYPE"),
                },
            };

            let mut next = ZSetData::new();
            for (member, &our_score) in result.dict.iter() {
                if let Some(&other_score) = other.dict.get(member) {
                    let new_score = aggregate.apply(our_score, other_score * weight);
                    next.insert(member.clone(), new_score);
                }
            }
            result = next;
        }

        let count = result.len();
        self.data.insert(dst.to_string(), StoreValue::zset(result));
        Ok(count)
    }

    pub fn zrandmember(&self, key: &str, count: i64) -> Result<Vec<(String, f64)>, &'static str> {
        match self.data.get_ref(key) {
            None => Ok(vec![]),
            Some(e) if e.is_expired() => Ok(vec![]),
            Some(e) => match e.value.as_zset() {
                Some(z) => {
                    if z.is_empty() {
                        return Ok(vec![]);
                    }
                    let members: Vec<(&String, &f64)> = z.dict.iter().collect();
                    if count >= 0 {
                        let n = (count as usize).min(members.len());
                        Ok(members.iter().take(n).map(|(m, s)| ((*m).clone(), **s)).collect())
                    } else {
                        let n = (-count) as usize;
                        let mut out = Vec::with_capacity(n);
                        let seed = crate::storage::value::now_ms();
                        for i in 0..n {
                            let idx = ((seed.wrapping_add(i as u64)).wrapping_mul(0x9e3779b97f4a7c15))
                                as usize
                                % members.len();
                            out.push((members[idx].0.clone(), *members[idx].1));
                        }
                        Ok(out)
                    }
                }
                None => Err("WRONGTYPE"),
            },
        }
    }
}

#[derive(Clone, Copy)]
pub enum ZAggregate {
    Sum,
    Min,
    Max,
}

impl ZAggregate {
    #[inline]
    pub fn apply(self, a: f64, b: f64) -> f64 {
        match self {
            Self::Sum => a + b,
            Self::Min => a.min(b),
            Self::Max => a.max(b),
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        if s.eq_ignore_ascii_case("SUM") {
            Some(Self::Sum)
        } else if s.eq_ignore_ascii_case("MIN") {
            Some(Self::Min)
        } else if s.eq_ignore_ascii_case("MAX") {
            Some(Self::Max)
        } else {
            None
        }
    }
}

impl Store {
    pub fn zlexcount(&self, key: &str, min: &str, max: &str) -> Result<usize, &'static str> {
        match self.data.get_ref(key) {
            None => Ok(0),
            Some(e) if e.is_expired() => Ok(0),
            Some(e) => match e.value.as_zset() {
                Some(z) => {
                    let count = z.dict.keys().filter(|m| lex_in_range(m, min, max)).count();
                    Ok(count)
                }
                None => Err("WRONGTYPE"),
            },
        }
    }

    pub fn zrangebylex(
        &self,
        key: &str,
        min: &str,
        max: &str,
        offset: usize,
        count: usize,
    ) -> Result<Vec<String>, &'static str> {
        match self.data.get_ref(key) {
            None => Ok(vec![]),
            Some(e) if e.is_expired() => Ok(vec![]),
            Some(e) => match e.value.as_zset() {
                Some(z) => {
                    let members: Vec<String> = z
                        .tree
                        .keys()
                        .map(|sk| sk.member.clone())
                        .filter(|m| lex_in_range(m, min, max))
                        .skip(offset)
                        .take(if count == 0 { usize::MAX } else { count })
                        .collect();
                    Ok(members)
                }
                None => Err("WRONGTYPE"),
            },
        }
    }

    pub fn zdiff(&self, keys: &[&str]) -> Result<Vec<(String, f64)>, &'static str> {
        if keys.is_empty() {
            return Ok(vec![]);
        }
        let first = match self.data.get_ref(keys[0]) {
            None => return Ok(vec![]),
            Some(e) if e.is_expired() => return Ok(vec![]),
            Some(e) => match e.value.as_zset() {
                Some(z) => z.clone(),
                None => return Err("WRONGTYPE"),
            },
        };

        let mut result = first;
        for &k in &keys[1..] {
            match self.data.get_ref(k) {
                None => {}
                Some(e) if e.is_expired() => {}
                Some(e) => match e.value.as_zset() {
                    Some(z) => {
                        let to_remove: Vec<String> = result
                            .dict
                            .keys()
                            .filter(|m| z.dict.contains_key(*m))
                            .cloned()
                            .collect();
                        for m in to_remove {
                            result.remove(&m);
                        }
                    }
                    None => return Err("WRONGTYPE"),
                },
            }
        }

        let items: Vec<(String, f64)> = result
            .tree
            .keys()
            .map(|sk| (sk.member.clone(), sk.score()))
            .collect();
        Ok(items)
    }

    pub fn zdiffstore(&self, dst: &str, keys: &[&str]) -> Result<usize, &'static str> {
        let items = self.zdiff(keys)?;
        let mut z = ZSetData::new();
        for (member, score) in &items {
            z.insert(member.clone(), *score);
        }
        let count = z.len();
        self.data.insert(dst.to_string(), StoreValue::zset(z));
        Ok(count)
    }

    pub fn zunion(
        &self,
        keys: &[&str],
        weights: &[f64],
        aggregate: ZAggregate,
    ) -> Result<Vec<(String, f64)>, &'static str> {
        let mut result = ZSetData::new();
        for (i, &k) in keys.iter().enumerate() {
            let weight = weights.get(i).copied().unwrap_or(1.0);
            match self.data.get_ref(k) {
                None => {}
                Some(e) if e.is_expired() => {}
                Some(e) => match e.value.as_zset() {
                    Some(z) => {
                        for (member, &score) in z.dict.iter() {
                            let weighted = score * weight;
                            let new_score = match result.score(member) {
                                Some(existing) => aggregate.apply(existing, weighted),
                                None => weighted,
                            };
                            result.insert(member.clone(), new_score);
                        }
                    }
                    None => return Err("WRONGTYPE"),
                },
            }
        }
        let items: Vec<(String, f64)> = result
            .tree
            .keys()
            .map(|sk| (sk.member.clone(), sk.score()))
            .collect();
        Ok(items)
    }

    pub fn zinter(
        &self,
        keys: &[&str],
        weights: &[f64],
        aggregate: ZAggregate,
    ) -> Result<Vec<(String, f64)>, &'static str> {
        if keys.is_empty() {
            return Ok(vec![]);
        }
        let first = match self.data.get_ref(keys[0]) {
            None => return Ok(vec![]),
            Some(e) if e.is_expired() => return Ok(vec![]),
            Some(e) => match e.value.as_zset() {
                Some(z) => z.clone(),
                None => return Err("WRONGTYPE"),
            },
        };

        let w0 = weights.first().copied().unwrap_or(1.0);
        let mut result = ZSetData::new();
        for (member, &score) in first.dict.iter() {
            result.insert(member.clone(), score * w0);
        }

        for (i, &k) in keys[1..].iter().enumerate() {
            let weight = weights.get(i + 1).copied().unwrap_or(1.0);
            let other = match self.data.get_ref(k) {
                None => return Ok(vec![]),
                Some(e) if e.is_expired() => return Ok(vec![]),
                Some(e) => match e.value.as_zset() {
                    Some(z) => z.clone(),
                    None => return Err("WRONGTYPE"),
                },
            };

            let mut next = ZSetData::new();
            for (member, &our_score) in result.dict.iter() {
                if let Some(&other_score) = other.dict.get(member) {
                    let new_score = aggregate.apply(our_score, other_score * weight);
                    next.insert(member.clone(), new_score);
                }
            }
            result = next;
        }

        let items: Vec<(String, f64)> = result
            .tree
            .keys()
            .map(|sk| (sk.member.clone(), sk.score()))
            .collect();
        Ok(items)
    }
}

fn lex_in_range(member: &str, min: &str, max: &str) -> bool {
    let min_ok = if min == "-" {
        true
    } else if let Some(m) = min.strip_prefix('[') {
        member >= m
    } else if let Some(m) = min.strip_prefix('(') {
        member > m
    } else {
        member >= min
    };

    let max_ok = if max == "+" {
        true
    } else if let Some(m) = max.strip_prefix('[') {
        member <= m
    } else if let Some(m) = max.strip_prefix('(') {
        member < m
    } else {
        member <= max
    };

    min_ok && max_ok
}

#[inline]
fn normalize_zset_index(index: i64, len: i64) -> usize {
    if index < 0 {
        (len + index).max(0) as usize
    } else {
        index.min(len.saturating_sub(1)) as usize
    }
}
