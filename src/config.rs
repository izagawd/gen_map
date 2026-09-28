use crate::key_layout::Split;
use crate::key_piece::KeyPiece;
use crate::map::MapSlot;
use crate::parity::Odd;
#[cfg(feature = "alloc")]
use crate::replace_strategy::NewerWins;
use crate::replace_strategy::ReplaceStrategy;
use crate::secondary_map::SecondaryMapSlot;
use crate::slot::{GenSlotItem, SecondarySlotItem};
use crate::storage::SlotStorage;
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
/// The map trusts what a key config hands back. For a value that
/// [`pack_unchecked`](Self::pack_unchecked) made, [`idx`](Self::idx) and
/// [`generation`](Self::generation) must return exactly the index and the
/// generation that `pack_unchecked` was given, and two values must be equal
/// only if they unpack to the same parts. [`max_idx`](Self::max_idx) and
/// [`max_generation`](Self::max_generation) must return the same value every
/// time, because the maps pack parts again long after they first checked
/// them against those limits.
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
    /// The integer type that represents the index of a slot. A
    /// [`GenMap`](crate::GenMap) never gives a slot the largest value of
    /// this type.
    type Idx: KeyPiece;

    /// The integer type that represents the generation of a slot.
    type Gen: KeyPiece;

    /// The largest index a key can hold.
    fn max_idx() -> Self::Idx;

    /// The largest generation a key can hold.
    fn max_generation() -> Odd<Self::Gen>;

    /// Packs an index and a generation.
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

    /// The index that was packed.
    fn idx(self) -> Self::Idx;

    /// The generation that was packed.
    fn generation(self) -> Odd<Self::Gen>;
}

/// Chooses the [`KeyConfig`] of the keys a map works with.
///
/// The config of a [`GenMap`](crate::GenMap) also implements
/// [`GenMapConfig`], which has `MapConfig` as a supertrait.
///
/// # Examples
///
/// ```
/// use gen_map::{GenMap, GenMapConfig, GenSlotItem, MapConfig, Split};
///
/// /// Four byte keys, and a slot whose generation runs out wraps instead
/// /// of retiring.
/// struct Small;
///
/// impl MapConfig for Small {
///     type KeyConfig = Split<u16, u16>;
/// }
///
/// impl<S: GenSlotItem> GenMapConfig<S> for Small {
///     const WRAP_ON_OVERFLOW: bool = true;
///     // The collection the slots live in.
///     type Storage = Vec<S>;
/// }
///
/// let mut map = GenMap::<&str, Small>::new_with_config();
/// let key = map.insert("hello");
/// assert_eq!(core::mem::size_of_val(&key), 4);
/// assert_eq!(map[key], "hello");
/// ```
pub trait MapConfig {
    /// The config of the keys the map hands out. Maps whose configs use the
    /// same key config share a key type.
    type KeyConfig: KeyConfig;
}

/// This trait is used to choose what happens when a slot's generation runs
/// out, and the collection the slots of a [`GenMap`](crate::GenMap) live in.
///
/// `S` is the slot type, the [`Slot`](crate::Slot) the map keeps each value
/// in, and `S::Value` is the type of that value. A config is usually
/// implemented for every slot type at once, with `impl<S: GenSlotItem>
/// GenMapConfig<S> for YourConfig`, as in the [`MapConfig`] example. A
/// `GenMap<T, C>` then uses the impl for its own slots,
/// [`MapSlot<T, C>`](crate::MapSlot).
///
/// [`MapConfig`] is a supertrait, and its key config is the key config of
/// the keys the map hands out. The impl can put bounds on `S::Value` or on
/// `S` to limit which maps can use the config.
/// [`GenSlotItem`](GenSlotItem#bounds-on-the-value-and-the-slot) has three
/// examples of these bounds.
pub trait GenMapConfig<S: GenSlotItem>: MapConfig {
    /// What happens when a slot's generation runs out, meaning it reaches
    /// the largest one its key can hold.
    ///
    /// `false`, the default, retires the slot. It is never used again, so no
    /// stale key can ever match a new value.
    ///
    /// `true` wraps the generation back to zero and keeps using the slot. A
    /// stale key from before the wrap can then match a new value.
    const WRAP_ON_OVERFLOW: bool = false;

    /// The collection the map keeps its slots in.
    type Storage: SlotStorage<Item = S>;
}

/// This trait is used to choose the [`ReplaceStrategy`] of a
/// [`SecondaryMap`](crate::SecondaryMap), and the collection its slots live
/// in.
///
/// `S` is the slot type, the [`SecondarySlot`](crate::SecondarySlot) the map
/// keeps each value in, and `S::Value` is the type of that value. A config is
/// usually implemented for every slot type at once, with
/// `impl<S: SecondarySlotItem> SecondaryMapConfig<S> for YourConfig`, as in
/// the example below. A `SecondaryMap<T, C>` then uses the impl for its own
/// slots, [`SecondaryMapSlot<T, C>`](crate::SecondaryMapSlot).
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
/// impl<S: SecondarySlotItem> SecondaryMapConfig<S> for Keep {
///     type ReplaceStrategy = ExistingWins;
///     type Storage = Vec<S>;
/// }
///
/// let mut names = GenMap::new();
/// let mut ages = SecondaryMap::<u32, Keep>::new_with_config();
/// let alice = names.insert("Alice");
/// ages.insert(alice, 30).unwrap();
///
/// // Bob gets Alice's slot, but her age stays until it is removed.
/// names.remove(alice);
/// let bob = names.insert("Bob");
/// assert!(ages.insert(bob, 25).is_err());
/// assert_eq!(ages[alice], 30);
/// ```
pub trait SecondaryMapConfig<S: SecondarySlotItem>: MapConfig {
    /// Decides whether a value inserted under a key replaces the value
    /// already in the slot at the key's index, when the value in the slot was
    /// inserted under a different generation. The built-in strategies are
    /// [`NewerWins`](crate::NewerWins) and
    /// [`ExistingWins`](crate::ExistingWins).
    type ReplaceStrategy: ReplaceStrategy<Self::KeyConfig>;

