//! A configurable generational map.
//! # Examples
//!
//! ```
//! use gen_map::GenMap;
//!
//! let mut map = GenMap::new();
//! let a = map.insert("a");
//! let b = map.insert("b");
//!
//! assert_eq!(map[a], "a");
//! assert_eq!(map.remove(a), Some("a"));
//! assert!(map.get(a).is_none());
//! assert_eq!(map.len(), 1);
//!
//! for (key, value) in &map {
//!     assert_eq!(key, b);
//!     assert_eq!(*value, "b");
//! }
//! ```
//!
//! With a custom config:
//!
//! ```
//! use gen_map::{Config, GenMap, Key};
//!
//! struct Tiny;
//!
//! impl Config for Tiny {
//!     type Idx = u8;
//!     type Gen = u8;
//! }
//!
//! let mut map = GenMap::<u64, Tiny>::new_with_config();
//! let key: Key<Tiny> = map.insert(7);
//! assert_eq!(core::mem::size_of_val(&key), 2);
//! assert_eq!(map[key], 7);
//! ```

#![no_std]
#![warn(missing_docs)]

extern crate alloc;

#[cfg(test)]
extern crate std;

mod config;
mod key;
mod key_piece;
mod map;

pub use config::{Config, DefaultConfig};
pub use key::Key;
pub use key_piece::KeyPiece;
pub use map::{Drain, GenMap, IntoIter, Iter, IterMut, Keys, Values, ValuesMut};

#[cfg(test)]
mod tests;
