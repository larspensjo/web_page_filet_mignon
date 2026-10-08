//! Shared in-memory storage for paid results. Entries have no size or age limit.
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, hash::Hash, ops::Deref};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ResultStore<K: Eq + Hash, E>(HashMap<K, E>);
impl<K: Eq + Hash, E> Default for ResultStore<K, E> {
    fn default() -> Self {
        Self(HashMap::new())
    }
}
impl<K: Eq + Hash, E> ResultStore<K, E> {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn insert(&mut self, key: K, entry: E) {
        self.0.insert(key, entry);
    }
    pub fn clear(&mut self) {
        self.0.clear();
    }
}
impl<K: Eq + Hash, E> Deref for ResultStore<K, E> {
    type Target = HashMap<K, E>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl<K: Eq + Hash, E> From<HashMap<K, E>> for ResultStore<K, E> {
    fn from(entries: HashMap<K, E>) -> Self {
        Self(entries)
    }
}
impl<'a, K: Eq + Hash, E> IntoIterator for &'a ResultStore<K, E> {
    type Item = (&'a K, &'a E);
    type IntoIter = std::collections::hash_map::Iter<'a, K, E>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}
/// Only newly inserted records cross the reducer's persistence boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SavedResult {
    Triage(crate::TriageCacheKey, crate::TriageCacheEntry),
    Summary(crate::SummaryCacheKey, crate::SummaryCacheEntry),
    SignalCandidate(
        crate::SignalCandidateCacheKey,
        crate::SignalCandidateCacheEntry,
    ),
}
