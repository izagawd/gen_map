use crate::config::{Config, DefaultConfig};
use crate::key_layout::KeyLayout;
use crate::key_piece::KeyPiece;
use core::cmp::Ordering;
use core::fmt;
use core::hash::{Hash, Hasher};

/// The layout of `C`'s keys.
type Layout<C> = <C as Config>::Layout;

/// What a key of `C` holds.
type Repr<C> = <Layout<C> as KeyLayout<<C as Config>::Idx, <C as Config>::Gen>>::Repr;

/// A key to a value in a [`GenMap`](crate::GenMap), returned by `insert`.
/// The config's [`Layout`](Config::Layout) says how it stores its index and
/// generation.
pub struct Key<C: Config = DefaultConfig> {
    repr: Repr<C>,
}

impl<C: Config> Key<C> {
    /// The index of the slot this key refers to.
    #[inline]
    pub fn idx(&self) -> C::Idx {
        <Layout<C> as KeyLayout<C::Idx, C::Gen>>::idx(self.repr)
    }

    /// The generation of the slot this key refers to at the time this key was handed out.
    #[inline]
    pub fn generation(&self) -> C::Gen {
        let generation = <Layout<C> as KeyLayout<C::Idx, C::Gen>>::generation(self.repr);
        C::Gen::from_non_zero(generation)
    }

    /// [`generation`](Self::generation) as the `NonZero` that
    /// [`from_raw_parts`](Self::from_raw_parts) takes. A key's generation is
    /// always odd, so it is never zero.
    #[inline]
    pub fn generation_non_zero(&self) -> <C::Gen as KeyPiece>::NonZero {
        <Layout<C> as KeyLayout<C::Idx, C::Gen>>::generation(self.repr)
    }

    /// Builds a key from an index and a generation.
    ///
    /// # Safety
    ///
    /// `generation` must be odd. The map only hands out odd generations, and
    /// an even one is what a vacant or detached slot holds. A key that
    /// carries an even generation can match such a slot, which is undefined behavior.
    /// Both parts must also fit the layout, meaning `idx` is at most
    /// [`KeyLayout::max_idx`] and `generation` at most
    /// [`KeyLayout::max_generation`].
    #[inline]
    pub unsafe fn from_raw_parts(idx: C::Idx, generation: <C::Gen as KeyPiece>::NonZero) -> Self {
        let generation = C::Gen::from_non_zero(generation);
        debug_assert!(generation.is_odd());
        // SAFETY: the caller promises an odd generation that fits the
        // layout, and an index that fits it.
        let repr =
            unsafe { <Layout<C> as KeyLayout<C::Idx, C::Gen>>::pack_unchecked(idx, generation) };
        Self { repr }
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
        self.repr == other.repr
    }
}

impl<C: Config> Eq for Key<C> {}

impl<C: Config> PartialOrd for Key<C> {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Keys are ordered by index, then by generation, whatever the layout.
impl<C: Config> Ord for Key<C> {
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        self.idx()
            .cmp(&other.idx())
            .then(self.generation().cmp(&other.generation()))
    }
}

impl<C: Config> Hash for Key<C> {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.repr.hash(state);
    }
}

impl<C: Config> fmt::Debug for Key<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Key")
            .field("idx", &self.idx())
            .field("generation", &self.generation())
            .finish()
    }
}
