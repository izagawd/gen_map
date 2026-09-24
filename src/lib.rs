//! A generational map with a configurable key.
//!
//! [`GenMap`] stores values and hands out a [`Key`] for each one. A key stays
//! valid until its value is removed, and it never matches the value that
//! later takes the same slot. That makes keys safe to hold on to where plain
//! indices or references are not, such as in graphs, entity systems and
//! anything else that refers to values by handle.
//!
//! Inserting, removing and looking up a value are all O(1). The crate is
//! `no_std`, and it only needs an allocator for its `Vec` storage, which can
//! be turned off. See [Cargo features](#cargo-features).
//!
//! # Examples
//!
//! ```
//! use gen_map::GenMap;
//!
//! let mut map = GenMap::new();
//! let a = map.insert("a");
//! let b = map.insert("b");
//! assert_eq!(map[a], "a");
//!
//! assert_eq!(map.remove(a), Some("a"));
//! assert!(map.get(a).is_none());
//!
//! // `c` takes the slot `a` had, but `a` still does not match it.
//! let c = map.insert("c");
//! assert_eq!(c.idx(), a.idx());
//! assert!(map.get(a).is_none());
//! assert_eq!(map[c], "c");
//!
//! for (key, value) in &map {
//!     assert!(key == b || key == c);
//!     assert!(*value == "b" || *value == "c");
//! }
//! ```
//!
//! # How it works
//!
//! The map keeps its values in a list of slots. A key is the index of a slot
//! together with the slot's generation at the time the key was handed out,
//! and [`Key::idx`] and [`Key::generation`] read the two back. A slot's
//! generation goes up by one on every insert and every remove, so it is odd
//! while the slot holds a value and even while it does not. A key only
//! matches its slot while the two generations are equal, so removing a value
//! invalidates every copy of its key at once, and the next value in that
//! slot gets a key with a newer generation.
//!
//! Freed slots go on a free list and are reused before the map adds new
//! ones, so the map only grows when no slot is free.
//!
//! # Configuring the map
//!
//! A [`Config`] decides four things.
//!
//! - [`Idx`](Config::Idx) and [`Gen`](Config::Gen) are the integer types of
//!   the index and the generation. Any type that implements [`KeyPiece`]
//!   works, which is every unsigned integer from `u8` to `u128`, and
//!   `usize`.
//! - [`Layout`](Config::Layout) is how a key stores the generation and index. [`Split`] keeps
//!   them as two fields and [`Packed`] puts them in the bits of one integer.
//! - [`Storage`](Config::Storage) is the collection the slots live in, such
//!   as a `Vec`.
//! - [`WRAP_ON_OVERFLOW`](Config::WRAP_ON_OVERFLOW) says what happens to a
//!   slot whose generation runs out.
//!
//! [`DefaultConfig`] is the config a [`GenMap`] uses when none is named. Its
//! keys are a `u32` index and a `u32` generation stored as two fields, its
//! slots live in a `Vec`, and a slot retires when its generation runs out.
//!
//! A config is a type that is only ever named and never built, so a unit
//! struct is enough.
//!
//! ```
//! use gen_map::{Config, GenMap, Split};
//!
//! /// Two byte keys, so the map holds at most 256 slots.
//! struct Tiny;
//!
//! impl Config for Tiny {
//!     type Idx = u8;
//!     type Gen = u8;
//!     type Layout = Split;
//!     type Storage<S> = Vec<S>;
//! }
//!
//! let mut map = GenMap::<u64, Tiny>::new_with_config();
//! let key = map.insert(7);
//! assert_eq!(core::mem::size_of_val(&key), 2);
//! assert_eq!(map[key], 7);
//! ```
//!
//! [`GenMap::new`] can only create a map with the default config, because a
//! default type parameter takes no part in type inference. Any other config
//! goes through [`GenMap::new_with_config`].
//!
//! ## Key layouts
//!
//! [`Split`] stores the index and the generation as two fields, so a key is
//! as large as the two together, plus any padding their alignment needs.
//! Every value of both types can be used.
//!
//! [`Packed`] stores them in the bits of one integer instead. `Packed<R,
//! GEN_BITS>` gives the low `GEN_BITS` bits of an `R` to the generation and
//! the bits above them to the index, so a key is as large as `R` and the two
//! parts can have any bit counts that add up to it. The config's `Idx` and
//! `Gen` only have to be wide enough for their parts, and a config whose bit
//! counts do not add up fails to compile.
//!
//! ```
//! use gen_map::{Config, GenMap, Key, Packed};
//!
//! /// Four byte keys with 24 bits of index and 8 bits of generation.
//! struct Compact;
//!
//! impl Config for Compact {
//!     type Idx = u32;
//!     type Gen = u8;
//!     type Layout = Packed<u32, 8>;
//!     type Storage<S> = Vec<S>;
//! }
//!
//! let mut map = GenMap::<&str, Compact>::new_with_config();
//! let key = map.insert("a");
//! assert_eq!(core::mem::size_of::<Key<Compact>>(), 4);
//! assert_eq!(map[key], "a");
//! ```
//!
//! With either layout, `Option<Key>` is the same size as `Key`.
//!
//! ## When a generation runs out
//!
//! A slot's generation can only go up to the largest one its key can hold,
//! which is `Gen::MAX` for [`Split`] and the largest value of the generation
//! part for [`Packed`]. Each value uses up two generations, so a `u32`
//! generation lets a slot hold over two billion values, while a 4 bit
//! generation lets it hold eight. What happens to a slot after that is up to
//! [`WRAP_ON_OVERFLOW`](Config::WRAP_ON_OVERFLOW).
//!
//! By default, the slot retires. It stays in the storage but is never used
//! again, so no stale key can ever match a new value.
//!
//! When `WRAP_ON_OVERFLOW` is `true`, the generation wraps back to zero and
//! the slot is reused. No slot is ever lost, but a key that is old enough can
//! match a new value once the generation wraps around to it again.
//!
//! ## Storage
//!
//! The slots can live in any collection that implements [`SlotStorage`]. A
//! `Vec` does, and so do two collections from other crates when their
//! features are on. `ArrayVec` from `arrayvec` has a fixed capacity and
//! never allocates, and `SmallVec` from `smallvec` keeps a few slots inline
//! before it allocates. [`SlotStorage`] is an unsafe trait, because the map
//! relies on the storage behaving like a `Vec` when it reads slots without
//! bounds checks.
//!
//! The methods that make room ahead of time, which are
//! [`reserve`](GenMap::reserve), [`try_reserve`](GenMap::try_reserve),
//! [`with_capacity`](GenMap::with_capacity) and
//! [`with_capacity_and_config`](GenMap::with_capacity_and_config), are only
//! there when the storage also implements [`ReserveStorage`], as a `Vec`
//! does.
//!
//! # When the map is full
//!
//! A map is full when none of its slots are free, and it can not add another
//! one, either because its keys have no index left for a new slot or because
//! the storage can not make room for one. A `u8` index, for example, allows
//! 256 slots.
//!
//! [`insert`](GenMap::insert) panics on a full map. The other ways to insert
//! report it as an error.
//!
//! - [`try_insert`](GenMap::try_insert) returns an [`InsertError`] that hands
//!   the value back.
//! - [`vacant_entry`](GenMap::vacant_entry) picks the slot before the value
//!   exists, and returns a [`FullError`] if there is none. The entry's
//!   [`key`](VacantEntry::key) is the key the value will get, and dropping
//!   the entry leaves the map as it was.
//! - [`try_insert_with_key`](GenMap::try_insert_with_key) runs a closure that
//!   may fail, and returns an [`InsertWithError`] if the map is full or the
//!   closure fails.
//!
//! These methods also report a `Vec` that can not allocate as
//! [`FullError::StorageFull`] instead of panicking.
//!
//! ```
//! use gen_map::{Config, GenMap, InsertError, Split};
//!
//! struct Tiny;
//!
//! impl Config for Tiny {
//!     type Idx = u8;
//!     type Gen = u8;
//!     type Layout = Split;
//!     type Storage<S> = Vec<S>;
//! }
//!
//! let mut map = GenMap::<u32, Tiny>::new_with_config();
//! for i in 0..256 {
//!     map.insert(i);
//! }
//! match map.try_insert(256) {
//!     Err(InsertError::IndexExhausted(value)) => assert_eq!(value, 256),
//!     _ => unreachable!(),
//! }
//! ```
//!
//! # Values that know their own key
//!
//! [`insert_with_key`](GenMap::insert_with_key) gives a closure the new
//! value's key before the value is stored, which helps with values that
//! refer to themselves, such as the nodes of a graph.
//!
//! ```
//! use gen_map::{GenMap, Key};
//!
//! struct Node {
//!     me: Key,
//!     edges: Vec<Key>,
//! }
//!
//! let mut graph = GenMap::new();
//! let a = graph.insert_with_key(|me| Node { me, edges: Vec::new() });
//! let b = graph.insert_with_key(|me| Node { me, edges: vec![a] });
//! assert_eq!(graph[b].me, b);
//! assert_eq!(graph[b].edges, [a]);
//! ```
//!
//! # Taking a value out for a while
//!
//! [`detach`](GenMap::detach) moves a value out of the map but keeps its slot
//! reserved for its key, and [`reattach`](GenMap::reattach) puts a value back
//! under that same key. In between, the key is invalid and no insert can
//! take the slot. This lets code change a value while it has the rest of the
//! map to work with.
//!
//! ```
//! use gen_map::GenMap;
//!
//! let mut map = GenMap::new();
//! let a = map.insert(1);
//! let b = map.insert(2);
//!
//! let mut value = map.detach(a).unwrap();
//! value += map[b];
//! map.reattach(a, value);
//! assert_eq!(map[a], 3);
//! ```
//!
//! # Several values at once
//!
//! [`get_disjoint_mut`](GenMap::get_disjoint_mut) hands out mutable
//! references to several values at once, after checking that every key is
//! valid and that no two keys point at the same slot.
//! [`get_disjoint_mut_at`](GenMap::get_disjoint_mut_at) does the same with
//! slot indices, and hands each key back with its value.
//!
//! ```
//! use gen_map::GenMap;
//!
//! let mut map = GenMap::new();
//! let a = map.insert(1);
//! let b = map.insert(2);
//!
//! let [x, y] = map.get_disjoint_mut([a, b]).unwrap();
//! core::mem::swap(x, y);
//! assert_eq!((map[a], map[b]), (2, 1));
//! ```
//!
//! # Looking a slot up by its index
//!
//! [`key_at`](GenMap::key_at) and [`get_at`](GenMap::get_at) find the value
//! in the slot at an index along with its current key, for code that only
//! kept the index. [`generation_at`](GenMap::generation_at) returns a slot's
//! generation whether it holds a value or not.
//!
//! # Iterating and removing in bulk
//!
//! Every iterator visits the values in slot order, which is also the order
//! of their keys. [`iter`](GenMap::iter), [`iter_mut`](GenMap::iter_mut),
//! [`keys`](GenMap::keys), [`values`](GenMap::values),
//! [`values_mut`](GenMap::values_mut) and the owning `into_iter` all know
//! their exact length and can run from both ends.
//!
//! [`retain`](GenMap::retain), [`drain`](GenMap::drain) and
//! [`clear`](GenMap::clear) remove values but keep every slot and its
//! generation, so old keys stay invalid. [`reset`](GenMap::reset) removes
//! the slots too while keeping the allocation. The generations start over
//! after a reset, so a key from before it can match a value inserted after
//! it.
//!
//! # Unchecked access
//!
//! Every lookup has an `_unchecked` form, such as
//! [`get_unchecked`](GenMap::get_unchecked), that skips the bounds and
//! generation checks for code that already knows its key or index is valid.
//! Calling one with an invalid key or index is undefined behavior.
//!
//! [`Key::from_raw_parts`] builds a key from an index and a generation. It
//! is unsafe because the generation must be odd and both parts must fit the
//! layout, and a key that breaks either rule makes later lookups undefined
//! behavior.
//!
//! # Cargo features
//!
//! - `alloc` is on by default. It adds the `Vec` storage, [`DefaultConfig`]
//!   and [`GenMap::new`], and makes [`DefaultConfig`] the config that
//!   [`GenMap`] and [`Key`] use when none is named. Without it the crate
//!   needs no allocator, and every map needs a config whose storage does not
//!   allocate, such as an `ArrayVec`.
//! - `arrayvec` lets an `arrayvec::ArrayVec` hold the slots.
//! - `smallvec` lets a `smallvec::SmallVec` hold the slots. It uses the 2.0
//!   beta of `smallvec`, which needs an allocator and Rust 1.86.
//!
//! To use the map without any allocator, turn `alloc` off and `arrayvec` on.
//!
//! ```toml
//! [dependencies]
//! gen_map = { version = "0.2", default-features = false, features = ["arrayvec"] }
//! ```
//!
//! # Minimum supported Rust version
//!
//! The crate builds on Rust 1.79 and later. The `smallvec` feature needs
//! Rust 1.86, because the 2.0 beta of `smallvec` does.

#![no_std]
#![warn(missing_docs)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(test)]
extern crate std;

mod config;
mod error;
mod key;
mod key_layout;
mod key_piece;
mod map;
mod storage;

pub use config::Config;
#[cfg(feature = "alloc")]
pub use config::DefaultConfig;
pub use error::{
    FullError, GetDisjointMutAtError, GetDisjointMutError, InsertError, InsertWithError,
};
pub use key::Key;
pub use key_layout::{KeyLayout, Packed, PackedRepr, Split, SplitRepr};
pub use key_piece::KeyPiece;
pub use map::{
    Drain, GenMap, IntoIter, Iter, IterMut, Keys, Slot, StorageError, VacantEntry, Values,
    ValuesMut,
};
pub use storage::{ReserveStorage, SlotStorage};

// The tests use `Vec` storage and the default config, so they need `alloc`.
#[cfg(all(test, feature = "alloc"))]
mod tests;
