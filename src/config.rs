use crate::key_piece::KeyPiece;

/// Compile time configuration of a [`GenMap`](crate::GenMap).
/// # Examples
///
/// ```
/// use gen_map::{Config, GenMap};
///
/// /// Four byte keys, and slots get reused forever.
/// struct Small;
///
/// impl Config for Small {
///     // index type
///     type Idx = u16;
///     // generation type
///     type Gen = u16;
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
/// Keys are `u32` index plus `u32` generation, so eight bytes, and slots retire
/// when their generation overflows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct DefaultConfig;

impl Config for DefaultConfig {
    type Idx = u32;
    type Gen = u32;
}
