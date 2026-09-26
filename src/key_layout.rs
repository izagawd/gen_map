use crate::key_piece::KeyPiece;
use crate::parity::Odd;
use core::hash::Hash;
use core::marker::PhantomData;

/// How a [`Key`](crate::Key) stores its index and generation. A
/// [`KeyConfig`](crate::KeyConfig) picks one through its `Layout` type.
///
/// # Safety
///
/// The map trusts what a layout hands back. [`idx`](Self::idx) and
/// [`generation`](Self::generation) must return exactly what
/// [`pack_unchecked`](Self::pack_unchecked) was given, and two `Repr`
/// values must be equal only if they were packed from the same parts.
///
/// `generation` is safe to call and returns an [`Odd`], and
/// [`Key::from_repr`](crate::Key::from_repr) accepts any `Repr`, so safe code
/// must not be able to make a `Repr` that `pack_unchecked` did not return. A
/// `Repr` whose fields are private, like [`SplitRepr`] and [`PackedRepr`],
/// meets this rule, because only `pack_unchecked` can make one.
pub unsafe trait KeyLayout<Idx: KeyPiece, Gen: KeyPiece> {
    /// The type a key stores its index and generation in.
    type Repr: Copy + Eq + Hash + Send + Sync + 'static;

    /// The largest index a key can hold.
    fn max_idx() -> Idx;

    /// The largest generation a key can hold.
    fn max_generation() -> Odd<Gen>;

    /// Packs an index and a generation.
    ///
    /// # Safety
    ///
    /// `idx` must be at most [`max_idx`](Self::max_idx), and `generation` at
    /// most [`max_generation`](Self::max_generation).
    unsafe fn pack_unchecked(idx: Idx, generation: Odd<Gen>) -> Self::Repr;

    /// Packs an index and a generation, or returns `None` if either is larger
    /// than the layout can hold.
    #[inline]
    fn pack(idx: Idx, generation: Odd<Gen>) -> Option<Self::Repr> {
        if idx <= Self::max_idx() && generation <= Self::max_generation() {
            // SAFETY: both parts were just checked to fit.
            Some(unsafe { Self::pack_unchecked(idx, generation) })
        } else {
            None
        }
    }

    /// The index that was packed.
    fn idx(repr: Self::Repr) -> Idx;

    /// The generation that was packed.
    fn generation(repr: Self::Repr) -> Odd<Gen>;
}

/// Stores the index and the generation as two fields, so a key is as large
/// as the two put together, plus any padding their alignment needs. Every
/// value of `Idx` and `Gen` fits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Split;

/// The type a key with the [`Split`] layout stores its index and generation
/// in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SplitRepr<Idx: KeyPiece, Gen: KeyPiece> {
    idx: Idx,
    /// An odd generation is never zero, so storing it as an [`Odd`] gives
    /// `Option<Key>` the size of `Key`.
    generation: Odd<Gen>,
}

// SAFETY: the two fields are stored and read back as they are.
unsafe impl<Idx: KeyPiece, Gen: KeyPiece> KeyLayout<Idx, Gen> for Split {
    type Repr = SplitRepr<Idx, Gen>;

    #[inline]
    fn max_idx() -> Idx {
        Idx::MAX
    }

    #[inline]
    fn max_generation() -> Odd<Gen> {
        // SAFETY: `KeyPiece` promises that the largest value is odd.
        unsafe { Odd::new_unchecked(Gen::MAX) }
    }

    #[inline]
    unsafe fn pack_unchecked(idx: Idx, generation: Odd<Gen>) -> SplitRepr<Idx, Gen> {
        SplitRepr { idx, generation }
    }

    #[inline]
    fn idx(repr: SplitRepr<Idx, Gen>) -> Idx {
        repr.idx
    }

    #[inline]
    fn generation(repr: SplitRepr<Idx, Gen>) -> Odd<Gen> {
        repr.generation
    }
}

