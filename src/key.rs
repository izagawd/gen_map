use crate::config::{DefaultKeyConfig, KeyConfig};
use crate::key_layout::KeyLayout;
use crate::parity::Odd;
use core::cmp::Ordering;
use core::fmt;
use core::hash::{Hash, Hasher};

/// The layout of `K`'s keys.
type Layout<K> = <K as KeyConfig>::Layout;

/// The type a key of `K` stores its index and generation in.
type Repr<K> = <Layout<K> as KeyLayout<<K as KeyConfig>::Idx, <K as KeyConfig>::Gen>>::Repr;

/// A key to a value in a [`GenMap`](crate::GenMap), returned by `insert`.
/// The key config's [`Layout`](KeyConfig::Layout) says how the key stores
/// its index and generation.
pub struct Key<K: KeyConfig = DefaultKeyConfig> {
    repr: Repr<K>,
}

impl<K: KeyConfig> Key<K> {
    /// The index of the slot this key refers to.
    #[inline]
    pub fn idx(&self) -> K::Idx {
        <Layout<K> as KeyLayout<K::Idx, K::Gen>>::idx(self.repr)
    }

    /// The generation of the slot this key has.
    #[inline]
    pub fn generation(&self) -> Odd<K::Gen> {
        <Layout<K> as KeyLayout<K::Idx, K::Gen>>::generation(self.repr)
    }

    /// Returns `true` if the key's generation is the largest one its layout
    /// can hold. While the key is valid, removing its value wraps or retires
    /// the slot, as the map's config says, and
    /// [`detach`](crate::GenMap::detach) refuses it.
    #[inline]
    pub fn is_max_generation(&self) -> bool {
        self.generation() == <Layout<K> as KeyLayout<K::Idx, K::Gen>>::max_generation()
    }

    /// Builds a key from an index and a generation.
    ///
    /// # Safety
    ///
    /// Both parts must fit the layout, meaning `idx` is at most
    /// [`KeyLayout::max_idx`] and `generation` at most
    /// [`KeyLayout::max_generation`].
    #[inline]
    pub unsafe fn from_raw_parts(idx: K::Idx, generation: Odd<K::Gen>) -> Self {
        // SAFETY: the caller promises that both parts fit the layout.
        let repr =
            unsafe { <Layout<K> as KeyLayout<K::Idx, K::Gen>>::pack_unchecked(idx, generation) };
        Self { repr }
    }
}

// The traits below are written by hand because a derive would also require
// `K` to implement them, and a config is only a marker type.

impl<K: KeyConfig> Clone for Key<K> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<K: KeyConfig> Copy for Key<K> {}

impl<K: KeyConfig> PartialEq for Key<K> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.repr == other.repr
    }
}

impl<K: KeyConfig> Eq for Key<K> {}

impl<K: KeyConfig> PartialOrd for Key<K> {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Keys are ordered by index, then by generation, whatever the layout.
impl<K: KeyConfig> Ord for Key<K> {
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        self.idx()
            .cmp(&other.idx())
            .then(self.generation().cmp(&other.generation()))
    }
}

impl<K: KeyConfig> Hash for Key<K> {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.repr.hash(state);
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
