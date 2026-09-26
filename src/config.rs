use crate::key_layout::{KeyLayout, Split};
use crate::key_piece::KeyPiece;
use crate::map::MapSlot;
use crate::secondary_storage::SecondarySlotStorage;
use crate::slot::{GenSlotItem, SecondarySlotItem};
use crate::storage::GenSlotStorage;
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
/// `T` is the type of the values in the map. A config is usually
/// implemented for every value type at once, with `impl<T> MapConfig<T> for
/// YourConfig`, as in the example below. The impl can put bounds on `T` to
/// limit which maps can use the config.
/// [`GenSlotItem`](GenSlotItem#bounds-on-the-value-and-the-slot) shows how,
/// with examples.
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
/// impl<T> MapConfig<T> for Small {
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
pub trait MapConfig<T> {
    /// The config of the keys the map hands out. It can depend on the value
    /// type, `T`. Maps whose configs use the same key config share a key
    /// type.
    type KeyConfig: KeyConfig;
}

/// This trait is used to choose what happens when a slot's generation runs out, and the
/// collection the slots of a [`GenMap`](crate::GenMap) live in.
///
/// `S` is the slot type, the [`Slot`](crate::Slot) the map keeps each value
/// in, and `S::Value` is the type of that value. A config is usually
/// implemented for every slot type at once, with `impl<S: GenSlotItem>
/// GenMapConfig<S> for YourConfig`, as in the [`MapConfig`] example. A
/// `GenMap<T, C>` then uses the impl for its own slots,
/// [`MapSlot<T, C>`](crate::MapSlot).
///
/// [`MapConfig<S::Value>`](MapConfig) is a supertrait, so the impl can only
/// cover slots whose values the [`MapConfig`] impl covers. The impl can put
/// bounds on `S::Value` or on `S` to limit which maps can use the config.
/// [`GenSlotItem`](GenSlotItem#bounds-on-the-value-and-the-slot) shows how,
/// with examples.
pub trait GenMapConfig<S: GenSlotItem>: MapConfig<S::Value> {
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
    type Storage: GenSlotStorage<Item = S>;
}

/// Chooses whether a key with a newer generation replaces the value in a
/// slot, and the collection [`SecondarySlotItem`] slots are kept in.
///
/// `S` is the slot type, and `S::Value` is the type of the value in it. The
/// key config comes from the [`MapConfig<S::Value>`](MapConfig) impl, which
/// is a supertrait.
pub trait SecondaryMapConfig<S: SecondarySlotItem>: MapConfig<S::Value> {
    /// What happens when a value is inserted under a key whose generation is
    /// newer than the generation of a slot that already holds a value.
    ///
    /// `true`, the default, replaces the slot's value with the new one.
    ///
    /// `false` keeps the slot's value, and the new value is not inserted.
    ///
    /// Either way, when the slot holds a value, a key whose generation is the
    /// same as the slot's always replaces that value, and a key whose
    /// generation is older than the slot's never does.
    const OVERRIDE_OLDER_GEN: bool = true;

    /// The collection the slots are kept in.
    type Storage: SecondarySlotStorage<Item = S>;
}

/// What a [`GenMap<T, C>`](crate::GenMap) needs from its config `C`: a
/// [`MapConfig<T>`](MapConfig) impl, and a [`GenMapConfig`] impl for the
/// map's [`MapSlot<T, C>`](crate::MapSlot).
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
pub trait MapConfigFor<T>:
    sealed::Sealed<T> + MapConfig<T> + GenMapConfig<MapSlot<T, Self>>
{
}

mod sealed {
    /// Keeps [`MapConfigFor`](super::MapConfigFor) from being implemented
    /// outside this crate.
    pub trait Sealed<T> {}
}

impl<T, C> sealed::Sealed<T> for C where C: MapConfig<T> + GenMapConfig<MapSlot<T, C>> {}

impl<T, C> MapConfigFor<T> for C where C: MapConfig<T> + GenMapConfig<MapSlot<T, C>> {}

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

/// The config a [`GenMap`](crate::GenMap) uses when none is named.
///
/// Keys use the [`DefaultKeyConfig`], slots live in a `Vec`, and slots
/// retire when their generation runs out. It needs the `alloc` feature,
/// which is on by default.
#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct DefaultMapConfig;

#[cfg(feature = "alloc")]
impl<T> MapConfig<T> for DefaultMapConfig {
    type KeyConfig = DefaultKeyConfig;
}

#[cfg(feature = "alloc")]
impl<S: GenSlotItem> GenMapConfig<S> for DefaultMapConfig {
    type Storage = Vec<S>;
}
