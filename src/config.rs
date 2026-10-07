use crate::key_layout::Split;
use crate::key_piece::KeyPiece;
use crate::pair_storage::PairStorage;
#[cfg(feature = "alloc")]
use crate::pair_vec::PairVec;
use crate::parity::Odd;
#[cfg(feature = "alloc")]
use crate::replace_strategy::NewerWins;
use crate::replace_strategy::ReplaceStrategy;
use crate::slot::{GenSlotItem, SecondarySlotItem};
use crate::storage::SliceStorage;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;
use core::hash::Hash;

/// Chooses the index and generation types of a [`Key`](crate::Key), and how
/// the key stores the two. A `Key<K>` holds a value of its key config `K`,
/// and that value holds the index and the generation.
///
/// [`Split<Idx, Gen>`](crate::Split) keeps the index and the generation as
/// two fields, and [`Packed<R, GEN_BITS>`](crate::Packed) puts them in the
/// bits of one integer. Either one can be the
/// [`KeyConfig`](MapConfig::KeyConfig) of a [`MapConfig`].
///
/// # Examples
///
/// ```
/// use gen_map::{Key, Packed, Split};
///
/// // These keys keep a `u16` index and a `u16` generation as two fields.
/// assert_eq!(core::mem::size_of::<Key<Split<u16, u16>>>(), 4);
///
/// // These keys keep 24 bits of index and 8 bits of generation in one `u32`.
/// assert_eq!(core::mem::size_of::<Key<Packed<u32, 8>>>(), 4);
/// ```
///
/// # Safety
///
/// The maps' unsafe code relies on what a key config returns. For a value that
/// [`pack_unchecked`](Self::pack_unchecked) made, [`idx`](Self::idx) and
/// [`generation`](Self::generation) must return exactly the index and the
/// generation that `pack_unchecked` was given, and two values must be equal
/// only if they unpack to the same parts. [`max_idx`](Self::max_idx) and
/// [`max_generation`](Self::max_generation) must return the same value every
/// time, because the maps pack parts again long after they first checked them
/// against those limits.
///
/// Safe code can make a key from any value of the key config it can build,
/// with [`Key::from_repr`](crate::Key::from_repr), and read the key's parts
/// through the safe [`idx`](Self::idx) and [`generation`](Self::generation)
/// methods. So every value that safe code can build must unpack to an odd
/// generation, since `generation` returns an [`Odd`]. It must also unpack to
/// an index of at most `max_idx` and a generation of at most
/// `max_generation`. A key config whose fields are private and whose values
/// only come from `pack_unchecked`, like [`Split`](crate::Split) and
/// [`Packed`](crate::Packed), meets these rules.
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a key config",
    label = "not a key config",
    note = "`Split<Idx, Gen>` and `Packed<R, GEN_BITS>` are key configs. `Packed` needs `R` to be `u8`, `u16`, `u32`, `u64` or `u128`, and `GEN_BITS` to be at least 1 and less than the bits of `R`"
)]
pub unsafe trait KeyConfig: Copy + Eq + Hash + Send + Sync + 'static {
    /// The integer type that represents the index of a slot. No map gives a
    /// slot the largest value of this type.
    type Idx: KeyPiece;

    /// The integer type that represents the generation of a slot.
    type Gen: KeyPiece;

    /// The largest index a key can hold.
    fn max_idx() -> Self::Idx;

    /// The largest generation a key can hold.
    fn max_generation() -> Odd<Self::Gen>;

    /// Packs an index and a generation into a value of this key config.
    ///
    /// # Safety
    ///
    /// `idx` must be at most [`max_idx`](Self::max_idx), and `generation` at
    /// most [`max_generation`](Self::max_generation).
    unsafe fn pack_unchecked(idx: Self::Idx, generation: Odd<Self::Gen>) -> Self;

    /// Packs an index and a generation, or returns `None` if either is larger
    /// than the key config can hold.
    #[inline]
    fn pack(idx: Self::Idx, generation: Odd<Self::Gen>) -> Option<Self> {
        if idx <= Self::max_idx() && generation <= Self::max_generation() {
            // SAFETY: both parts were just checked to fit.
            Some(unsafe { Self::pack_unchecked(idx, generation) })
        } else {
            None
        }
    }

    /// Returns the index that was packed into this value.
    fn idx(self) -> Self::Idx;

    /// Returns the generation that was packed into this value.
    fn generation(self) -> Odd<Self::Gen>;
}

