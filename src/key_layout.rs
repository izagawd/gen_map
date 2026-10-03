use crate::config::KeyConfig;
use crate::key_piece::KeyPiece;
use crate::parity::Odd;

/// Stores the index and the generation as two fields, so a key is as large
/// as the two put together, plus any padding their alignment needs. A key
/// can hold any index of type `Idx` and any odd generation of type `Gen`.
///
/// `Idx` and `Gen` can each be any unsigned integer from `u8` to `u128`, or
/// `usize`. Both default to `u32`, so `Split` on its own is the same type as
/// `Split<u32, u32>`.
///
/// # Examples
///
/// ```
/// use gen_map::{Key, Split};
///
/// assert_eq!(core::mem::size_of::<Key<Split<u16, u16>>>(), 4);
/// assert_eq!(core::mem::size_of::<Key<Split>>(), 8);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Split<Idx = u32, Gen: KeyPiece = u32> {
    idx: Idx,
    /// An odd generation is never zero, and an [`Odd`] stores it as a
    /// `NonZero`, which lets `Option` use zero to represent `None` and gives
    /// `Option<Key>` the size of `Key`.
    generation: Odd<Gen>,
}

// SAFETY: `pack_unchecked` stores the index and the generation in two fields,
// and `idx` and `generation` return those fields unchanged. The fields are
// private, so a `Split` only comes from `pack_unchecked`.
unsafe impl<Idx: KeyPiece, Gen: KeyPiece> KeyConfig for Split<Idx, Gen> {
    type Idx = Idx;
    type Gen = Gen;

    #[inline]
    fn max_idx() -> Idx {
        Idx::MAX
    }

    #[inline]
    fn max_generation() -> Odd<Gen> {
        // SAFETY: every `KeyPiece` type is an unsigned integer, and the
        // largest value of an unsigned integer is odd.
        unsafe { Odd::new_unchecked(Gen::MAX) }
    }

    #[inline]
    unsafe fn pack_unchecked(idx: Idx, generation: Odd<Gen>) -> Self {
        Self { idx, generation }
    }

    #[inline]
    fn idx(self) -> Idx {
        self.idx
    }

    #[inline]
    fn generation(self) -> Odd<Gen> {
        self.generation
    }
}

/// Stores the index and the generation in the bits of one integer `R`, so a
/// key is as large as `R`. The low `GEN_BITS` bits hold the generation and
/// the bits above them hold the index.
///
/// `R` can be `u8`, `u16`, `u32`, `u64` or `u128`. It cannot be `usize`,
/// because `Packed` picks its index and generation types from the number of
/// bits in `R`, and the number of bits in a `usize` depends on the target. To
/// get keys the size of a pointer, pick `u32` or `u64` for `R` with
/// `cfg(target_pointer_width)`. `GEN_BITS` must be at least one and less than
/// the bits of `R`. A `Packed` that breaks these rules does not implement
/// [`KeyConfig`], so a map config cannot use it.
///
/// The index type is the smallest unsigned integer with at least
/// `R::BITS - GEN_BITS` bits, and the generation type is the smallest one
/// with at least `GEN_BITS` bits.
///
/// | Key config | Index type | Generation type |
/// | --- | --- | --- |
/// | `Packed<u16, 4>` | `u16` | `u8` |
/// | `Packed<u32, 8>` | `u32` | `u8` |
/// | `Packed<u32, 16>` | `u16` | `u16` |
/// | `Packed<u64, 40>` | `u32` | `u64` |
///
/// The largest index is `(1 << (R::BITS - GEN_BITS)) - 1` and the largest
/// generation is `(1 << GEN_BITS) - 1`. When the index bits fill the whole
/// index type, as in `Packed<u32, 16>`, the largest index is also the largest
/// value of that type. No map ever gives a slot that index, so such a map can hold
/// one slot fewer than the index bits could address. When a value is removed
/// from a slot that has the largest generation, the slot retires or its
/// generation wraps back to zero, as the `WRAP_ON_OVERFLOW` of the map's
/// config decides. [`GenMapConfig`](crate::GenMapConfig) and
/// [`DenseGenMapConfig`](crate::DenseGenMapConfig) both have it.
///
/// # Examples
///
/// ```
/// use gen_map::{GenMap, GenMapConfig, GenSlotItem, Key, MapConfig, Packed};
///
/// /// Maps with this config hand out four byte keys with 24 bits of index
/// /// and 8 bits of generation.
/// struct Compact;
///
/// impl MapConfig for Compact {
///     type KeyConfig = Packed<u32, 8>;
/// }
///
/// impl GenMapConfig for Compact {
///     type Storage<S: GenSlotItem> = Vec<S>;
/// }
///
/// let mut map = GenMap::<&str, Compact>::new_with_config();
/// let key = map.insert("a");
/// assert_eq!(core::mem::size_of_val(&key), 4);
/// assert_eq!(core::mem::size_of::<Option<Key<Packed<u32, 8>>>>(), 4);
/// assert_eq!(key.idx(), 0);
/// // `generation` returns an `Odd<u8>`. Its `get` returns a `NonZero<u8>`,
/// // and that type's `get` returns the `u8`.
/// assert_eq!(key.generation().get().get(), 1);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Packed<R: KeyPiece, const GEN_BITS: u32>(
    /// Every generation that `pack_unchecked` packs is odd, so the lowest bit
    /// is always set and the integer is never zero. Storing it as a
    /// `NonZero` makes `Option<Key>` the same size as `Key`.
    R::NonZero,
);

