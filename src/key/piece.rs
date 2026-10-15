use core::fmt::{self, Debug, Display};
use core::hash::Hash;
use core::num::NonZero;
use core::panic::{RefUnwindSafe, UnwindSafe};

// A `Slot` reads one of its union fields based on what `is_odd` says about its
// generation, and `SingleVec` and `PairVec` build slices from the lengths they
// store in this type. So the crate's unsafe code relies on every method
// behaving exactly like it does for an unsigned integer with `BITS` bits. In
// particular, `into_non_zero` returns `Some` for every value except `ZERO`, and
// `into_non_max` returns `Some` for every value except `MAX`. `NON_MAX_ZERO` is
// `ZERO` in its `NonMax` form. `from_usize`, `into_usize` and `from_u128`
// return `None` for a value that does not fit in the target type. Converting a
// value to its `NonZero` or `NonMax` form, or to a type that can hold it, and
// back gives the same value. The largest value is odd, which `Odd`, `Even` and
// `Split`'s `max_generation` rely on.

/// A `KeyPiece` is an unsigned integer that can be the index or the generation
/// of a [`Key`](crate::Key), or the length type of a
/// [`SingleVec`](crate::SingleVec) or a [`PairVec`](crate::PairVec). It is
/// implemented for [`u1`], `u8`, `u16`, `u32`, `u64`, `u128` and `usize`.
///
/// It is sealed, so it cannot be implemented outside this crate.
pub trait KeyPiece:
    sealed::Sealed
    + Copy
    + Eq
    + Ord
    + Hash
    + Debug
    + Send
    + Sync
    + Unpin
    + UnwindSafe
    + RefUnwindSafe
    + 'static
{
    /// The `NonZero` form of this type is [`NonZeroU1`] for [`u1`] and
    /// `NonZero<Self>` for the other integers.
    type NonZero: Copy
        + Eq
        + Ord
        + Hash
        + Debug
        + Send
        + Sync
        + Unpin
        + UnwindSafe
        + RefUnwindSafe
        + 'static;

    /// The `NonMax` form of this type is [`NonMaxU1`] for [`u1`] and
    /// [`NonMax<Self>`](NonMax) for the other integers.
    type NonMax: Copy
        + Eq
        + Ord
        + Hash
        + Debug
        + Send
        + Sync
        + Unpin
        + UnwindSafe
        + RefUnwindSafe
        + 'static;

    /// The value zero.
    const ZERO: Self;

    /// `NON_MAX_ZERO` is the value zero in its `NonMax` form.
    const NON_MAX_ZERO: Self::NonMax;

    /// The value one.
    const ONE: Self;

    /// The largest value.
    const MAX: Self;

    /// The number of bits.
    const BITS: u32;

    /// Returns `self + rhs`, or `None` if the sum does not fit in this type.
    fn checked_add(self, rhs: Self) -> Option<Self>;

    /// Returns `self + rhs`, wrapping around at the largest value of this
    /// type.
    fn wrapping_add(self, rhs: Self) -> Self;

    /// Returns `self - rhs`, wrapping around below zero to the largest value
    /// of this type.
    fn wrapping_sub(self, rhs: Self) -> Self;

    /// Returns `true` if the lowest bit is set.
    fn is_odd(self) -> bool;

    /// Converts the value to a `usize`, or returns `None` if it does not fit.
    fn into_usize(self) -> Option<usize>;

    /// Converts the value to a `usize`, like [`into_usize`](Self::into_usize),
    /// but without checking that it fits.
    ///
    /// # Safety
    ///
    /// The value must fit in a `usize`, meaning
    /// [`into_usize`](Self::into_usize) returns `Some` for it.
    unsafe fn into_usize_unchecked(self) -> usize;

    /// Converts a `usize` to this type, or returns `None` if it does not fit.
    fn from_usize(v: usize) -> Option<Self>;

    /// Converts a `usize` to this type, like [`from_usize`](Self::from_usize),
    /// but without checking that it fits.
    ///
    /// # Safety
    ///
    /// `v` must fit in this type, meaning [`from_usize`](Self::from_usize)
    /// returns `Some` for it.
    unsafe fn from_usize_unchecked(v: usize) -> Self;

    /// Converts the value to its `NonZero` form, or returns `None` if it is
    /// zero.
    fn into_non_zero(self) -> Option<Self::NonZero>;

    /// Converts the value to its `NonZero` form, like
    /// [`into_non_zero`](Self::into_non_zero), but without checking that it is
    /// not zero.
    ///
    /// # Safety
    ///
    /// The value must not be [`ZERO`](Self::ZERO).
    unsafe fn into_non_zero_unchecked(self) -> Self::NonZero;

    /// Converts a `NonZero` value back to this type.
    fn from_non_zero(v: Self::NonZero) -> Self;

    /// Converts the value to its `NonMax` form, or returns `None` if it is the
    /// largest value.
    fn into_non_max(self) -> Option<Self::NonMax>;

    /// Converts the value to its `NonMax` form, like
    /// [`into_non_max`](Self::into_non_max), but without checking that it is
    /// not the largest value.
    ///
    /// # Safety
    ///
    /// The value must not be [`MAX`](Self::MAX).
    unsafe fn into_non_max_unchecked(self) -> Self::NonMax;

    /// Converts a `NonMax` value back to this type.
    fn from_non_max(v: Self::NonMax) -> Self;

    /// Converts the value to a `u128`, which every value fits in.
    fn into_u128(self) -> u128;

    /// Converts a `u128` to this type, or returns `None` if it does not fit.
    fn from_u128(v: u128) -> Option<Self>;

    /// Converts a `u128` to this type, like [`from_u128`](Self::from_u128), but
    /// without checking that it fits.
    ///
    /// # Safety
    ///
    /// `v` must fit in this type, meaning [`from_u128`](Self::from_u128)
    /// returns `Some` for it.
    unsafe fn from_u128_unchecked(v: u128) -> Self;
}