/// Chooses the [`KeyConfig`] of the keys a map works with.
///
/// The config of a [`GenMap`](crate::GenMap) also implements
/// [`GenMapConfig`], and the config of a
/// [`SecondaryMap`](crate::SecondaryMap) also implements
/// [`SecondaryMapConfig`]. The configs of the dense maps implement
/// [`DenseGenMapConfig`] and [`DenseSecondaryMapConfig`], and the config of a
/// [`SparseSecondaryMap`](crate::SparseSecondaryMap) implements
/// [`SparseSecondaryMapConfig`]. All five traits have `MapConfig` as a
/// supertrait.
///
/// # Examples
///
/// ```
/// use gen_map::{GenMap, GenMapConfig, GenSlotItem, MapConfig, Split};
///
/// /// Maps with this config hand out four byte keys, and a slot whose
/// /// generation runs out wraps instead of retiring.
/// struct Small;
///
/// impl MapConfig for Small {
///     type KeyConfig = Split<u16, u16>;
/// }
///
/// impl GenMapConfig for Small {
///     const WRAP_ON_OVERFLOW: bool = true;
///     // `Storage` is the collection the slots live in. `S` is the slot type.
///     type Storage<S: GenSlotItem> = Vec<S>;
/// }
///
/// let mut map = GenMap::<&str, Small>::new_with_config();
/// let key = map.insert("hello");
/// assert_eq!(core::mem::size_of_val(&key), 4);
/// assert_eq!(map[key], "hello");
/// ```
pub trait MapConfig {
    /// The key config of the keys the map works with. Maps whose configs use
    /// the same key config share a key type.
    type KeyConfig: KeyConfig;
}

/// This trait is used to choose what happens when a slot's generation runs
/// out, and the collection the slots of a [`GenMap`](crate::GenMap) live in.
///
/// [`MapConfig`] is a supertrait, and its key config is the key config of
/// the keys the map hands out. The [`MapConfig`] example shows a config that
/// implements this trait.
pub trait GenMapConfig: MapConfig {
    /// What happens when a slot's generation runs out, meaning it reaches
    /// the largest one its key can hold.
    ///
    /// `false`, the default, retires the slot. It is never used again, so no
    /// stale key can ever match a new value.
    ///
    /// `true` wraps the generation back to zero and keeps using the slot. A
    /// stale key from before the wrap can then match a new value.
    const WRAP_ON_OVERFLOW: bool = false;

    /// The collection the map keeps its slots in. `S` is the slot type, which
    /// is [`MapSlot<T, C>`](crate::MapSlot) for a `GenMap<T, C>`.
    type Storage<S: GenSlotItem>: SliceStorage<Item = S>;
}

/// This trait is used to choose the [`ReplaceStrategy`] of a
/// [`SecondaryMap`](crate::SecondaryMap), and the collection its slots live
/// in.
///
/// [`MapConfig`] is a supertrait, and its key config is the key config of
/// the keys the map works with.
///
/// # Examples
///
/// A map with the config below keeps each value until it is removed, even
/// when a value is inserted under a newer key for the same slot.
///
/// ```
/// use gen_map::{
///     DefaultKeyConfig, ExistingWins, GenMap, MapConfig, SecondaryMap, SecondaryMapConfig,
///     SecondarySlotItem,
/// };
///
/// struct Keep;
///
/// impl MapConfig for Keep {
///     type KeyConfig = DefaultKeyConfig;
/// }
///
/// impl SecondaryMapConfig for Keep {
///     type ReplaceStrategy = ExistingWins;
///     type Storage<S: SecondarySlotItem> = Vec<S>;
/// }
///
/// let mut people = GenMap::new();
/// let mut ages = SecondaryMap::<u32, Keep>::new_with_config();
/// let alice = people.insert("Alice");
/// ages.insert(alice, 30).unwrap();
///
/// // Bob gets Alice's slot, but her age stays until it is removed.
/// people.remove(alice);
/// let bob = people.insert("Bob");
/// assert!(ages.insert(bob, 25).is_err());
/// assert_eq!(ages[alice], 30);
/// ```
pub trait SecondaryMapConfig: MapConfig {
    /// Decides whether a value inserted under a key replaces the value
    /// already in the slot at the key's index, when the value in the slot was
    /// inserted under a different generation. The built-in strategies are
    /// [`NewerWins`](crate::NewerWins) and
    /// [`ExistingWins`](crate::ExistingWins).
    type ReplaceStrategy: ReplaceStrategy<Self::KeyConfig>;

