use crate::key::piece::KeyPiece;
use core::hint::unreachable_unchecked;

/// An `Odd` is an odd number of type `G`. It is never zero, so an `Odd` stores
/// it as [`G::NonZero`](KeyPiece::NonZero), which makes `Option<Odd<G>>` the
/// same size as `G`.
// `repr(transparent)` gives an `Odd` the layout of `G::NonZero`. `KeyPiece` is
// sealed, and `G::NonZero` is either `NonZeroU1` for `u1` or the standard
// `NonZero` of another integer, which has the layout of that integer.
// `NonZeroU1` takes no space, has an alignment of one and has a single value,
// which stands for one, the only odd `u1`. So casting a reference to an odd
// `G` gives a valid reference to an `Odd<G>`. `UnionSlot` relies on this to
// hand out its generation as a reference to an `Odd`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
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

    /// Returns `value` as an `Odd`, like [`new`](Self::new), but without
    /// checking that it is odd.
    ///
    /// # Safety
    ///
    /// `value` must be odd.
    #[inline]
    pub unsafe fn new_unchecked(value: G) -> Self {
        debug_assert!(value.is_odd());
        if !value.is_odd() {
            // SAFETY: the caller promises an odd value, so this branch never
            // runs. Marking it unreachable lets the compiler assume that the
            // value is odd in the code that follows.
            unsafe { unreachable_unchecked() }
        }
        // SAFETY: the caller promises an odd value, and zero is even.
        Self(unsafe { value.into_non_zero_unchecked() })
    }

    /// Returns the number as a `G::NonZero`.
    #[inline]
    pub fn get(self) -> G::NonZero {
        self.0
    }

    /// The even number after this one. The largest value of `G` wraps around
    /// to zero.
    #[inline]
    pub fn wrapping_next(self) -> Even<G> {
        // SAFETY: an odd number plus one is even, and so is the zero that the
        // largest value wraps around to.
        unsafe { Even::new_unchecked(G::from_non_zero(self.0).wrapping_add(G::ONE)) }
    }

    /// The even number before this one. It never wraps, because the smallest
    /// odd number is one.
    #[inline]
    pub fn previous(self) -> Even<G> {
        // SAFETY: an odd number minus one is even, and an odd number is at
        // least one, so the subtraction does not wrap.
        unsafe { Even::new_unchecked(G::from_non_zero(self.0).wrapping_sub(G::ONE)) }
    }
}

/// An `Even` is an even number of type `G`. It is never the largest value of
/// `G`, which is odd, so an `Even` stores it as
/// [`G::NonMax`](KeyPiece::NonMax).
// `repr(transparent)` gives an `Even` the layout of `G::NonMax`. `KeyPiece` is
// sealed, and `G::NonMax` is either `NonMaxU1` for `u1` or the `NonMax` of
// another integer, which has the layout of that integer. `NonMaxU1` takes no
// space, has an alignment of one and has a single value, which stands for
// zero, the only even `u1`. So casting a reference to an even `G` gives a
// valid reference to an `Even<G>`. `UnionSlot` relies on this to hand out its
// generation as a reference to an `Even`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct Even<G: KeyPiece>(G::NonMax);

impl<G: KeyPiece> Even<G> {
    /// The even number zero.
    pub const ZERO: Self = Self(G::NON_MAX_ZERO);

    /// Returns `value` as an `Even`, or `None` if it is odd.
    #[inline]
    pub fn new(value: G) -> Option<Self> {
        if value.is_odd() {
            None
        } else {
            // SAFETY: `value` was just found to be even.
            Some(unsafe { Self::new_unchecked(value) })
        }
    }

    /// Returns `value` as an `Even`, like [`new`](Self::new), but without
    /// checking that it is even.
    ///
    /// # Safety
    ///
    /// `value` must be even.
    #[inline]
    pub unsafe fn new_unchecked(value: G) -> Self {
        debug_assert!(!value.is_odd());
        if value.is_odd() {
            // SAFETY: the caller promises an even value, so this branch never
            // runs. Marking it unreachable lets the compiler assume that the
            // value is even in the code that follows.
            unsafe { unreachable_unchecked() }
        }
        // SAFETY: the caller promises an even value, and the largest value of
        // `G` is odd, so `value` is not the largest value.
        Self(unsafe { value.into_non_max_unchecked() })
    }

    /// Returns the number as a `G::NonMax`.
    #[inline]
    pub fn get(self) -> G::NonMax {
        self.0
    }

    /// The odd number after this one. It never overflows, because the
    /// largest value of `G` is odd and so is above every even number.
    #[inline]
    pub fn next(self) -> Odd<G> {
        // SAFETY: an even number plus one is odd, and the sum fits in `G`
        // because the largest value of `G` is odd.
        unsafe { Odd::new_unchecked(G::from_non_max(self.0).wrapping_add(G::ONE)) }
    }

    /// The odd number before this one. Zero wraps around to the largest value
    /// of `G`.
    #[inline]
    pub fn wrapping_previous(self) -> Odd<G> {
        // SAFETY: an even number minus one is odd, and so is the largest value
        // of `G` that zero wraps around to.
        unsafe { Odd::new_unchecked(G::from_non_max(self.0).wrapping_sub(G::ONE)) }
    }
}
