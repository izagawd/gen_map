use crate::config::{DefaultKeyConfig, KeyConfig};
use crate::parity::Odd;
use core::cmp::Ordering;
use core::fmt;

/// A key to a value in a [`GenMap`](crate::GenMap) or a
/// [`DenseGenMap`](crate::DenseGenMap), returned by `insert`.
/// The key config `K` decides how the key stores its index and generation.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key<K = DefaultKeyConfig> {
    repr: K,
}

impl<K: KeyConfig> Key<K> {
    /// The index of the slot this key refers to.
    #[inline]
    pub fn idx(&self) -> K::Idx {
        self.repr.idx()
    }

    /// The generation the key's slot had when the key was handed out. The
    /// key matches the slot only while the slot still has this generation.
    #[inline]
    pub fn generation(&self) -> Odd<K::Gen> {
        self.repr.generation()
    }

    /// Returns `true` if the key's generation is the largest one its key config
    /// can hold. The slot's generation cannot go up past that one, so removing
    /// the key's value either wraps the slot's generation back to zero or
    /// retires the slot, depending on the `WRAP_ON_OVERFLOW` of the map's
    /// config, which [`GenMapConfig`](crate::GenMapConfig) and
    /// [`DenseGenMapConfig`](crate::DenseGenMapConfig) both have.
    #[inline]
    pub fn is_max_generation(&self) -> bool {
        self.generation() == K::max_generation()
    }

    /// Builds a key from a value of its key config, which
    /// [`KeyConfig::pack`] makes from an index and a generation.
    #[inline]
    pub fn from_repr(repr: K) -> Self {
        Self { repr }
    }

    /// The value of the key config that holds this key's index and
    /// generation.
    #[inline]
    pub fn repr(&self) -> K {
        self.repr
    }
}

impl<K: KeyConfig> PartialOrd for Key<K> {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Keys are ordered by index, then by generation, whatever the key config.
impl<K: KeyConfig> Ord for Key<K> {
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        self.idx()
            .cmp(&other.idx())
            .then(self.generation().cmp(&other.generation()))
    }
}

impl<K: KeyConfig> fmt::Debug for Key<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Key")
            .field("idx", &self.idx())
            .field("generation", &self.generation().get())
            .finish()
    }
}