mod sealed {
    use crate::key_piece::KeyPiece;

    /// The index and generation types of a [`Packed`](super::Packed) key
    /// config. It is only implemented when `R` is `u8`, `u16`, `u32`, `u64`
    /// or `u128` and `GEN_BITS` is at least one and less than the bits of
    /// `R`, so any other `Packed` is not a key config.
    #[diagnostic::on_unimplemented(
        message = "`{Self}` is not a key config",
        label = "not a key config",
        note = "`Packed<R, GEN_BITS>` needs `R` to be `u8`, `u16`, `u32`, `u64` or `u128`, and `GEN_BITS` to be at least 1 and less than the bits of `R`"
    )]
    pub trait PackedParts {
        /// The smallest unsigned integer with at least as many bits as the
        /// index field.
        type Index: KeyPiece;

        /// The smallest unsigned integer with at least as many bits as the
        /// generation field.
        type Generation: KeyPiece;
    }
}

/// Hands `$callback` the table of every [`Packed`] key config. The table has
/// a block for each `R`. Each line in a block gives an index type and a
/// generation type, and then every `GEN_BITS` that gets those two types.
macro_rules! packed_table {
    ($callback:ident) => {
        $callback! {
            u8 {
                u8, u8 => 1 2 3 4 5 6 7;
            }
            u16 {
                u16, u8 => 1 2 3 4 5 6 7;
                u8, u8 => 8;
                u8, u16 => 9 10 11 12 13 14 15;
            }
            u32 {
                u32, u8 => 1 2 3 4 5 6 7 8;
                u32, u16 => 9 10 11 12 13 14 15;
                u16, u16 => 16;
                u16, u32 => 17 18 19 20 21 22 23;
                u8, u32 => 24 25 26 27 28 29 30 31;
            }
            u64 {
                u64, u8 => 1 2 3 4 5 6 7 8;
                u64, u16 => 9 10 11 12 13 14 15 16;
                u64, u32 => 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31;
                u32, u32 => 32;
                u32, u64 => 33 34 35 36 37 38 39 40 41 42 43 44 45 46 47;
                u16, u64 => 48 49 50 51 52 53 54 55;
                u8, u64 => 56 57 58 59 60 61 62 63;
            }
            u128 {
                u128, u8 => 1 2 3 4 5 6 7 8;
                u128, u16 => 9 10 11 12 13 14 15 16;
                u128, u32 => 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31 32;
                u128, u64 => 33 34 35 36 37 38 39 40 41 42 43 44 45 46 47 48 49 50 51 52 53
                    54 55 56 57 58 59 60 61 62 63;
                u64, u64 => 64;
                u64, u128 => 65 66 67 68 69 70 71 72 73 74 75 76 77 78 79 80 81 82 83 84 85
                    86 87 88 89 90 91 92 93 94 95;
                u32, u128 => 96 97 98 99 100 101 102 103 104 105 106 107 108 109 110 111;
                u16, u128 => 112 113 114 115 116 117 118 119;
                u8, u128 => 120 121 122 123 124 125 126 127;
            }
        }
    };
}

/// Implements `PackedParts` for every `Packed` key config in the table that
/// `packed_table` hands it.
macro_rules! impl_packed_parts {
    ($($r:ty { $($idx:ty, $gen:ty => $($gen_bits:literal)*;)* })*) => {
        $($($(
            impl sealed::PackedParts for Packed<$r, $gen_bits> {
                type Index = $idx;
                type Generation = $gen;
            }
        )*)*)*
    };
}

