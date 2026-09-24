use crate::key_layout::KeyLayout;
#[cfg(feature = "alloc")]
use crate::key_layout::Split;
use crate::key_piece::KeyPiece;
use crate::storage::SlotStorage;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// Compile time configuration of a [`GenMap`](crate::GenMap).
/// # Examples
///
/// ```
/// use gen_map::{Config, GenMap, Split};
///
/// /// Four byte keys, and slots are never retired.
/// struct Small;
///
/// impl Config for Small {
///     // index type
///     type Idx = u16;
///     // generation type
///     type Gen = u16;
///     // How a key stores the index and the generation.
///     type Layout = Split;
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
pub trait Config {
    /// The integer type that represents the index of a slot.
    type Idx: KeyPiece;

    /// The integer type that represents the generation of a slot.
    type Gen: KeyPiece;

    /// How a key stores its index and generation. [`Split`](crate::Split)
    /// keeps them as two fields and [`Packed`](crate::Packed) puts them in
    /// the bits of one integer.
    type Layout: KeyLayout<Self::Idx, Self::Gen>;

    /// The collection the map keeps its slots in. `S` is the map's
    /// [`Slot`](crate::Slot) type.
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

/// The config a [`GenMap`](crate::GenMap) uses when none is named.
///
/// Keys are a `u32` index and a `u32` generation stored as two fields,
/// slots live in a `Vec`, and slots retire when their generation overflows.
/// It needs the `alloc` feature, which is on by default.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct DefaultConfig;

#[cfg(feature = "alloc")]
impl Config for DefaultConfig {
    type Idx = u32;
    type Gen = u32;
    type Layout = Split;
    type Storage<S> = Vec<S>;
}
