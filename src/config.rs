use crate::dense::map::DenseMapSlot;
use crate::dense::secondary::DenseSecondaryMapSlot;
use crate::key::layout::Split;
use crate::key::parity::Odd;
use crate::key::piece::KeyPiece;
#[cfg(feature = "alloc")]
use crate::len::KeepLen;
use crate::len::Len;
#[cfg(feature = "alloc")]
use crate::map::MapIdx;
use crate::map::{MapKey, MapSlot};
#[cfg(feature = "alloc")]
use crate::secondary::replace_strategy::NewerWins;
use crate::secondary::replace_strategy::ReplaceStrategy;
use crate::secondary::SecondaryMapSlot;
use crate::storage::pair::PairStorage;
#[cfg(feature = "alloc")]
use crate::storage::pair_vec::PairVec;
#[cfg(feature = "alloc")]
use crate::storage::single_vec::SingleVec;
use crate::storage::SliceStorage;
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
/// against those limits. `idx` and `generation` must also return the same
/// parts every time they are called on the same value, because the maps check
/// a key's parts once and then read them again.
///
/// [`Key`](crate::Key) compares and hashes keys with the `PartialEq` and
/// `Hash` implementations of the key config. Those must only look at the
/// index and the generation, and two values with the same index and
/// generation must be equal and hash the same.
///
/// None of the methods of this trait may panic. The maps call them partway
/// through changes that a panic would leave half done, which can lead to
/// undefined behavior. The [`Clone`](core::clone::Clone) implementation of
/// the key config must behave like a copy, so it must not panic either.
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
    /// `Idx` is the type that represents the index of a slot. A key holds its
    /// index as an [`Idx::NonMax`](KeyPiece::NonMax), so the index is never the
    /// largest value of `Idx`. It can be [`u1`](crate::u1), `u8`, `u16`, `u32`,
    /// `u64`, `u128` or `usize`.
    type Idx: KeyPiece;

    /// `Gen` is the type that represents the generation of a slot. It can be
    /// [`u1`](crate::u1), `u8`, `u16`, `u32`, `u64`, `u128` or `usize`.
    type Gen: KeyPiece;

    /// The largest index a key can hold.
    fn max_idx() -> <Self::Idx as KeyPiece>::NonMax;

    /// The largest generation a key can hold.
    fn max_generation() -> Odd<Self::Gen>;

    /// Packs an index and a generation into a value of this key config.
    ///
    /// # Safety
    ///
    /// `idx` must be at most [`max_idx`](Self::max_idx), and `generation` at
    /// most [`max_generation`](Self::max_generation).
    unsafe fn pack_unchecked(
        idx: <Self::Idx as KeyPiece>::NonMax,
        generation: Odd<Self::Gen>,
    ) -> Self;

    /// Packs an index and a generation, or returns `None` if either is larger
    /// than the key config can hold.
    #[inline]
    fn pack(idx: <Self::Idx as KeyPiece>::NonMax, generation: Odd<Self::Gen>) -> Option<Self> {
        if idx <= Self::max_idx() && generation <= Self::max_generation() {
            // SAFETY: both parts were just checked to fit.
            Some(unsafe { Self::pack_unchecked(idx, generation) })
        } else {
            None
        }
    }

    /// Returns the index that was packed into this value.
    fn idx(self) -> <Self::Idx as KeyPiece>::NonMax;

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
/// use gen_map::{GenMap, GenMapConfig, KeepLen, MapConfig, MapSlot, Split};
///
/// /// Maps with this config hand out four byte keys, and when a slot's
/// /// generation runs out, the map wraps it back to zero instead of retiring
/// /// the slot.
/// struct Small;
///
/// impl MapConfig for Small {
///     type KeyConfig = Split<u16, u16>;
/// }
///
/// // `T` is the type of the map's values.
/// impl<T> GenMapConfig<T> for Small {
///     const WRAP_ON_OVERFLOW: bool = true;
///     type Len = KeepLen;
///     // `Storage` is the collection the slots live in, and
///     // `MapSlot<T, Self>` is the type of those slots.
///     type Storage = Vec<MapSlot<T, Self>>;
/// }
///
/// let mut map = GenMap::<&str, Small>::new_with_config();
/// let key = map.insert("hello");
/// assert_eq!(core::mem::size_of_val(&key), 4);
/// assert_eq!(map[key], "hello");
/// ```
pub trait MapConfig {
    /// The key config of the keys the map works with. Maps whose configs use
    /// the same key config share a key type. The built-in key configs are
    /// [`Split`] and [`Packed`](crate::Packed).
    type KeyConfig: KeyConfig;
}