    /// The collection the map keeps its slots in. `S` is the slot type, which
    /// is [`SecondaryMapSlot<T, C>`](crate::SecondaryMapSlot) for a
    /// `SecondaryMap<T, C>`. The map keeps a slot at every index up to the
    /// highest index that an insert has used. With
    /// `type Storage<S: SecondarySlotItem> = Vec<S>;`, the map keeps them in a
    /// `Vec`, and any other [`SliceStorage`] works too, such as the `ArrayVec`
    /// and `SmallVec` a [`GenMapConfig`] can use.
    type Storage<S: SecondarySlotItem>: SliceStorage<Item = S>;
}

/// This trait is used to choose what happens when a slot's generation runs
/// out, and the collections a [`DenseGenMap`](crate::DenseGenMap) keeps its
/// slots, values and keys in.
///
/// [`MapConfig`] is a supertrait, and its key config is the key config of
/// the keys the map hands out.
///
/// # Examples
///
/// ```
/// use gen_map::{DenseGenMap, DenseGenMapConfig, GenSlotItem, MapConfig, PairVec, Split};
///
/// /// Maps with this config hand out four byte keys.
/// struct Small;
///
/// impl MapConfig for Small {
///     type KeyConfig = Split<u16, u16>;
/// }
///
/// impl DenseGenMapConfig for Small {
///     type SlotStorage<S: GenSlotItem> = Vec<S>;
///     type PairStorage<K, V> = PairVec<K, V>;
/// }
///
/// let mut map = DenseGenMap::<&str, Small>::new_with_config();
/// let a = map.insert("a");
/// let b = map.insert("b");
/// map.remove(a);
/// assert_eq!(map[b], "b");
/// assert_eq!(core::mem::size_of_val(&b), 4);
/// ```
pub trait DenseGenMapConfig: MapConfig {
    /// What happens when a slot's generation runs out. It works the same way
    /// as [`GenMapConfig::WRAP_ON_OVERFLOW`].
    const WRAP_ON_OVERFLOW: bool = false;

    /// The collection the map keeps its slots in. `S` is the slot type, which
    /// is [`DenseMapSlot<C>`](crate::DenseMapSlot) for a `DenseGenMap<T, C>`.
    type SlotStorage<S: GenSlotItem>: SliceStorage<Item = S>;

    /// The collection the map keeps its keys and values in. The first
    /// parameter is the key type, and the second is the value type.
    ///
    /// # Examples
    ///
    /// ```
    /// use gen_map::{
    ///     DenseGenMap, DenseGenMapConfig, GenSlotItem, MapConfig, PairVec, Split, SplitPair,
    /// };
    ///
    /// /// Maps with this config keep their keys and values in a `PairVec`.
    /// struct Paired;
    ///
    /// impl MapConfig for Paired {
    ///     type KeyConfig = Split<u32, u32>;
    /// }
    ///
    /// impl DenseGenMapConfig for Paired {
    ///     type SlotStorage<S: GenSlotItem> = Vec<S>;
    ///     type PairStorage<K, V> = PairVec<K, V>;
    /// }
    ///
    /// /// Maps with this config keep their keys in one `Vec` and their values in
    /// /// another.
    /// struct TwoVecs;
    ///
    /// impl MapConfig for TwoVecs {
    ///     type KeyConfig = Split<u32, u32>;
    /// }
    ///
    /// impl DenseGenMapConfig for TwoVecs {
    ///     type SlotStorage<S: GenSlotItem> = Vec<S>;
    ///     type PairStorage<K, V> = SplitPair<Vec<K>, Vec<V>>;
    /// }
    ///
    /// let mut paired = DenseGenMap::<&str, Paired>::new_with_config();
    /// let mut split = DenseGenMap::<&str, TwoVecs>::new_with_config();
    /// let a = paired.insert("a");
    /// let b = split.insert("b");
    /// assert_eq!(paired[a], "a");
    /// assert_eq!(split[b], "b");
    /// ```
    type PairStorage<K, V>: PairStorage<First = K, Second = V>;
}

