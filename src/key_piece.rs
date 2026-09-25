use core::fmt::Debug;
use core::hash::Hash;
use core::num::NonZero;

/// An unsigned integer that can be the index or the generation of a
/// [`Key`](crate::Key). Implemented for `u8`, `u16`, `u32`, `u64`, `u128` and
/// `usize`.
///
/// # Safety
///
/// The map reads a union field based on what [`is_odd`](Self::is_odd) says
/// about a slot's generation, so every method must behave exactly like it does
/// for the standard unsigned integers. In particular,
/// [`into_non_zero`](Self::into_non_zero) must return `Some` for every value
/// except [`ZERO`](Self::ZERO). [`from_usize`](Self::from_usize) and
/// [`into_usize`](Self::into_usize) must return `None` for a value that does
/// not fit, and converting a value that fits there and back must give the
/// same value. The largest value must be odd.
pub unsafe trait KeyPiece: Copy + Eq + Ord + Hash + Debug + Send + Sync + 'static {
    /// The `NonZero` form of this integer. A key hands out its generation in
    /// this form, and a [`Split`](crate::Split) key stores it this way, which
    /// makes `Option<Key>` the same size as `Key`.
    type NonZero: Copy + Eq + Ord + Hash + Debug + Send + Sync + 'static;

    /// The value zero.
    const ZERO: Self;

    /// The value one.
    const ONE: Self;

    /// The largest value.
    const MAX: Self;

    /// The number of bits.
    const BITS: u32;

    /// Returns `self + rhs`, or `None` if the sum does not fit in this type.
    fn checked_add(self, rhs: Self) -> Option<Self>;

    /// Returns `self + rhs`, wrapping around at the largest value of this
    /// type. The map only calls this where it knows the sum fits, so the
    /// wrapping never happens, and the method exists so that the add does
    /// not have to be checked twice.
    fn wrapping_add(self, rhs: Self) -> Self;

    /// Returns `self - rhs`, wrapping around below zero to the largest value
    /// of this type.
    fn wrapping_sub(self, rhs: Self) -> Self;

    /// Returns `true` if the lowest bit is set.
    fn is_odd(self) -> bool;

    /// Converts the value to a `usize`, or returns `None` if it does not fit.
    fn into_usize(self) -> Option<usize>;

    /// [`into_usize`](Self::into_usize) for a value that is known to fit.
    ///
    /// # Safety
    ///
    /// The value must fit in a `usize`, meaning
    /// [`into_usize`](Self::into_usize) returns `Some` for it.
    unsafe fn into_usize_unchecked(self) -> usize;

    /// Converts a `usize` to this type, or returns `None` if it does not fit.
    fn from_usize(v: usize) -> Option<Self>;

    /// [`from_usize`](Self::from_usize) for a value that is known to fit.
    ///
    /// # Safety
    ///
    /// `v` must fit in this type, meaning [`from_usize`](Self::from_usize)
    /// returns `Some` for it.
    unsafe fn from_usize_unchecked(v: usize) -> Self;

    /// Converts the value to its `NonZero` form, or returns `None` if it is zero.
    fn into_non_zero(self) -> Option<Self::NonZero>;

    /// [`into_non_zero`](Self::into_non_zero) for a value that is known not
    /// to be zero.
    ///
    /// # Safety
    ///
    /// The value must not be [`ZERO`](Self::ZERO).
    unsafe fn into_non_zero_unchecked(self) -> Self::NonZero;

    /// Converts a `NonZero` value back to the plain integer.
    fn from_non_zero(v: Self::NonZero) -> Self;

    /// Converts the value to a `u128`, which every value fits in.
    fn into_u128(self) -> u128;

    /// Converts a `u128` to this type, or returns `None` if it does not fit.
    fn from_u128(v: u128) -> Option<Self>;

    /// [`from_u128`](Self::from_u128) for a value that is known to fit.
    ///
    /// # Safety
    ///
    /// `v` must fit in this type, meaning [`from_u128`](Self::from_u128)
    /// returns `Some` for it.
    unsafe fn from_u128_unchecked(v: u128) -> Self;
}

macro_rules! impl_key_piece {
    ($($t:ty)*) => {
        $(
            unsafe impl KeyPiece for $t {
                type NonZero = NonZero<$t>;

                const ZERO: Self = 0;
                const ONE: Self = 1;
                const MAX: Self = <$t>::MAX;
                const BITS: u32 = <$t>::BITS;

                #[inline]
                fn checked_add(self, rhs: Self) -> Option<Self> {
                    <$t>::checked_add(self, rhs)
                }

                #[inline]
                fn wrapping_add(self, rhs: Self) -> Self {
                    <$t>::wrapping_add(self, rhs)
                }

                #[inline]
                fn wrapping_sub(self, rhs: Self) -> Self {
                    <$t>::wrapping_sub(self, rhs)
                }

                #[inline]
                fn is_odd(self) -> bool {
                    self & 1 == 1
                }

                #[inline]
                fn into_usize(self) -> Option<usize> {
                    usize::try_from(self).ok()
                }

                /// An `as` cast only truncates a value that does not fit,
                /// and the caller promises the value fits, so the cast is
                /// exact.
                #[inline]
                unsafe fn into_usize_unchecked(self) -> usize {
                    debug_assert!(usize::try_from(self).is_ok());
                    self as usize
                }

                #[inline]
                fn from_usize(v: usize) -> Option<Self> {
                    <$t>::try_from(v).ok()
                }

                /// The same kind of cast, from a `usize` to this type.
                #[inline]
                unsafe fn from_usize_unchecked(v: usize) -> Self {
                    debug_assert!(<$t>::try_from(v).is_ok());
                    v as $t
                }

                #[inline]
                fn into_non_zero(self) -> Option<Self::NonZero> {
                    NonZero::new(self)
                }

                #[inline]
                unsafe fn into_non_zero_unchecked(self) -> Self::NonZero {
                    debug_assert!(self != 0);
                    // SAFETY: the caller promises the value is not zero.
                    unsafe { NonZero::new_unchecked(self) }
                }

                #[inline]
                fn from_non_zero(v: Self::NonZero) -> Self {
                    v.get()
                }

                #[inline]
                fn into_u128(self) -> u128 {
                    self as u128
                }

                #[inline]
                fn from_u128(v: u128) -> Option<Self> {
                    <$t>::try_from(v).ok()
                }

                /// The same cast as `from_usize_unchecked`, for a `u128`.
                #[inline]
                unsafe fn from_u128_unchecked(v: u128) -> Self {
                    debug_assert!(<$t>::try_from(v).is_ok());
                    v as $t
                }
            }
        )*
    };
}

impl_key_piece!(u8 u16 u32 u64 u128 usize);