    /// The collection the map keeps its slots in. The map keeps a slot at
    /// every index up to the highest index that an insert has used.
    /// `type Storage = Vec<S>;` keeps them in a `Vec`, and any other
    /// [`SlotStorage`] works too, such as the `ArrayVec` and `SmallVec` a
    /// [`GenMapConfig`] can use.
    type Storage: SlotStorage<Item = S>;
}

/// What a [`GenMap<T, C>`](crate::GenMap) needs from its config `C`. `C`
/// must implement [`MapConfig`], and [`GenMapConfig`] for the map's
/// [`MapSlot<T, C>`](crate::MapSlot).
///
/// It is implemented for every such `C`, and it is sealed, so it cannot be
/// implemented outside this crate. It is the bound to use in code that is
/// generic over maps.
///
/// ```
/// use gen_map::{GenMap, MapConfigFor};
///
/// fn total<C: MapConfigFor<u32>>(map: &GenMap<u32, C>) -> u32 {
///     map.values().sum()
/// }
///
/// let mut map = GenMap::new();
/// map.insert(1);
/// map.insert(2);
/// assert_eq!(total(&map), 3);
/// ```
pub trait MapConfigFor<T>: sealed::Sealed<T> + MapConfig + GenMapConfig<MapSlot<T, Self>> {}

mod sealed {
    /// Keeps [`MapConfigFor`](super::MapConfigFor) from being implemented
    /// outside this crate.
    pub trait Sealed<T> {}

    /// Keeps [`SecondaryMapConfigFor`](super::SecondaryMapConfigFor) from
    /// being implemented outside this crate.
    pub trait SecondarySealed<T> {}
}

impl<T, C> sealed::Sealed<T> for C where C: MapConfig + GenMapConfig<MapSlot<T, C>> {}

impl<T, C> MapConfigFor<T> for C where C: MapConfig + GenMapConfig<MapSlot<T, C>> {}

/// What a [`SecondaryMap<T, C>`](crate::SecondaryMap) needs from its config
/// `C`. `C` must implement [`MapConfig`], and [`SecondaryMapConfig`] for the
/// map's [`SecondaryMapSlot<T, C>`](crate::SecondaryMapSlot).
///
/// It is implemented for every such `C`, and it is sealed, so it cannot be
/// implemented outside this crate. It is the bound to use in code that is
/// generic over secondary maps.
///
/// ```
/// use gen_map::{GenMap, SecondaryMap, SecondaryMapConfigFor};
///
/// fn total<C: SecondaryMapConfigFor<u32>>(map: &SecondaryMap<u32, C>) -> u32 {
///     map.values().sum()
/// }
///
/// let mut keys = GenMap::new();
/// let mut map = SecondaryMap::new();
/// map.insert(keys.insert(()), 1).unwrap();
/// map.insert(keys.insert(()), 2).unwrap();
/// assert_eq!(total(&map), 3);
/// ```
pub trait SecondaryMapConfigFor<T>:
    sealed::SecondarySealed<T> + MapConfig + SecondaryMapConfig<SecondaryMapSlot<T, Self>>
{
}

impl<T, C> sealed::SecondarySealed<T> for C where
    C: MapConfig + SecondaryMapConfig<SecondaryMapSlot<T, C>>
{
}

impl<T, C> SecondaryMapConfigFor<T> for C where
    C: MapConfig + SecondaryMapConfig<SecondaryMapSlot<T, C>>
{
}

/// The key config a [`Key`](crate::Key) uses when its `K` parameter is left
/// out.
///
/// Keys are a `u32` index and a `u32` generation stored as two fields.
pub type DefaultKeyConfig = Split<u32, u32>;

/// The config of a [`GenMap<T>`](crate::GenMap) and a
/// [`SecondaryMap<T>`](crate::SecondaryMap), which leave out their config
/// parameter `C`.
///
/// Keys use the [`DefaultKeyConfig`], and slots live in a `Vec`. A `GenMap`
/// slot retires when its generation runs out. A `SecondaryMap` uses
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
impl<S: GenSlotItem> GenMapConfig<S> for DefaultMapConfig {
    type Storage = Vec<S>;
}

#[cfg(feature = "alloc")]
impl<S: SecondarySlotItem> SecondaryMapConfig<S> for DefaultMapConfig {
    type ReplaceStrategy = NewerWins;
    type Storage = Vec<S>;
}