mod sealed {
    /// Keeps [`KeyPiece`](super::KeyPiece) from being implemented outside
    /// this crate.
    pub trait Sealed {}
}

/// A `NonMax<T>` is a value of type `T` that is not the largest value of `T`.
/// `T` can be `u8`, `u16`, `u32`, `u64`, `u128` or `usize`. A `NonMax<T>` has
/// the layout of `T`, so `Option<NonMax<T>>` is larger than `T`. `Debug` and
/// `Display` print it the same way as the `T` it holds.
///
/// # Examples
///
/// ```
/// use gen_map::NonMax;
///
/// assert_eq!(NonMax::<u8>::new(7).map(NonMax::get), Some(7));
/// assert_eq!(NonMax::<u8>::new(u8::MAX), None);
/// assert_eq!(core::mem::size_of::<NonMax<u32>>(), 4);
/// ```
// `repr(transparent)` gives a `NonMax<T>` the layout of `T`. The comment on
// `Even` explains what relies on this.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct NonMax<T>(T);

impl<T: Copy> NonMax<T> {
    /// Returns the value as a plain `T`.
    #[inline]
    pub const fn get(self) -> T {
        self.0
    }
}

impl<T: Debug> Debug for NonMax<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Debug::fmt(&self.0, f)
    }
}

impl<T: Display> Display for NonMax<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.0, f)
    }
}

/// Adds `new` and `new_unchecked` to `NonMax<T>` for each integer type `T`
/// that the macro call lists.
macro_rules! impl_non_max {
    ($($t:ty)*) => {
        $(
            impl NonMax<$t> {
                /// Returns `n` as a `NonMax`, or `None` if `n` is the largest
                /// value of its type.
                #[inline]
                pub const fn new(n: $t) -> Option<Self> {
                    if n == <$t>::MAX {
                        None
                    } else {
                        Some(Self(n))
                    }
                }

                /// Returns `n` as a `NonMax`, like [`new`](Self::new), but
                /// without checking that `n` is not the largest value of its
                /// type.
                ///
                /// # Safety
                ///
                /// `n` must not be the largest value of its type.
                #[inline]
                pub const unsafe fn new_unchecked(n: $t) -> Self {
                    debug_assert!(n != <$t>::MAX);
                    Self(n)
                }
            }
        )*
    };
}

impl_non_max!(u8 u16 u32 u64 u128 usize);