/// This trait is used to choose what happens when a slot's generation runs
/// out, whether a [`GenMap`](crate::GenMap) keeps a count of its values, and
/// the collection its slots live in.
///
/// `T` is the type of the map's values. Implement this trait for every `T` to
/// let a map hold values of any type, or put bounds on `T` to restrict the
/// types of the values a map can hold, or even implement it for only a few specific
/// `T`s if desired.
///
/// [`MapConfig`] is a supertrait, and its key config is the key config of
/// the keys the map hands out. The [`MapConfig`] example shows a config that
/// implements this trait.
///
/// # Examples
///
/// A map with the config below can only hold values that are `Copy`.
///
/// ```
/// use gen_map::{GenMap, GenMapConfig, KeepLen, MapConfig, MapSlot, Split};
///
/// struct CopyOnly;
///
/// impl MapConfig for CopyOnly {
///     type KeyConfig = Split<u32, u32>;
/// }
///
/// impl<T: Copy> GenMapConfig<T> for CopyOnly {
///     type Len = KeepLen;
///     type Storage = Vec<MapSlot<T, Self>>;
/// }
///
/// let mut map = GenMap::<u8, CopyOnly>::new_with_config();
/// let key = map.insert(1);
/// assert_eq!(map[key], 1);
/// ```
pub trait GenMapConfig<T>: MapConfig {
    /// This decides what happens when a slot's generation runs out, which is
    /// when the map removes a value whose key has the largest generation its
    /// key config can hold.
    ///
    /// When it is `false`, the default, the map retires the slot and does not
    /// use it again until the map is reset, so no old key to the slot can
    /// match a new value.
    ///
    /// When it is `true`, the map wraps the generation back to zero and keeps
    /// using the slot. A key from before the wrap can then match a new value.
    const WRAP_ON_OVERFLOW: bool = false;

    /// This decides whether the map keeps a count of its values. The built-in
    /// choices are [`KeepLen`](crate::KeepLen) and [`NoLen`](crate::NoLen).
    type Len: Len;

    /// The collection the map keeps its slots in. The slot type is
    /// [`MapSlot<T, Self>`](crate::MapSlot). The built-in storages are
    /// [`SingleVec`](crate::SingleVec), `Vec`, `ArrayVec` and `SmallVec`.
    type Storage: SliceStorage<Item = MapSlot<T, Self>>;
}

/// This trait is used to choose the [`ReplaceStrategy`] of a
/// [`SecondaryMap`](crate::SecondaryMap), whether it keeps a count of its
/// values, and the collection its slots live in.
///
/// `T` is the type of the map's values. Implement this trait for every `T` to
/// let a map hold values of any type, or put bounds on `T` to restrict the
/// types of the values a map can hold, or even implement it for only a few specific
/// `T`s if desired.
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
///     DefaultKeyConfig, ExistingWins, GenMap, KeepLen, MapConfig, SecondaryMap,
///     SecondaryMapConfig, SecondaryMapSlot,
/// };
///
/// struct Keep;
///
/// impl MapConfig for Keep {
///     type KeyConfig = DefaultKeyConfig;
/// }
///
/// impl<T> SecondaryMapConfig<T> for Keep {
///     type ReplaceStrategy = ExistingWins;
///     type Len = KeepLen;
///     type Storage = Vec<SecondaryMapSlot<T, Self>>;
/// }
///
/// let mut people = GenMap::new();
/// let mut ages = SecondaryMap::<u32, Keep>::new_with_config();
/// let alice = people.insert("Alice");
/// ages.insert(alice, 30);
///
/// // Bob gets Alice's slot, but her age stays until it is removed.
/// people.remove(alice);
/// let bob = people.insert("Bob");
/// assert!(ages.try_insert(bob, 25).is_err());
/// assert_eq!(ages[alice], 30);
/// ```
pub trait SecondaryMapConfig<T>: MapConfig {
    /// Decides whether a value inserted under a key replaces the value
    /// already in the slot at the key's index, when the value in the slot was
    /// inserted under a different generation. The built-in strategies are
    /// [`NewerWins`](crate::NewerWins) and
    /// [`ExistingWins`](crate::ExistingWins).
    type ReplaceStrategy: ReplaceStrategy<Self::KeyConfig>;

    /// This decides whether the map keeps a count of its values. The built-in
    /// choices are [`KeepLen`](crate::KeepLen) and [`NoLen`](crate::NoLen).
    type Len: Len;

