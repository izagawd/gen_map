use crate::config::{Config, DefaultConfig};
use crate::key_piece::KeyPiece;
use core::cmp::Ordering;
use core::fmt;
use core::hash::{Hash, Hasher};

/// A key to a value in a [`GenMap`](crate::GenMap), returned by `insert`.
pub struct Key<C: Config = DefaultConfig> {
    pub(crate) idx: C::Idx,
    pub(crate) generation: <C::Gen as KeyPiece>::NonZero,
}

impl<C: Config> Key<C> {
    /// The index of the slot this key refers to.
    #[inline]
    pub fn idx(&self) -> C::Idx {
        self.idx
    }

    /// The generation of the slot this key refers to at the time this key was handed out.
    #[inline]
    pub fn generation(&self) -> C::Gen {
        C::Gen::from_non_zero(self.generation)
    }

    /// Builds a key from an index and a generation
    ///
    /// # Safety
    ///
    /// `generation` must be odd. The map only hands out odd generations, and
    /// an even one is what a vacant or detached slot holds. A key that
    /// carries an even generation can match such a slot, which is undefined behavior.
    #[inline]
    pub unsafe fn from_raw_parts(idx: C::Idx, generation: <C::Gen as KeyPiece>::NonZero) -> Self {
        debug_assert!(<C::Gen as KeyPiece>::from_non_zero(generation).is_odd());
        // SAFETY: the caller promises an odd generation, and zero is even.
        Self { idx, generation }
    }
}

// The traits below are written by hand because a derive would also demand
// them from `C`, and a config is only a marker type.

impl<C: Config> Clone for Key<C> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<C: Config> Copy for Key<C> {}

impl<C: Config> PartialEq for Key<C> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.idx == other.idx && self.generation == other.generation
    }
}

impl<C: Config> Eq for Key<C> {}

impl<C: Config> PartialOrd for Key<C> {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Keys are ordered by index, then by generation.
impl<C: Config> Ord for Key<C> {
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        self.idx
            .cmp(&other.idx)
            .then(self.generation.cmp(&other.generation))
    }
}

impl<C: Config> Hash for Key<C> {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.idx.hash(state);
        self.generation.hash(state);
    }
}

impl<C: Config> fmt::Debug for Key<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Key")
            .field("idx", &self.idx)
            .field("generation", &self.generation)
            .finish()
    }
}
