use crate::config::{Config, DefaultConfig};
use crate::key_piece::KeyPiece;

/// A key to a value in a [`GenMap`](crate::GenMap), returned by `insert`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key<
    Idx: KeyPiece = <DefaultConfig as Config>::Idx,
    Gen: KeyPiece = <DefaultConfig as Config>::Gen,
> {
    pub(crate) idx: Idx,
    pub(crate) generation: Gen::NonZero,
}

/// The key type of a `GenMap<T, C>`.
pub type KeyOf<C> = Key<<C as Config>::Idx, <C as Config>::Gen>;

impl<Idx: KeyPiece, Gen: KeyPiece> Key<Idx, Gen> {
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