    /// The collection the map keeps its slots in. The slot type is
    /// [`SecondaryMapSlot<T, Self>`](crate::SecondaryMapSlot). The map keeps a
    /// slot at every index up to the highest index that an insert has used.
    /// The built-in storages are [`SingleVec`](crate::SingleVec), `Vec`,
    /// `ArrayVec` and `SmallVec`.
    type Storage: SliceStorage<Item = SecondaryMapSlot<T, Self>>;
}

/// This trait is used to choose what happens when a slot's generation runs
/// out, and the collections a [`DenseGenMap`](crate::DenseGenMap) keeps its
/// slots, values and keys in.
///
/// `T` is the type of the map's values. Implement this trait for every `T` to
/// let a map hold values of any type, or put bounds on `T` to restrict the
/// types of the values a map can hold, or even implement it for only a few specific
/// `T`s if desired.
///
/// [`MapConfig`] is a supertrait, and its key config is the key config of
/// the keys the map hands out.
///
/// # Examples
///
/// ```
/// use gen_map::{
///     DenseGenMap, DenseGenMapConfig, DenseMapSlot, MapConfig, MapKey, PairVec, Split,
/// };
///
/// /// Maps with this config hand out four byte keys.
/// struct Small;
///
/// impl MapConfig for Small {
///     type KeyConfig = Split<u16, u16>;
/// }
///
/// impl<T> DenseGenMapConfig<T> for Small {
///     type SlotStorage = Vec<DenseMapSlot<Self>>;
///     type PairStorage = PairVec<MapKey<Self>, T>;
/// }
///
/// let mut map = DenseGenMap::<&str, Small>::new_with_config();
/// let a = map.insert("a");
/// let b = map.insert("b");
/// map.remove(a);
/// assert_eq!(map[b], "b");
/// assert_eq!(core::mem::size_of_val(&b), 4);
/// ```
pub trait DenseGenMapConfig<T>: MapConfig {
    /// What happens when a slot's generation runs out. It works the same way
    /// as [`GenMapConfig::WRAP_ON_OVERFLOW`].
    const WRAP_ON_OVERFLOW: bool = false;

    /// The collection the map keeps its slots in. The slot type is
    /// [`DenseMapSlot<Self>`](crate::DenseMapSlot). The built-in storages are
    /// [`SingleVec`](crate::SingleVec), `Vec`, `ArrayVec` and `SmallVec`.
    type SlotStorage: SliceStorage<Item = DenseMapSlot<Self>>;

    /// The collection the map keeps its keys and values in. The key type is
    /// [`MapKey<Self>`](crate::MapKey), and the value type is `T`. The
    /// built-in pair storages are [`PairVec`](crate::PairVec) and
    /// [`SplitPair`](crate::SplitPair).
    ///
    /// # Examples
    ///
    /// ```
    /// use gen_map::{
    ///     DenseGenMap, DenseGenMapConfig, DenseMapSlot, MapConfig, MapKey, PairVec, Split,
    ///     SplitPair,
    /// };
    ///
    /// /// Maps with this config keep their keys and values in a `PairVec`.
    /// struct Paired;
    ///
    /// impl MapConfig for Paired {
    ///     type KeyConfig = Split<u32, u32>;
    /// }
    ///
    /// impl<T> DenseGenMapConfig<T> for Paired {
    ///     type SlotStorage = Vec<DenseMapSlot<Self>>;
    ///     type PairStorage = PairVec<MapKey<Self>, T>;
    /// }
    ///
    /// /// Maps with this config keep their keys in one `Vec` and their
    /// /// values in another.
    /// struct TwoVecs;
    ///
    /// impl MapConfig for TwoVecs {
    ///     type KeyConfig = Split<u32, u32>;
    /// }
    ///
    /// impl<T> DenseGenMapConfig<T> for TwoVecs {
    ///     type SlotStorage = Vec<DenseMapSlot<Self>>;
    ///     type PairStorage = SplitPair<Vec<MapKey<Self>>, Vec<T>>;
    /// }
    ///
    /// let mut paired = DenseGenMap::<&str, Paired>::new_with_config();
    /// let mut split = DenseGenMap::<&str, TwoVecs>::new_with_config();
    /// let a = paired.insert("a");
    /// let b = split.insert("b");
    /// assert_eq!(paired[a], "a");
    /// assert_eq!(split[b], "b");
    /// ```
    type PairStorage: PairStorage<First = MapKey<Self>, Second = T>;
}