macro_rules! impl_key_piece {
    ($($t:ty)*) => {
        $(
            impl sealed::Sealed for $t {}

            // This macro implements the trait for the standard unsigned
            // integers. Every method forwards to the integer's own operation,
            // to an exact cast or to the constructors and `get` methods of
            // `NonZero` and `NonMax`, and the largest value of every unsigned
            // integer is odd.
            impl KeyPiece for $t {
                type NonZero = NonZero<$t>;
                type NonMax = NonMax<$t>;

                const ZERO: Self = 0;
                const NON_MAX_ZERO: Self::NonMax = NonMax(0);
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

                // An `as` cast only truncates a value that does not fit,
                // and the caller promises the value fits, so the cast is
                // exact.
                #[inline]
                unsafe fn into_usize_unchecked(self) -> usize {
                    debug_assert!(usize::try_from(self).is_ok());
                    self as usize
                }

                #[inline]
                fn from_usize(v: usize) -> Option<Self> {
                    <$t>::try_from(v).ok()
                }

                // An `as` cast from a `usize` is exact here too, because the
                // caller promises that `v` fits in this type.
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
                fn into_non_max(self) -> Option<Self::NonMax> {
                    NonMax::<$t>::new(self)
                }

                #[inline]
                unsafe fn into_non_max_unchecked(self) -> Self::NonMax {
                    // SAFETY: the caller promises the value is not the largest
                    // value.
                    unsafe { NonMax::<$t>::new_unchecked(self) }
                }

                #[inline]
                fn from_non_max(v: Self::NonMax) -> Self {
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

                // An `as` cast from a `u128` is exact here too, because the
                // caller promises that `v` fits in this type.
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

/// `u1` is an unsigned integer with one bit, so its only values are zero and
/// one. It has the layout of a `bool`, where `false` is zero and `true` is
/// one. `Debug` and `Display` print it as `0` or `1`.
///
/// # Examples
///
/// ```
/// use gen_map::u1;
///
/// assert_eq!(u1::from(true), u1::MAX);
/// assert!(!bool::from(u1::MIN));
/// assert_eq!(u1::default(), u1::MIN);
/// assert_eq!(format!("{} {:?}", u1::MIN, u1::MAX), "0 1");
/// assert_eq!(core::mem::size_of::<u1>(), 1);
/// ```
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(transparent)]
pub struct u1(bool);

impl u1 {
    /// `MIN` is the smallest value, which is zero.
    pub const MIN: Self = Self(false);

    /// `MAX` is the largest value, which is one.
    pub const MAX: Self = Self(true);

    /// `BITS` is the number of bits, which is one.
    pub const BITS: u32 = 1;
}

impl From<bool> for u1 {
    #[inline]
    fn from(value: bool) -> Self {
        Self(value)
    }
}

impl From<u1> for bool {
    #[inline]
    fn from(value: u1) -> Self {
        value.0
    }
}

impl Debug for u1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Debug::fmt(&u8::from(self.0), f)
    }
}

impl Display for u1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(&u8::from(self.0), f)
    }
}

/// `NonZeroU1` is the `NonZero` form of a [`u1`]. One is the only `u1` that is
/// not zero, so `NonZeroU1` has a single value, which stands for one, and it
/// takes no space. `Debug` and `Display` print it the same way as the `u1` it
/// stands for.
///
/// # Examples
///
/// ```
/// use gen_map::{u1, KeyPiece, NonZeroU1};
///
/// assert_eq!(u1::MAX.into_non_zero(), Some(NonZeroU1));
/// assert_eq!(u1::MIN.into_non_zero(), None);
/// assert_eq!(NonZeroU1.get(), u1::MAX);
/// ```
// `repr(transparent)` gives `NonZeroU1` the layout of `()`, so it takes no
// space and has an alignment of one. The comment on `Odd` explains what relies
// on this.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct NonZeroU1;

impl NonZeroU1 {
    /// Returns the `u1` that `NonZeroU1` stands for, which is always one.
    #[inline]
    pub const fn get(self) -> u1 {
        u1::MAX
    }
}

impl Debug for NonZeroU1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Debug::fmt(&self.get(), f)
    }
}

impl Display for NonZeroU1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.get(), f)
    }
}