packed_table!(impl_packed_parts);

#[cfg(all(test, feature = "alloc"))]
pub(crate) use packed_table;

/// Fails to compile when `GEN_BITS` is zero or not less than the bits of
/// `R`, or when the index or generation type has fewer bits than its field.
/// Every `Packed` key config passes, so the check only guards the unsafe
/// code below against a wrong line in the table. The `const` block is
/// evaluated for each set of types it is used with.
#[inline(always)]
fn check_packed<Idx: KeyPiece, Gen: KeyPiece, R: KeyPiece, const GEN_BITS: u32>() {
    const {
        assert!(GEN_BITS >= 1, "Packed needs at least one generation bit");
        assert!(GEN_BITS < R::BITS, "Packed needs at least one index bit");
        assert!(
            Gen::BITS >= GEN_BITS,
            "the generation type of a Packed key config has fewer bits than GEN_BITS"
        );
        assert!(
            Idx::BITS >= R::BITS - GEN_BITS,
            "the index type of a Packed key config has fewer bits than the index field"
        );
    }
}

/// Returns a `u128` with its lowest `bits` bits set. `bits` must be below
/// 128.
#[inline]
fn low_bits(bits: u32) -> u128 {
    debug_assert!(bits < 128);
    (1u128 << bits) - 1
}

// SAFETY: the two parts are put in bit fields that do not overlap and are
// read back from the same fields. The field is private, so a `Packed` only
// comes from `pack_unchecked`, which only packs parts that fit, with an odd
// generation.
unsafe impl<R: KeyPiece, const GEN_BITS: u32> KeyConfig for Packed<R, GEN_BITS>
where
    Self: sealed::PackedParts,
{
    type Idx = <Self as sealed::PackedParts>::Index;
    type Gen = <Self as sealed::PackedParts>::Generation;

    #[inline]
    fn max_idx() -> Self::Idx {
        check_packed::<Self::Idx, Self::Gen, R, GEN_BITS>();
        // SAFETY: the check makes sure the index type has at least as many
        // bits as the index field.
        unsafe { Self::Idx::from_u128_unchecked(low_bits(R::BITS - GEN_BITS)) }
    }

    #[inline]
    fn max_generation() -> Odd<Self::Gen> {
        check_packed::<Self::Idx, Self::Gen, R, GEN_BITS>();
        // SAFETY: the check makes sure the generation type has at least
        // `GEN_BITS` bits and that `GEN_BITS` is at least one, so the value
        // with those bits set fits in the generation type and is odd.
        unsafe { Odd::new_unchecked(Self::Gen::from_u128_unchecked(low_bits(GEN_BITS))) }
    }

    #[inline]
    unsafe fn pack_unchecked(idx: Self::Idx, generation: Odd<Self::Gen>) -> Self {
        check_packed::<Self::Idx, Self::Gen, R, GEN_BITS>();
        debug_assert!(idx <= Self::max_idx());
        debug_assert!(generation <= Self::max_generation());
        let generation = Self::Gen::from_non_zero(generation.get());
        let bits = (idx.into_u128() << GEN_BITS) | generation.into_u128();
        // SAFETY: the caller promises that `idx` fits in the index field and
        // `generation` in the generation field, so `bits` fits in `R`, and
        // `generation` is odd, so `bits` is not zero.
        Self(unsafe { R::from_u128_unchecked(bits).into_non_zero_unchecked() })
    }

    #[inline]
    fn idx(self) -> Self::Idx {
        check_packed::<Self::Idx, Self::Gen, R, GEN_BITS>();
        // SAFETY: the index field has at most as many bits as the index
        // type, which the check makes sure of.
        unsafe { Self::Idx::from_u128_unchecked(R::from_non_zero(self.0).into_u128() >> GEN_BITS) }
    }

    #[inline]
    fn generation(self) -> Odd<Self::Gen> {
        check_packed::<Self::Idx, Self::Gen, R, GEN_BITS>();
        // SAFETY: the generation field has at most as many bits as the
        // generation type, which the check makes sure of. A `Packed` only
        // comes from `pack_unchecked`, which puts an odd generation in the
        // lowest bits, so the generation read here is odd.
        unsafe {
            Odd::new_unchecked(Self::Gen::from_u128_unchecked(
                R::from_non_zero(self.0).into_u128() & low_bits(GEN_BITS),
            ))
        }
    }
}
