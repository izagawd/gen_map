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
/// for the standard unsigned integers. In particular
/// [`into_non_zero`](Self::into_non_zero) must return `Some` for every value
/// except [`ZERO`](Self::ZERO), and [`from_usize`](Self::from_usize) and
/// [`into_usize`](Self::into_usize) must return `None` for a value that does
/// not fit.
pub unsafe trait KeyPiece: Copy + Eq + Ord + Hash + Debug + Send + Sync + 'static {
    /// The `NonZero` form of this integer. A key stores its generation in
    /// this form, which makes `Option<Key>` the same size as `Key`.
    type NonZero: Copy + Eq + Ord + Hash + Debug + Send + Sync + 'static;

    /// The value zero.
    const ZERO: Self;

    /// The value one.
    const ONE: Self;

    /// Returns `self + rhs`, or `None` if the sum does not fit in this type.
    fn checked_add(self, rhs: Self) -> Option<Self>;

    /// Returns `true` if the lowest bit is set.
    fn is_odd(self) -> bool;

    /// Converts the value to a `usize`, or returns `None` if it does not fit.
    fn into_usize(self) -> Option<usize>;

    /// Converts a `usize` to this type, or returns `None` if it does not fit.
    fn from_usize(v: usize) -> Option<Self>;

    /// Converts the value to its `NonZero` form, or returns `None` if it is zero.
    fn into_non_zero(self) -> Option<Self::NonZero>;

    /// Converts a `NonZero` value back to the plain integer.
    fn from_non_zero(v: Self::NonZero) -> Self;
}

macro_rules! impl_key_piece {
    ($($t:ty)*) => {
        $(
            unsafe impl KeyPiece for $t {
                type NonZero = NonZero<$t>;

                const ZERO: Self = 0;
                const ONE: Self = 1;

                #[inline]
                fn checked_add(self, rhs: Self) -> Option<Self> {
                    <$t>::checked_add(self, rhs)
                }

                #[inline]
                fn is_odd(self) -> bool {
                    self & 1 == 1
                }

                #[inline]
                fn into_usize(self) -> Option<usize> {
                    usize::try_from(self).ok()
                }

                #[inline]
                fn from_usize(v: usize) -> Option<Self> {
                    <$t>::try_from(v).ok()
                }

                #[inline]
                fn into_non_zero(self) -> Option<Self::NonZero> {
                    NonZero::new(self)
                }

                #[inline]
                fn from_non_zero(v: Self::NonZero) -> Self {
                    v.get()
                }
            }
        )*
    };
}

impl_key_piece!(u8 u16 u32 u64 u128 usize);
