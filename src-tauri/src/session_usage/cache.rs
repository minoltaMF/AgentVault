//! Process-local parsed results only. All filesystem checks happen outside this lock.
use super::{ParsedUsage, UsageSource};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::SystemTime,
};
pub(super) type Fingerprint = (u64, SystemTime, String);
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Key(String, PathBuf, String);
impl Key {
    pub(super) fn new(source: &UsageSource, root: &Path) -> Self {
        Self(
            source.provider.clone(),
            root.to_path_buf(),
            source.rollout_path.clone(),
        )
    }
    fn weight(&self) -> usize {
        self.0.len() + self.1.as_os_str().len() * 4 + self.2.len() + 128
    }
}
struct Entry {
    fingerprint: Fingerprint,
    parsed: ParsedUsage,
    weight: usize,
    used: u64,
}
pub(super) struct UsageCache {
    entries: BTreeMap<Key, Entry>,
    order: BTreeMap<u64, Key>,
    bytes: usize,
    clock: u64,
    max_entries: usize,
    max_bytes: usize,
}
static CACHE: OnceLock<Mutex<UsageCache>> = OnceLock::new();
pub(super) fn shared() -> &'static Mutex<UsageCache> {
    CACHE.get_or_init(|| Mutex::new(UsageCache::new(10_000, 16 * 1024 * 1024)))
}
impl UsageCache {
    pub(super) fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            order: BTreeMap::new(),
            bytes: 0,
            clock: 0,
            max_entries,
            max_bytes,
        }
    }
    pub(super) fn get(
        &mut self,
        key: &Key,
        fingerprint: &Fingerprint,
        force: bool,
    ) -> Option<ParsedUsage> {
        // A mismatch or explicit refresh invalidates the old result even if parsing fails later.
        if force
            || self
                .entries
                .get(key)
                .is_some_and(|entry| entry.fingerprint != *fingerprint)
        {
            self.remove(key);
            return None;
        }
        self.tick();
        self.entries.get_mut(key).map(|entry| {
            self.order.remove(&entry.used);
            entry.used = self.clock;
            self.order.insert(self.clock, key.clone());
            entry.parsed.clone()
        })
    }
    fn tick(&mut self) {
        // Clearing this optimization on an exhausted clock preserves ordering and bounds.
        if self.clock == u64::MAX {
            self.entries.clear();
            self.order.clear();
            self.bytes = 0;
            self.clock = 0;
        }
        self.clock += 1;
    }
    fn remove(&mut self, key: &Key) {
        if let Some(old) = self.entries.remove(key) {
            self.bytes -= old.weight;
            self.order.remove(&old.used);
        }
    }
    pub(super) fn insert(&mut self, key: Key, fingerprint: Fingerprint, parsed: ParsedUsage) {
        self.remove(&key);
        let weight = key.weight() * 2
            + fingerprint.2.len()
            + 128
            + parsed
                .models
                .iter()
                .map(|m| m.model.len() + 96)
                .sum::<usize>()
            + parsed.warnings.iter().map(|w| w.len() + 32).sum::<usize>();
        if self.max_entries == 0 || weight > self.max_bytes {
            return;
        }
        while self.entries.len() >= self.max_entries
            || self.bytes.saturating_add(weight) > self.max_bytes
        {
            let Some(old) = self.order.first_key_value().map(|(_, key)| key.clone()) else {
                break;
            };
            self.remove(&old);
        }
        self.tick();
        self.bytes += weight;
        self.order.insert(self.clock, key.clone());
        self.entries.insert(
            key,
            Entry {
                fingerprint,
                parsed,
                weight,
                used: self.clock,
            },
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn key(value: &str) -> Key {
        Key("claude".into(), PathBuf::from("root"), value.into())
    }
    fn fp() -> Fingerprint {
        (1, SystemTime::UNIX_EPOCH, "id".into())
    }
    #[test]
    fn bounded_lru_and_oversized_entries() {
        let mut cache = UsageCache::new(2, 10000);
        for name in ["a", "b"] {
            cache.insert(key(name), fp(), ParsedUsage::default());
        }
        assert!(cache.get(&key("a"), &fp(), false).is_some());
        cache.insert(key("c"), fp(), ParsedUsage::default());
        assert!(cache.get(&key("b"), &fp(), false).is_none());
        assert!(cache.get(&key("a"), &fp(), false).is_some());
        let mut tiny = UsageCache::new(100, 1);
        tiny.insert(key("a"), fp(), ParsedUsage::default());
        assert!(tiny.entries.is_empty());
        let weight = cache.entries[&key("a")].weight;
        let mut budget = UsageCache::new(100, weight + 1);
        budget.insert(key("a"), fp(), ParsedUsage::default());
        budget.insert(key("b"), fp(), ParsedUsage::default());
        assert_eq!(budget.entries.len(), 1);
        assert!(budget.bytes <= budget.max_bytes);
    }
    #[test]
    fn roots_providers_and_force_refresh_are_isolated() {
        let mut cache = UsageCache::new(10, 10000);
        cache.insert(key("a"), fp(), ParsedUsage::default());
        let mut other = key("a");
        other.1 = PathBuf::from("other");
        assert!(cache.get(&other, &fp(), false).is_none());
        other = key("a");
        other.0 = "codex".into();
        assert!(cache.get(&other, &fp(), false).is_none());
        assert!(cache.get(&key("a"), &fp(), true).is_none());
        assert!(cache.entries.is_empty());
    }
}
