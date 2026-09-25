use crate::key_layout::{KeyLayout, Split};
use crate::key_piece::KeyPiece;
use crate::storage::SlotStorage;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// Compile time configuration of a [`Key`](crate::Key).
/// # Examples
///
/// ```
/// use gen_map::{Key, KeyConfig, Split};
///
/// /// Four byte keys.
/// struct SmallKeys;
///
/// impl KeyConfig for SmallKeys {
///     // index type
///     type Idx = u16;
///     // generation type
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

/// Compile time configuration of a [`GenMap`](crate::GenMap).
/// # Examples
///
/// ```
/// use gen_map::{GenMap, KeyConfig, MapConfig, Split};
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
/// /// Four byte keys, and slots are never retired.
/// struct Small;
///
/// impl MapConfig for Small {
///     type KeyConfig = SmallKeys;
///     // The collection the slots live in.
///     type Storage<S> = Vec<S>;
///     const WRAP_ON_OVERFLOW: bool = true;
/// }
///
/// let mut map = GenMap::<&str, Small>::new_with_config();
/// let key = map.insert("hello");
/// assert_eq!(core::mem::size_of_val(&key), 4);
/// assert_eq!(map[key], "hello");
/// ```
pub trait MapConfig {
    /// The config of the keys the map hands out. Maps whose configs name the
    /// same key config share a key type.
    type KeyConfig: KeyConfig;

    /// The collection the map keeps its slots in. `S` is the map's
    /// [`MapSlot`](crate::MapSlot) type.
    type Storage<S>: SlotStorage<S>;

    /// What happens when a slot's generation overflows.
    ///
    /// `false` retires the slot. It is never used again, so no stale key can
    /// ever match a new value.
    ///
    /// `true` wraps the generation back to zero and keeps using the slot. A
    /// stale key from before the wrap can then match a new value.
    const WRAP_ON_OVERFLOW: bool = false;
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
/// retire when their generation overflows. It needs the `alloc` feature,
/// which is on by default.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct DefaultMapConfig;

#[cfg(feature = "alloc")]
impl MapConfig for DefaultMapConfig {
    type KeyConfig = DefaultKeyConfig;
    type Storage<S> = Vec<S>;
}