/// `NonMaxU1` is the `NonMax` form of a [`u1`]. Zero is the only `u1` that is
/// not the largest value, so `NonMaxU1` has a single value, which stands for
/// zero, and it takes no space. `Debug` and `Display` print it the same way as
/// the `u1` it stands for.
///
/// # Examples
///
/// ```
/// use gen_map::{u1, KeyPiece, NonMaxU1};
///
/// assert_eq!(u1::MIN.into_non_max(), Some(NonMaxU1));
/// assert_eq!(u1::MAX.into_non_max(), None);
/// assert_eq!(NonMaxU1.get(), u1::MIN);
/// ```
// `repr(transparent)` gives `NonMaxU1` the layout of `()`, so it takes no space
// and has an alignment of one. The comment on `Even` explains what relies on
// this.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct NonMaxU1;

impl NonMaxU1 {
    /// Returns the `u1` that `NonMaxU1` stands for, which is always zero.
    #[inline]
    pub const fn get(self) -> u1 {
        u1::MIN
    }
}

impl Debug for NonMaxU1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Debug::fmt(&self.get(), f)
    }
}

impl Display for NonMaxU1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.get(), f)
    }
}

impl sealed::Sealed for u1 {}

// Every method below works like it does for an unsigned integer with one bit,
// so the contract at the top of this file holds. Adding or subtracting two
// one-bit integers wraps around at two, which gives the same result as `^` on
// their `bool`s. The largest value is one, which is odd.
impl KeyPiece for u1 {
    type NonZero = NonZeroU1;
    type NonMax = NonMaxU1;

    const ZERO: Self = Self(false);
    const NON_MAX_ZERO: Self::NonMax = NonMaxU1;
    const ONE: Self = Self(true);
    const MAX: Self = Self(true);
    const BITS: u32 = 1;

    #[inline]
    fn checked_add(self, rhs: Self) -> Option<Self> {
        if self.0 && rhs.0 {
            None
        } else {
            Some(Self(self.0 | rhs.0))
        }
    }

    #[inline]
    fn wrapping_add(self, rhs: Self) -> Self {
        Self(self.0 ^ rhs.0)
    }

    #[inline]
    fn wrapping_sub(self, rhs: Self) -> Self {
        Self(self.0 ^ rhs.0)
    }

    #[inline]
    fn is_odd(self) -> bool {
        self.0
    }

    #[inline]
    fn into_usize(self) -> Option<usize> {
        Some(usize::from(self.0))
    }

    #[inline]
    unsafe fn into_usize_unchecked(self) -> usize {
        usize::from(self.0)
    }

    #[inline]
    fn from_usize(v: usize) -> Option<Self> {
        match v {
            0 => Some(Self(false)),
            1 => Some(Self(true)),
            _ => None,
        }
    }

    // The caller promises that `v` fits in a `u1`, so `v` is zero or one, and
    // `v != 0` turns those into `false` and `true`.
    #[inline]
    unsafe fn from_usize_unchecked(v: usize) -> Self {
        debug_assert!(v <= 1);
        Self(v != 0)
    }

    #[inline]
    fn into_non_zero(self) -> Option<Self::NonZero> {
        self.0.then_some(NonZeroU1)
    }

    #[inline]
    unsafe fn into_non_zero_unchecked(self) -> Self::NonZero {
        debug_assert!(self.0);
        NonZeroU1
    }

    #[inline]
    fn from_non_zero(_: Self::NonZero) -> Self {
        Self(true)
    }

    #[inline]
    fn into_non_max(self) -> Option<Self::NonMax> {
        (!self.0).then_some(NonMaxU1)
    }

    #[inline]
    unsafe fn into_non_max_unchecked(self) -> Self::NonMax {
        debug_assert!(!self.0);
        NonMaxU1
    }

    #[inline]
    fn from_non_max(_: Self::NonMax) -> Self {
        Self(false)
    }

    #[inline]
    fn into_u128(self) -> u128 {
        u128::from(self.0)
    }

    #[inline]
    fn from_u128(v: u128) -> Option<Self> {
        match v {
            0 => Some(Self(false)),
            1 => Some(Self(true)),
            _ => None,
        }
    }

    // The caller promises that `v` fits in a `u1`, so `v` is zero or one, and
    // `v != 0` turns those into `false` and `true`.
    #[inline]
    unsafe fn from_u128_unchecked(v: u128) -> Self {
        debug_assert!(v <= 1);
        Self(v != 0)
    }
}
