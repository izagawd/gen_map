use crate::key::parity::Even;
use crate::key::piece::KeyPiece;
use crate::parity_union::ParityUnion;
use core::ops::{Deref, DerefMut};

/// A `Slot` wraps a [`ParityUnion`]. A slot holds a value while the
/// generation of its `ParityUnion` is odd, and it holds no value while that
/// generation is even. This struct is created to be used on the maps,
/// to enable things like default implementations in case a storage needs the slot to have it.
///
/// A `Slot` dereferences to its `ParityUnion`, so the methods of the
/// `ParityUnion` that take `&self` or `&mut self` can be called on the `Slot`.
///
/// `Slot` implements `Default` when its `U` is `()` or a
/// [`KeyPiece`](crate::KeyPiece), and the default slot holds no value.
///
/// # Examples
///
/// ```
/// use gen_map::{Odd, ParityUnion, Slot};
///
/// let one = Odd::new(1).unwrap();
/// let mut slot = Slot(ParityUnion::<u32, &str, ()>::new_odd(one, "a"));
/// // `get_odd` is a method of the `ParityUnion`.
/// assert_eq!(slot.get_odd(one), Some(&"a"));
///
/// // The default slot holds no value, so its generation is even.
/// slot = Slot::default();
/// assert!(slot.is_even());
/// ```
// `repr(transparent)` gives a `Slot` the layout of its `ParityUnion`, so a
// `Slot` takes no more space than the `ParityUnion` it wraps.
#[derive(Clone, Debug)]
#[repr(transparent)]
pub struct Slot<G: KeyPiece, T, U>(
    /// This `ParityUnion` holds the generation of the slot together with its
    /// `T` or its `U`.
    pub ParityUnion<G, T, U>,
);

impl<G: KeyPiece, T, I: KeyPiece> Default for Slot<G, T, I> {
    /// Returns a slot that holds no value. Its generation is zero and it
    /// stores the largest value of `I`.
    #[inline]
    fn default() -> Self {
        Self(ParityUnion::new_even(Even::ZERO, I::MAX))
    }
}

impl<G: KeyPiece, T> Default for Slot<G, T, ()> {
    /// Returns a slot that holds no value. Its generation is zero and it
    /// stores `()`.
    #[inline]
    fn default() -> Self {
        Self(ParityUnion::new_even(Even::ZERO, ()))
    }
}

impl<G: KeyPiece, T, U> Deref for Slot<G, T, U> {
    type Target = ParityUnion<G, T, U>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<G: KeyPiece, T, U> DerefMut for Slot<G, T, U> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