/// This trait is used to choose the [`ReplaceStrategy`] of a
/// [`DenseSecondaryMap`](crate::DenseSecondaryMap), and the collections it
/// keeps its slots, values and keys in.
///
/// `T` is the type of the map's values. Implement this trait for every `T` to
/// let a map hold values of any type, or put bounds on `T` to restrict the
/// types of the values a map can hold, or even implement it for only a few specific
/// `T`s if desired.
///
/// [`MapConfig`] is a supertrait, and its key config is the key config of
/// the keys the map works with.
///
/// # Examples
///
/// ```
/// use gen_map::{
///     DefaultKeyConfig, DenseSecondaryMap, DenseSecondaryMapConfig, DenseSecondaryMapSlot,
///     ExistingWins, GenMap, MapConfig, MapKey, PairVec,
/// };
///
/// /// Maps with this config keep each value until it is removed.
/// struct Keep;
///
/// impl MapConfig for Keep {
///     type KeyConfig = DefaultKeyConfig;
/// }
///
/// impl<T> DenseSecondaryMapConfig<T> for Keep {
///     type ReplaceStrategy = ExistingWins;
///     type SlotStorage = Vec<DenseSecondaryMapSlot<Self>>;
///     type PairStorage = PairVec<MapKey<Self>, T>;
/// }
///
/// let mut people = GenMap::new();
/// let mut ages = DenseSecondaryMap::<u32, Keep>::new_with_config();
/// let alice = people.insert("Alice");
/// ages.insert(alice, 30);
///
/// // Bob gets Alice's slot, but her age stays until it is removed.
/// people.remove(alice);
/// let bob = people.insert("Bob");
/// assert!(ages.try_insert(bob, 25).is_err());
/// assert_eq!(ages[alice], 30);
/// ```
pub trait DenseSecondaryMapConfig<T>: MapConfig {
    /// Decides whether a value inserted under a key replaces the value
    /// already stored at the key's index, when that value was inserted under a
    /// different generation. It works the same way as
    /// [`SecondaryMapConfig::ReplaceStrategy`]. The built-in strategies are
    /// [`NewerWins`](crate::NewerWins) and
    /// [`ExistingWins`](crate::ExistingWins).
    type ReplaceStrategy: ReplaceStrategy<Self::KeyConfig>;

    /// The collection the map keeps its slots in. The slot type is
    /// [`DenseSecondaryMapSlot<Self>`](crate::DenseSecondaryMapSlot). The map
    /// keeps a slot at every index up to the highest index that an insert has
    /// used. The built-in storages are [`SingleVec`](crate::SingleVec), `Vec`,
    /// `ArrayVec` and `SmallVec`.
    type SlotStorage: SliceStorage<Item = DenseSecondaryMapSlot<Self>>;

    /// The collection the map keeps its keys and values in. The key type is
    /// [`MapKey<Self>`](crate::MapKey), and the value type is `T`. The
    /// built-in pair storages are [`PairVec`](crate::PairVec) and
    /// [`SplitPair`](crate::SplitPair).
    ///
    /// # Examples
    ///
    /// ```
    /// use gen_map::{
    ///     DefaultKeyConfig, DenseSecondaryMap, DenseSecondaryMapConfig, DenseSecondaryMapSlot,
    ///     GenMap, MapConfig, MapKey, NewerWins, PairVec, SplitPair,
    /// };
    ///
    /// /// Maps with this config keep their keys and values in a `PairVec`.
    /// struct Paired;
    ///
    /// impl MapConfig for Paired {
    ///     type KeyConfig = DefaultKeyConfig;
    /// }
    ///
    /// impl<T> DenseSecondaryMapConfig<T> for Paired {
    ///     type ReplaceStrategy = NewerWins;
    ///     type SlotStorage = Vec<DenseSecondaryMapSlot<Self>>;
    ///     type PairStorage = PairVec<MapKey<Self>, T>;
    /// }
    ///
    /// /// Maps with this config keep their keys in one `Vec` and their
    /// /// values in another.
    /// struct TwoVecs;
    ///
    /// impl MapConfig for TwoVecs {
    ///     type KeyConfig = DefaultKeyConfig;
    /// }
    ///
    /// impl<T> DenseSecondaryMapConfig<T> for TwoVecs {
    ///     type ReplaceStrategy = NewerWins;
    ///     type SlotStorage = Vec<DenseSecondaryMapSlot<Self>>;
    ///     type PairStorage = SplitPair<Vec<MapKey<Self>>, Vec<T>>;
    /// }
    ///
    /// let mut people = GenMap::new();
    /// let alice = people.insert("Alice");
    /// let mut ages = DenseSecondaryMap::<u32, Paired>::new_with_config();
    /// let mut heights = DenseSecondaryMap::<u32, TwoVecs>::new_with_config();
    /// ages.insert(alice, 30);
    /// heights.insert(alice, 165);
    /// assert_eq!(ages[alice], 30);
    /// assert_eq!(heights[alice], 165);
    /// ```
    type PairStorage: PairStorage<First = MapKey<Self>, Second = T>;
}

