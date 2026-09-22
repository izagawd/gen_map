use crate::config::{Config, DefaultConfig};
use crate::key_piece::KeyPiece;

/// A handle to a value in a [`GenMap`](crate::GenMap).
///
/// Holds the slot index and the generation the slot had when the value was
/// inserted. Once the value is removed the key matches nothing, so lookups with
/// it return `None`.
///
/// The type parameters default to those of [`DefaultConfig`], so `Key` is the
/// key of `GenMap<T>`. [`KeyOf`] names the key of any other config.
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
    /// Rebuilds a key from the parts [`into_parts`](Self::into_parts) gave out.
    ///
    /// Returns `None` if `generation` is even, since such a key could never
    /// match a slot.
    #[inline]
    pub fn from_parts(idx: Idx, generation: Gen) -> Option<Self> {
        if !generation.is_odd() {
            return None;
        }
        Some(Self {
            idx,
            generation: generation.into_non_zero()?,
        })
    }

    /// Splits the key into its index and generation.
    #[inline]
    pub fn into_parts(self) -> (Idx, Gen) {
        (self.idx, self.generation())
    }

    /// The slot index this key points at.
    #[inline]
    pub fn index(&self) -> Idx {
        self.idx
    }

    /// The generation the slot had when this key was handed out. Always odd.
    #[inline]
    pub fn generation(&self) -> Gen {
        Gen::from_non_zero(self.generation)
    }

    /// The generation in its `NonZero` form.
    #[inline]
    pub fn generation_non_zero(&self) -> Gen::NonZero {
        self.generation
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