/// Stores the index and the generation in the bits of one integer `R`, so a
/// key is as large as `R`. The low `GEN_BITS` bits hold the generation and
/// the bits above them hold the index.
///
/// `GEN_BITS` must be at least one and less than the bits of `R`, `Gen` must
/// have at least `GEN_BITS` bits, and `Idx` must have at least the bits that
/// are left for the index. A key config that breaks one of these fails to
/// compile, but only in `cargo build` or `cargo test`, and only if a map
/// uses it. `cargo check` and some editors such as rust-analyzer may not report
/// it.
///
/// The largest index is `(1 << (R::BITS - GEN_BITS)) - 1` and the largest
/// generation is `(1 << GEN_BITS) - 1`, no matter how wide `Idx` and `Gen`
/// are. A slot whose generation reaches the largest one retires or wraps, as
/// [`WRAP_ON_OVERFLOW`](crate::MapConfig::WRAP_ON_OVERFLOW) says.
///
/// # Examples
///
/// ```
/// use gen_map::{GenMap, Key, KeyConfig, MapConfig, Packed, SlotItem};
///
/// /// Four byte keys with 24 bits of index and 8 bits of generation.
/// struct Compact;
///
/// impl KeyConfig for Compact {
///     type Idx = u32;
///     type Gen = u8;
///     type Layout = Packed<u32, 8>;
/// }
///
/// impl<S: SlotItem> MapConfig<S> for Compact {
///     type KeyConfig = Self;
///     type Storage = Vec<S>;
/// }
///
/// let mut map = GenMap::<&str, Compact>::new_with_config();
/// let key = map.insert("a");
/// assert_eq!(core::mem::size_of_val(&key), 4);
/// assert_eq!(core::mem::size_of::<Option<Key<Compact>>>(), 4);
/// assert_eq!(key.idx(), 0);
/// // `generation` returns an `Odd<u8>`. Its `get` returns a `NonZero<u8>`,
/// // and that type's `get` returns the `u8`.
/// assert_eq!(key.generation().get().get(), 1);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Packed<R, const GEN_BITS: u32>(PhantomData<R>);

/// The type a key with the [`Packed`] layout stores its index and
/// generation in. Only
/// [`pack_unchecked`](KeyLayout::pack_unchecked) can make one, so its
/// generation field is never zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PackedRepr<R: KeyPiece>(R::NonZero);

/// Fails to compile when the bit counts of a [`Packed`] layout do not add
/// up. The `const` block is evaluated for each set of types it is used with.
#[inline(always)]
fn check_packed<Idx: KeyPiece, Gen: KeyPiece, R: KeyPiece, const GEN_BITS: u32>() {
    const {
        assert!(GEN_BITS >= 1, "Packed needs at least one generation bit");
        assert!(GEN_BITS < R::BITS, "Packed needs at least one index bit");
        assert!(
            Gen::BITS >= GEN_BITS,
            "the Gen type of the key config has fewer bits than GEN_BITS"
        );
        assert!(
            Idx::BITS >= R::BITS - GEN_BITS,
            "the Idx type of the key config has fewer bits than the index field"
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
// read back from the same fields.
unsafe impl<Idx: KeyPiece, Gen: KeyPiece, R: KeyPiece, const GEN_BITS: u32> KeyLayout<Idx, Gen>
    for Packed<R, GEN_BITS>
{
    type Repr = PackedRepr<R>;

    #[inline]
    fn max_idx() -> Idx {
        check_packed::<Idx, Gen, R, GEN_BITS>();
        // SAFETY: the check makes sure `Idx` has at least as many bits as the
        // index field.
        unsafe { Idx::from_u128_unchecked(low_bits(R::BITS - GEN_BITS)) }
    }

    #[inline]
    fn max_generation() -> Odd<Gen> {
        check_packed::<Idx, Gen, R, GEN_BITS>();
        // SAFETY: the check makes sure `Gen` has at least `GEN_BITS` bits and
        // that `GEN_BITS` is at least one, so the value with those bits set
        // fits in `Gen` and is odd.
        unsafe { Odd::new_unchecked(Gen::from_u128_unchecked(low_bits(GEN_BITS))) }
    }

    #[inline]
    unsafe fn pack_unchecked(idx: Idx, generation: Odd<Gen>) -> PackedRepr<R> {
        check_packed::<Idx, Gen, R, GEN_BITS>();
        debug_assert!(idx <= <Self as KeyLayout<Idx, Gen>>::max_idx());
        debug_assert!(generation <= <Self as KeyLayout<Idx, Gen>>::max_generation());
        let generation = Gen::from_non_zero(generation.get());
        let bits = (idx.into_u128() << GEN_BITS) | generation.into_u128();
        // SAFETY: the caller promises that `idx` fits in the index field and
        // `generation` in the generation field, so `bits` fits in `R`, and
        // `generation` is odd, so `bits` is not zero.
        PackedRepr(unsafe { R::from_u128_unchecked(bits).into_non_zero_unchecked() })
    }

    #[inline]
    fn idx(repr: PackedRepr<R>) -> Idx {
        check_packed::<Idx, Gen, R, GEN_BITS>();
        // SAFETY: the index field has at most as many bits as `Idx`, which
        // the check makes sure of.
        unsafe { Idx::from_u128_unchecked(R::from_non_zero(repr.0).into_u128() >> GEN_BITS) }
    }

    #[inline]
    fn generation(repr: PackedRepr<R>) -> Odd<Gen> {
        check_packed::<Idx, Gen, R, GEN_BITS>();
        // SAFETY: the generation field has at most as many bits as `Gen`,
        // which the check makes sure of. A `PackedRepr` only comes from
        // `pack_unchecked`, which was given an odd generation.
        unsafe {
            Odd::new_unchecked(Gen::from_u128_unchecked(
                R::from_non_zero(repr.0).into_u128() & low_bits(GEN_BITS),
            ))
        }
    }
}
