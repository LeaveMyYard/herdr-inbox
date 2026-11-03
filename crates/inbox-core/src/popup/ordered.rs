//! A small map that keeps insertion order, as Python's dict does.

/// Insertion-ordered pairs (Python's dict): the popup's placeholders and
/// gone tabs, keyed by entry or tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ordered<K, V>(Vec<(K, V)>);

impl<K, V> Default for Ordered<K, V> {
    fn default() -> Self {
        Ordered(Vec::new())
    }
}

impl<K: PartialEq, V> Ordered<K, V> {
    /// The value under `k`.
    pub fn get(&self, k: &K) -> Option<&V> {
        self.0.iter().find(|(x, _)| x == k).map(|(_, v)| v)
    }

    /// `k` has a value.
    pub fn contains(&self, k: &K) -> bool {
        self.get(k).is_some()
    }

    /// Set `k` (a known key keeps its place, as in a dict).
    pub fn insert(&mut self, k: K, v: V) {
        match self.0.iter_mut().find(|(x, _)| *x == k) {
            Some(slot) => slot.1 = v,
            None => self.0.push((k, v)),
        }
    }

    /// Keep only the pairs `keep` says.
    pub fn retain(&mut self, mut keep: impl FnMut(&K) -> bool) {
        self.0.retain(|(k, _)| keep(k));
    }

    /// Every pair, in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.0.iter().map(|(k, v)| (k, v))
    }

    /// Nothing in it.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