/// This trait is used to choose the [`ReplaceStrategy`] of a
/// [`DenseSecondaryMap`](crate::DenseSecondaryMap), and the collections it
/// keeps its slots, values and keys in.
///
/// [`MapConfig`] is a supertrait, and its key config is the key config of
/// the keys the map works with.
///
/// # Examples
///
/// ```
/// use gen_map::{
///     DefaultKeyConfig, DenseSecondaryMap, DenseSecondaryMapConfig, ExistingWins, GenMap,
///     GenSlotItem, MapConfig, PairVec,
/// };
///
/// /// Maps with this config keep each value until it is removed.
/// struct Keep;
///
/// impl MapConfig for Keep {
///     type KeyConfig = DefaultKeyConfig;
/// }
///
/// impl DenseSecondaryMapConfig for Keep {
///     type ReplaceStrategy = ExistingWins;
///     type SlotStorage<S: GenSlotItem> = Vec<S>;
///     type PairStorage<K, V> = PairVec<K, V>;
/// }
///
/// let mut people = GenMap::new();
/// let mut ages = DenseSecondaryMap::<u32, Keep>::new_with_config();
/// let alice = people.insert("Alice");
/// ages.insert(alice, 30).unwrap();
///
/// // Bob gets Alice's slot, but her age stays until it is removed.
/// people.remove(alice);
/// let bob = people.insert("Bob");
/// assert!(ages.insert(bob, 25).is_err());
/// assert_eq!(ages[alice], 30);
/// ```
pub trait DenseSecondaryMapConfig: MapConfig {
    /// Decides whether a value inserted under a key replaces the value
    /// already stored at the key's index, when that value was inserted under a
    /// different generation. It works the same way as
    /// [`SecondaryMapConfig::ReplaceStrategy`].
    type ReplaceStrategy: ReplaceStrategy<Self::KeyConfig>;

    /// The collection the map keeps its slots in. `S` is the slot type, which
    /// is [`DenseSecondaryMapSlot<C>`](crate::DenseSecondaryMapSlot) for a
    /// `DenseSecondaryMap<T, C>`. The map keeps a slot at every index up to the
    /// highest index that an insert has used.
    type SlotStorage<S: GenSlotItem>: SliceStorage<Item = S>;

    /// The collection the map keeps its keys and values in. The first
    /// parameter is the key type, and the second is the value type.
    ///
    /// # Examples
    ///
    /// ```
    /// use gen_map::{
    ///     DefaultKeyConfig, DenseSecondaryMap, DenseSecondaryMapConfig, GenMap, GenSlotItem,
    ///     MapConfig, NewerWins, PairVec, SplitPair,
    /// };
    ///
    /// /// Maps with this config keep their keys and values in a `PairVec`.
    /// struct Paired;
    ///
    /// impl MapConfig for Paired {
    ///     type KeyConfig = DefaultKeyConfig;
    /// }
    ///
    /// impl DenseSecondaryMapConfig for Paired {
    ///     type ReplaceStrategy = NewerWins;
    ///     type SlotStorage<S: GenSlotItem> = Vec<S>;
    ///     type PairStorage<K, V> = PairVec<K, V>;
    /// }
    ///
    /// /// Maps with this config keep their keys in one `Vec` and their values in
    /// /// another.
    /// struct TwoVecs;
    ///
    /// impl MapConfig for TwoVecs {
    ///     type KeyConfig = DefaultKeyConfig;
    /// }
    ///
    /// impl DenseSecondaryMapConfig for TwoVecs {
    ///     type ReplaceStrategy = NewerWins;
    ///     type SlotStorage<S: GenSlotItem> = Vec<S>;
    ///     type PairStorage<K, V> = SplitPair<Vec<K>, Vec<V>>;
    /// }
    ///
    /// let mut people = GenMap::new();
    /// let alice = people.insert("Alice");
    /// let mut ages = DenseSecondaryMap::<u32, Paired>::new_with_config();
    /// let mut heights = DenseSecondaryMap::<u32, TwoVecs>::new_with_config();
    /// ages.insert(alice, 30).unwrap();
    /// heights.insert(alice, 165).unwrap();
    /// assert_eq!(ages[alice], 30);
    /// assert_eq!(heights[alice], 165);
    /// ```
    type PairStorage<K, V>: PairStorage<First = K, Second = V>;
}