/// This trait is used to choose the [`ReplaceStrategy`] of a
/// [`SparseSecondaryMap`](crate::SparseSecondaryMap).
///
/// `T` is the type of the map's values. Implement this trait for every `T` to
/// let a map hold values of any type, or put bounds on `T` to restrict the
/// types of the values a map can hold, or even implement it for only a few specific
/// `T`s if desired.
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
/// impl<T> SparseSecondaryMapConfig<T> for Keep {
///     type ReplaceStrategy = ExistingWins;
/// }
///
/// let mut people = GenMap::new();
/// let mut ages = SparseSecondaryMap::<u32, Keep>::new_with_config();
/// let alice = people.insert("Alice");
/// ages.insert(alice, 30);
///
/// // Bob gets Alice's slot, but her age stays until it is removed.
/// people.remove(alice);
/// let bob = people.insert("Bob");
/// assert!(ages.try_insert(bob, 25).is_err());
/// assert_eq!(ages[alice], 30);
/// ```
#[cfg(feature = "std")]
#[cfg_attr(docsrs, doc(cfg(feature = "std")))]
pub trait SparseSecondaryMapConfig<T>: MapConfig {
    /// Decides whether a value inserted under a key replaces the value
    /// already stored at the key's index, when that value was inserted under a
    /// different generation. It works the same way as
    /// [`SecondaryMapConfig::ReplaceStrategy`]. The built-in strategies are
    /// [`NewerWins`](crate::NewerWins) and
    /// [`ExistingWins`](crate::ExistingWins).
    type ReplaceStrategy: ReplaceStrategy<Self::KeyConfig>;
}

/// A map cannot use a config that does not implement [`GenMapConfig`] for the
/// type of the map's values. The example below must fail to compile.
///
/// ```compile_fail,E0277
/// # use gen_map::{GenMap, GenMapConfig, KeepLen, MapConfig, MapSlot, Split};
/// # struct CopyOnly;
/// # impl MapConfig for CopyOnly {
/// #     type KeyConfig = Split<u32, u32>;
/// # }
/// # impl<T: Copy> GenMapConfig<T> for CopyOnly {
/// #     type Len = KeepLen;
/// #     type Storage = Vec<MapSlot<T, Self>>;
/// # }
/// let map = GenMap::<String, CopyOnly>::new_with_config();
/// ```
#[cfg(doctest)]
struct BoundedConfigRejectsOtherValues;

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
/// Keys use the [`DefaultKeyConfig`]. Every map except a `SparseSecondaryMap`
/// keeps its slots in a [`SingleVec`](crate::SingleVec), and the dense maps
/// keep their keys and values in a [`PairVec`](crate::PairVec). Both store
/// their length and capacity as a `u32`, the index type of the keys. Maps
/// with this config retire a slot whose generation runs out, and the
/// secondary maps use [`NewerWins`](crate::NewerWins). A `GenMap` and a
/// `SecondaryMap` with this config keep a count of their values. This config
/// needs the `alloc` feature.
#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct DefaultMapConfig;

#[cfg(feature = "alloc")]
impl MapConfig for DefaultMapConfig {
    type KeyConfig = DefaultKeyConfig;
}

#[cfg(feature = "alloc")]
impl<T> GenMapConfig<T> for DefaultMapConfig {
    type Len = KeepLen;
    type Storage = SingleVec<MapSlot<T, Self>, MapIdx<Self>>;
}

#[cfg(feature = "alloc")]
impl<T> SecondaryMapConfig<T> for DefaultMapConfig {
    type ReplaceStrategy = NewerWins;
    type Len = KeepLen;
    type Storage = SingleVec<SecondaryMapSlot<T, Self>, MapIdx<Self>>;
}

#[cfg(feature = "alloc")]
impl<T> DenseGenMapConfig<T> for DefaultMapConfig {
    type SlotStorage = SingleVec<DenseMapSlot<Self>, MapIdx<Self>>;
    type PairStorage = PairVec<MapKey<Self>, T, MapIdx<Self>>;
}

#[cfg(feature = "alloc")]
impl<T> DenseSecondaryMapConfig<T> for DefaultMapConfig {
    type ReplaceStrategy = NewerWins;
    type SlotStorage = SingleVec<DenseSecondaryMapSlot<Self>, MapIdx<Self>>;
    type PairStorage = PairVec<MapKey<Self>, T, MapIdx<Self>>;
}

#[cfg(feature = "std")]
impl<T> SparseSecondaryMapConfig<T> for DefaultMapConfig {
    type ReplaceStrategy = NewerWins;
}
