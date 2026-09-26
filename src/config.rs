use crate::key_layout::{KeyLayout, Split};
use crate::key_piece::KeyPiece;
use crate::map::{MapKeyConfig, MapSlot};
use crate::slot::{Slot, SlotItem};
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

/// Chooses the key config of a [`GenMap`](crate::GenMap), the storage its
/// slots live in, and what happens when a slot's generation runs out.
///
/// A config is implemented for slot types, usually all of them at once with
/// a blanket impl over `S: SlotItem`. A `GenMap<T, C>` uses `C`'s impl for
/// its [`MapSlot<T, C>`](crate::MapSlot). The impl can put bounds on `S`, or
/// on the value it holds through [`SlotItem::Item`], when its storage needs
/// them.
///
/// A map's slot type is made from its key config, so the map reads the key
/// config from the impl for a stand-in slot, `Slot<u8, T, ()>`, which holds
/// the same value type. See [`MapConfigFor`].
///
/// # Examples
///
/// ```
/// use gen_map::{GenMap, KeyConfig, MapConfig, SlotItem, Split};
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
/// impl<S: SlotItem> MapConfig<S> for Small {
///     type KeyConfig = SmallKeys;
///     // The collection the slots live in.
///     type Storage = Vec<S>;
///     const WRAP_ON_OVERFLOW: bool = true;
/// }
///
/// let mut map = GenMap::<&str, Small>::new_with_config();
/// let key = map.insert("hello");
/// assert_eq!(core::mem::size_of_val(&key), 4);
/// assert_eq!(map[key], "hello");
/// ```
pub trait MapConfig<S: SlotItem> {
    /// The config of the keys the map hands out. It can depend on the value
    /// type, `S::Item`, but not on the rest of `S`. Maps whose configs name
    /// the same key config share a key type.
    type KeyConfig: KeyConfig;

    /// The collection the map keeps its slots in. `S` is the map's
    /// [`MapSlot`](crate::MapSlot) type.
    type Storage: SlotStorage<Item = S>;

    /// What happens when a slot's generation runs out, meaning it reaches
    /// the largest one its key can hold.
    ///
    /// `false` retires the slot. It is never used again, so no stale key can
    /// ever match a new value.
    ///
    /// `true` wraps the generation back to zero and keeps using the slot. A
    /// stale key from before the wrap can then match a new value.
    const WRAP_ON_OVERFLOW: bool = false;
}

/// The slot type a `GenMap<T, _>` reads its config's key config from. It
/// holds the same `T` as the map's own slot, but its other types are fixed,
/// so naming it needs no key config.
pub(crate) type KeyConfigSlot<T> = Slot<u8, T, ()>;

/// What a [`GenMap<T, C>`](crate::GenMap) needs from its config `C`.
///
/// `C` must implement [`MapConfig`] for the map's
/// [`MapSlot<T, C>`](crate::MapSlot), and for `Slot<u8, T, ()>`, and both
/// impls must name the same key config. The map reads the key config from
/// the second one, since its own slot type is made from the key config.
/// Both slots hold a `T`, so an impl over every [`SlotItem`] covers both.
///
/// It is implemented for every such `C`, so a config never implements it by
/// hand. It is the bound to use in code that is generic over maps.
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
    MapConfig<KeyConfigSlot<T>> + MapConfig<MapSlot<T, Self>, KeyConfig = MapKeyConfig<T, Self>>
{
}

impl<T, C> MapConfigFor<T> for C where
    C: MapConfig<KeyConfigSlot<T>> + MapConfig<MapSlot<T, C>, KeyConfig = MapKeyConfig<T, C>>
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
impl<S: SlotItem> MapConfig<S> for DefaultMapConfig {
    type KeyConfig = DefaultKeyConfig;
    type Storage = Vec<S>;
}
