use crate::key_layout::{KeyLayout, Split};
use crate::key_piece::KeyPiece;
use crate::map::MapSlot;
#[cfg(feature = "alloc")]
use crate::replace_strategy::NewerWinsWrapping;
use crate::replace_strategy::ReplaceStrategy;
use crate::secondary_map::SecondaryMapSlot;
use crate::slot::{GenSlotItem, SecondarySlotItem};
use crate::storage::SlotStorage;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// Chooses the index and generation types of a [`Key`](crate::Key), and how
/// the key stores the two.
///
/// # Examples
///
/// ```
/// use gen_map::{Key, KeyConfig, Split};
///
/// /// Four byte keys.
/// struct SmallKeys;
///
/// impl KeyConfig for SmallKeys {
///     type Idx = u16;
///     type Gen = u16;
///     // How a key stores the index and the generation.
///     type Layout = Split;
/// }
///
/// assert_eq!(core::mem::size_of::<Key<SmallKeys>>(), 4);
/// ```
pub trait KeyConfig {
    /// The integer type that represents the index of a slot.
    type Idx: KeyPiece;

    /// The integer type that represents the generation of a slot.
    type Gen: KeyPiece;

    /// How a key stores its index and generation. [`Split`](crate::Split)
    /// keeps them as two fields and [`Packed`](crate::Packed) puts them in
    /// the bits of one integer.
    type Layout: KeyLayout<Self::Idx, Self::Gen>;
}

/// Chooses the [`KeyConfig`] of the keys a map works with.
///
/// The config of a [`GenMap`](crate::GenMap) also implements
/// [`GenMapConfig`], which has `MapConfig` as a supertrait.
///
/// # Examples
///
/// ```
/// use gen_map::{GenMap, GenMapConfig, GenSlotItem, KeyConfig, MapConfig, Split};
///
/// /// Four byte keys.
/// struct SmallKeys;
///
/// impl KeyConfig for SmallKeys {
///     type Idx = u16;
///     type Gen = u16;
///     type Layout = Split;
/// }
///
/// /// Four byte keys, and a slot whose generation runs out wraps instead
/// /// of retiring.
/// struct Small;
///
/// impl MapConfig for Small {
///     type KeyConfig = SmallKeys;
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
    /// [`NewerWinsWrapping`](crate::NewerWinsWrapping),
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

/// The key config a [`Key`](crate::Key) uses when none is named.
///
/// Keys are a `u32` index and a `u32` generation stored as two fields.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct DefaultKeyConfig;

impl KeyConfig for DefaultKeyConfig {
    type Idx = u32;
    type Gen = u32;
    type Layout = Split;
}

/// The config of a [`GenMap<T>`](crate::GenMap) and a
/// [`SecondaryMap<T>`](crate::SecondaryMap), which leave out their config
/// parameter `C`.
///
/// Keys use the [`DefaultKeyConfig`], and slots live in a `Vec`. A `GenMap`
/// slot retires when its generation runs out. A `SecondaryMap` uses
/// [`NewerWinsWrapping`](crate::NewerWinsWrapping) to decide whether an
/// insert replaces a value that was inserted under a different generation.
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
    type ReplaceStrategy = NewerWinsWrapping;
    type Storage = Vec<S>;
}
