use crate::config::{Config, DefaultConfig};
use crate::key_piece::KeyPiece;

/// A key to a value in a [`GenMap`](crate::GenMap), returned by `insert`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key<
    Idx: KeyPiece = <DefaultConfig as Config>::Idx,
    Gen: KeyPiece = <DefaultConfig as Config>::Gen,
> {
    idx: Idx,
    generation: Gen::NonZero,
}

/// The key type of a `GenMap<T, C>`.
pub type KeyOf<C> = Key<<C as Config>::Idx, <C as Config>::Gen>;

impl<Idx: KeyPiece, Gen: KeyPiece> Key<Idx, Gen> {
    /// Builds a key from an index and a generation.
    ///
    /// Returns `None` if `generation` is even, since such a key could never
    /// match a slot.
    #[inline]
    pub(crate) fn from_parts(idx: Idx, generation: Gen) -> Option<Self> {
        if !generation.is_odd() {
            return None;
        }
        Some(Self {
            idx,
            generation: generation.into_non_zero()?,
        })
    }

    /// The slot's index.
    #[inline]
    pub(crate) fn index(&self) -> Idx {
        self.idx
    }

    /// The slot's generation.
    #[inline]
    pub(crate) fn generation(&self) -> Gen {
        Gen::from_non_zero(self.generation)
    }

    /// # Safety
    ///
    /// `generation` must be odd.
    #[inline]
    pub(crate) unsafe fn from_parts_unchecked(idx: Idx, generation: Gen) -> Self {
        debug_assert!(generation.is_odd());
        Self {
            idx,
            generation: generation.into_non_zero().unwrap_unchecked(),
        }
    }
}