/// This trait is used to choose the [`ReplaceStrategy`] of a
/// [`SparseSecondaryMap`](crate::SparseSecondaryMap).
///
/// [`MapConfig`] is a supertrait, and its key config is the key config of
/// the keys the map works with. The map keeps its values in a `HashMap`, so
/// the config does not pick a storage.
///
/// # Examples
///
/// A map with the config below keeps each value until it is removed, even
/// when a value is inserted under a newer key for the same index.
///
/// ```
/// use gen_map::{
///     DefaultKeyConfig, ExistingWins, GenMap, MapConfig, SparseSecondaryMap,
///     SparseSecondaryMapConfig,
/// };
///
/// struct Keep;
///
/// impl MapConfig for Keep {
///     type KeyConfig = DefaultKeyConfig;
/// }
///
/// impl SparseSecondaryMapConfig for Keep {
///     type ReplaceStrategy = ExistingWins;
/// }
///
/// let mut people = GenMap::new();
/// let mut ages = SparseSecondaryMap::<u32, Keep>::new_with_config();
/// let alice = people.insert("Alice");
/// ages.insert(alice, 30).unwrap();
///
/// // Bob gets Alice's slot, but her age stays until it is removed.
/// people.remove(alice);
/// let bob = people.insert("Bob");
/// assert!(ages.insert(bob, 25).is_err());
/// assert_eq!(ages[alice], 30);
/// ```
#[cfg(feature = "std")]
#[cfg_attr(docsrs, doc(cfg(feature = "std")))]
pub trait SparseSecondaryMapConfig: MapConfig {
    /// Decides whether a value inserted under a key replaces the value
    /// already stored at the key's index, when that value was inserted under a
    /// different generation. It works the same way as
    /// [`SecondaryMapConfig::ReplaceStrategy`].
    type ReplaceStrategy: ReplaceStrategy<Self::KeyConfig>;
}

/// The key config a [`Key`](crate::Key) uses when its `K` parameter is left
/// out.
///
/// Keys are a `u32` index and a `u32` generation stored as two fields.
pub type DefaultKeyConfig = Split<u32, u32>;

/// The config of a [`GenMap<T>`](crate::GenMap), a
/// [`SecondaryMap<T>`](crate::SecondaryMap), a
/// [`DenseGenMap<T>`](crate::DenseGenMap), a
/// [`DenseSecondaryMap<T>`](crate::DenseSecondaryMap) and a
/// [`SparseSecondaryMap<T>`](crate::SparseSecondaryMap), which leave out
/// their config parameter `C`.
///
/// Keys use the [`DefaultKeyConfig`], and slots live in a `Vec`. The dense
/// maps keep their keys and values in a [`PairVec`](crate::PairVec), and a
/// `SparseSecondaryMap` keeps its values in a `HashMap`. A slot retires when
/// its generation runs out. The secondary maps use
/// [`NewerWins`](crate::NewerWins) to decide whether an insert replaces a
/// value that was inserted under a different generation.
/// This config needs the `alloc` feature.
#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct DefaultMapConfig;

#[cfg(feature = "alloc")]
impl MapConfig for DefaultMapConfig {
    type KeyConfig = DefaultKeyConfig;
}

#[cfg(feature = "alloc")]
impl GenMapConfig for DefaultMapConfig {
    type Storage<S: GenSlotItem> = Vec<S>;
}

#[cfg(feature = "alloc")]
impl SecondaryMapConfig for DefaultMapConfig {
    type ReplaceStrategy = NewerWins;
    type Storage<S: SecondarySlotItem> = Vec<S>;
}

#[cfg(feature = "alloc")]
impl DenseGenMapConfig for DefaultMapConfig {
    type SlotStorage<S: GenSlotItem> = Vec<S>;
    type PairStorage<K, V> = PairVec<K, V>;
}

#[cfg(feature = "alloc")]
impl DenseSecondaryMapConfig for DefaultMapConfig {
    type ReplaceStrategy = NewerWins;
    type SlotStorage<S: GenSlotItem> = Vec<S>;
    type PairStorage<K, V> = PairVec<K, V>;
}

#[cfg(feature = "std")]
impl SparseSecondaryMapConfig for DefaultMapConfig {
    type ReplaceStrategy = NewerWins;
}
