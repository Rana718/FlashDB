use super::small_str::SmallStr;

#[derive(Clone)]
pub struct ZSetData {
    entries: Vec<ZEntry>,
}

#[derive(Clone)]
pub struct ZEntry {
    pub score: f64,
    pub member: SmallStr,
}

impl ZSetData {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self {
            entries: Vec::with_capacity(cap),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// O(log n) find_insert_pos + O(n) memmove for insert.
    /// For ascending score pattern, append is O(1).
    pub fn insert(&mut self, score: f64, member: &str) -> bool {
        if let Some(last) = self.entries.last()
            && last.member.as_str() == member
        {
            if (last.score - score).abs() > f64::EPSILON {
                self.entries.pop();
                let insert_pos = self.find_insert_pos(score, member);
                self.entries.insert(insert_pos, ZEntry { score, member: SmallStr::new(member) });
            }
            return false;
        }
        if let Some(pos) = self.entries.iter().position(|e| e.member.as_str() == member) {
            let old_score = self.entries[pos].score;
            if (old_score - score).abs() > f64::EPSILON {
                self.entries.remove(pos);
                let insert_pos = self.find_insert_pos(score, member);
                self.entries.insert(insert_pos, ZEntry { score, member: SmallStr::new(member) });
            }
            return false;
        }
        if self.entries.last().is_none_or(|last| {
            last.score < score || (last.score == score && last.member.as_str() <= member)
        }) {
            self.entries.push(ZEntry { score, member: SmallStr::new(member) });
        } else {
            let insert_pos = self.find_insert_pos(score, member);
            self.entries.insert(insert_pos, ZEntry { score, member: SmallStr::new(member) });
        }
        true
    }

    pub fn remove(&mut self, member: &str) -> Option<f64> {
        let pos = self.entries.iter().position(|e| e.member.as_str() == member)?;
        let score = self.entries[pos].score;
        self.entries.remove(pos);
        self.reclaim_capacity();
        Some(score)
    }

    #[inline]
    pub fn get_score(&self, member: &str) -> Option<f64> {
        self.entries
            .iter()
            .find(|e| e.member.as_str() == member)
            .map(|e| e.score)
    }

    pub fn rank(&self, member: &str) -> Option<usize> {
        self.entries
            .iter()
            .position(|e| e.member.as_str() == member)
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

    /// O(log n) binary search on sorted entries.
    pub fn range_by_score(&self, min: f64, max: f64) -> &[ZEntry] {
        let start = self.entries.partition_point(|e| e.score < min);
        let end = self.entries.partition_point(|e| e.score <= max);
        &self.entries[start..end]
    }

    /// O(log n) binary search + reverse iteration.
    pub fn rev_range_by_score(&self, min: f64, max: f64) -> Vec<&ZEntry> {
        let start = self.entries.partition_point(|e| e.score < min);
        let end = self.entries.partition_point(|e| e.score <= max);
        self.entries[start..end].iter().rev().collect()
    }

    /// O(log n) count.
    pub fn count_in_score_range(&self, min: f64, max: f64) -> usize {
        let start = self.entries.partition_point(|e| e.score < min);
        let end = self.entries.partition_point(|e| e.score <= max);
        end - start
    }

    pub fn pop_min(&mut self) -> Option<ZEntry> {
        if self.entries.is_empty() {
            return None;
        }
        let entry = self.entries.remove(0);
        self.reclaim_capacity();
        Some(entry)
    }

    pub fn pop_max(&mut self) -> Option<ZEntry> {
        let entry = self.entries.pop()?;
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
        let seed = self
            .entries
            .len()
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1);
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

    pub fn lex_range(
        &self,
        min: &str,
        max: &str,
        min_exclusive: bool,
        max_exclusive: bool,
    ) -> Vec<&ZEntry> {
        self.entries
            .iter()
            .filter(|e| {
                let above_min = if min_exclusive {
                    e.member.as_str() > min
                } else {
                    e.member.as_str() >= min
                };
                let below_max = if max == "+" {
                    true
                } else if max_exclusive {
                    e.member.as_str() < max
                } else {
                    e.member.as_str() <= max
                };
                above_min && below_max
            })
            .collect()
    }

    pub fn count_lex_range(
        &self,
        min: &str,
        max: &str,
        min_exclusive: bool,
        max_exclusive: bool,
    ) -> usize {
        self.lex_range(min, max, min_exclusive, max_exclusive).len()
    }

    pub fn remove_range_by_rank(&mut self, start: usize, end: usize) -> usize {
        let end = end.min(self.entries.len());
        if start >= end {
            return 0;
        }
        self.entries.drain(start..end);
        self.reclaim_capacity();
        end - start
    }

    pub fn remove_range_by_score(&mut self, min: f64, max: f64) -> usize {
        let start = self.entries.partition_point(|e| e.score < min);
        let end = self.entries.partition_point(|e| e.score <= max);
        if start >= end {
            return 0;
        }
        let count = end - start;
        self.entries.drain(start..end);
        self.reclaim_capacity();
        count
    }

    pub fn remove_lex_range(
        &mut self,
        min: &str,
        max: &str,
        min_exclusive: bool,
        max_exclusive: bool,
    ) -> usize {
        let before = self.entries.len();
        self.entries.retain(|e| {
            let above_min = if min_exclusive {
                e.member.as_str() > min
            } else {
                e.member.as_str() >= min
            };
            let below_max = if max == "+" {
                true
            } else if max_exclusive {
                e.member.as_str() < max
            } else {
                e.member.as_str() <= max
            };
            !(above_min && below_max)
        });
        let removed = before - self.entries.len();
        if removed > 0 {
            self.reclaim_capacity();
        }
        removed
    }

    pub fn incr(&mut self, member: &str, increment: f64) -> f64 {
        if let Some(pos) = self.entries.iter().position(|e| e.member.as_str() == member) {
            let new_score = self.entries[pos].score + increment;
            let stays = (pos == 0
                || self.entries[pos - 1].score < new_score
                || (self.entries[pos - 1].score == new_score
                    && self.entries[pos - 1].member.as_str() <= member))
                && (pos >= self.entries.len() - 1
                    || self.entries[pos + 1].score > new_score
                    || (self.entries[pos + 1].score == new_score
                        && self.entries[pos + 1].member.as_str() >= member));
            if stays {
                self.entries[pos].score = new_score;
            } else {
                self.entries.remove(pos);
                let insert_pos = self.find_insert_pos(new_score, member);
                self.entries.insert(insert_pos, ZEntry { score: new_score, member: SmallStr::new(member) });
            }
            new_score
        } else {
            self.insert(increment, member);
            increment
        }
    }

    pub fn shrink_to_fit(&mut self) {
        self.entries.shrink_to_fit();
    }

    #[inline]
    fn reclaim_capacity(&mut self) {
        if self.entries.capacity() > self.entries.len().saturating_mul(2).max(64) {
            self.entries.shrink_to_fit();
        }
    }

    fn find_insert_pos(&self, score: f64, member: &str) -> usize {
        self.entries.partition_point(|e| {
            e.score < score || (e.score == score && e.member.as_str() < member)
        })
    }
}

impl Default for ZSetData {
    fn default() -> Self {
        Self::new()
    }
}
