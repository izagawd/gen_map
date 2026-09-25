use crate::key_piece::KeyPiece;
use core::hint::unreachable_unchecked;

/// An odd number of type `G`. It is never zero, so it is stored as a
/// `NonZero`, which makes `Option<Odd<G>>` the same size as `G`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Odd<G: KeyPiece>(G::NonZero);

impl<G: KeyPiece> Odd<G> {
    /// Returns `value` as an `Odd`, or `None` if it is even.
    #[inline]
    pub fn new(value: G) -> Option<Self> {
        if value.is_odd() {
            // SAFETY: `value` was just found to be odd.
            Some(unsafe { Self::new_unchecked(value) })
        } else {
            None
        }
    }

    /// [`new`](Self::new) without the check.
    ///
    /// # Safety
    ///
    /// `value` must be odd.
    #[inline]
    pub unsafe fn new_unchecked(value: G) -> Self {
        debug_assert!(value.is_odd());
        if !value.is_odd() {
            // SAFETY: the caller promises an odd value. Saying so here lets
            // the compiler rely on it in the code that follows.
            unsafe { unreachable_unchecked() }
        }
        // SAFETY: the caller promises an odd value, and zero is even.
        Self(unsafe { value.into_non_zero_unchecked() })
    }

    /// The number as a `NonZero`.
    #[inline]
    pub fn get(self) -> G::NonZero {
        self.0
    }

    /// The even number after this one. The largest value of `G` wraps around
    /// to zero.
    #[inline]
    pub fn wrapping_next(self) -> Even<G> {
        // An odd number plus one is even, and so is the zero that the largest
        // value wraps around to.
        Even(G::from_non_zero(self.0).wrapping_add(G::ONE))
    }

    /// The even number before this one. It never wraps, because the smallest
    /// odd number is one.
    #[inline]
    pub fn previous(self) -> Even<G> {
        // An odd number minus one is even, and an odd number is at least one,
        // so the subtraction does not wrap.
        Even(G::from_non_zero(self.0).wrapping_sub(G::ONE))
    }
}

/// An even number of type `G`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Even<G: KeyPiece>(G);

impl<G: KeyPiece> Even<G> {
    /// The even number zero.
    pub const ZERO: Self = Self(G::ZERO);

    /// Returns `value` as an `Even`, or `None` if it is odd.
    #[inline]
    pub fn new(value: G) -> Option<Self> {
        if value.is_odd() {
            None
        } else {
            Some(Self(value))
        }
    }

    /// [`new`](Self::new) without the check.
    ///
    /// # Safety
    ///
    /// `value` must be even.
    #[inline]
    pub unsafe fn new_unchecked(value: G) -> Self {
        debug_assert!(!value.is_odd());
        if value.is_odd() {
            // SAFETY: the caller promises an even value. Saying so here lets
            // the compiler rely on it in the code that follows.
            unsafe { unreachable_unchecked() }
        }
        Self(value)
    }

    /// The number.
    #[inline]
    pub fn get(self) -> G {
        self.0
    }

    /// The odd number after this one. It never overflows, because the
    /// largest value of `G` is odd and so is above every even number.
    #[inline]
    pub fn next(self) -> Odd<G> {
        // SAFETY: an even number plus one is odd, and the sum fits in `G`
        // because the largest value of `G` is odd.
        unsafe { Odd::new_unchecked(self.0.wrapping_add(G::ONE)) }
    }

    /// The odd number before this one. Zero wraps around to the largest value
    /// of `G`.
    #[inline]
    pub fn wrapping_previous(self) -> Odd<G> {
        // SAFETY: an even number minus one is odd, and so is the largest value
        // of `G` that zero wraps around to.
        unsafe { Odd::new_unchecked(self.0.wrapping_sub(G::ONE)) }
    }
}
