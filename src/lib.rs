//! A generational map with a configurable key.
//!
//! [`GenMap`] stores values and hands out a [`Key`] for each one. A key
//! matches its value until the value is removed, and by default it never
//! matches a value that later takes the same slot. That makes keys safe to
//! hold on to where plain indices or references are not, such as in graphs,
//! entity systems and anything else that refers to values by handle. An old
//! key can only match a new value with a config that wraps generations,
//! after [`reset`](GenMap::reset), or through
//! [`reattach`](GenMap::reattach), which puts a value back under the key it
//! was detached from. All three are described below.
//!
//! Inserting, removing and looking up a value are all O(1). The crate is
//! `no_std`, and it only needs an allocator for storage that uses the heap,
//! such as `Vec`. The `Vec` storage comes from the `alloc` feature, which is
//! on by default and can be turned off. See [Cargo features](#cargo-features).
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
//! // `c` takes the slot `a` had, but `a` still does not match the new value.
//! let c = map.insert("c");
//! assert_eq!(c.idx(), a.idx());
//! assert!(map.get(a).is_none());
//! assert_eq!(map[c], "c");
//!
//! // Values come out in slot order, and `c` took the slot `a` had.
//! let pairs: Vec<_> = map.iter().collect();
//! assert_eq!(pairs, [(c, &"c"), (b, &"b")]);
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
//! stops every copy of its key from matching at once, and the next value in
//! that slot gets a key with a newer generation. Retiring, wrapping and
//! [`reattach`](GenMap::reattach) change a slot's generation in other ways.
//!
//! Freed slots go on a free list and are reused before the map adds new
//! ones, so the map only grows when no slot is free.
//!
//! ## Slots
//!
//! Each slot is a [`Slot`], which pairs a generation with a `T` while the
//! generation is odd, or with a `U` while it is even. The map keeps a value
//! in the `T` and, while the slot is free, a link to the next free slot in
//! the `U`. [`Even`] and [`Odd`] hold numbers known to be even or odd, and
//! [`Parity`] is a slot's generation together with its value.
//!
//! # Configuring the map
//!
//! A map is configured by a [`MapConfig`], and its keys by a [`KeyConfig`].
//!
//! A [`KeyConfig`] decides:
//!
//! - [`Idx`](KeyConfig::Idx) and [`Gen`](KeyConfig::Gen) are the integer
//!   types of the index and the generation. Any type that implements
//!   [`KeyPiece`] works, which is every unsigned integer from `u8` to
//!   `u128`, and `usize`.
//! - [`Layout`](KeyConfig::Layout) is how a key stores the generation and
//!   index. [`Split`] keeps them as two fields and [`Packed`] puts them in
//!   the bits of one integer.
//!
//! A [`MapConfig`] decides three things.
//!
//! - [`KeyConfig`](MapConfig::KeyConfig) is the config of the keys the map
//!   hands out. Maps whose configs have the same key config share a key
//!   type. A key does not record which map handed it out, so another map
//!   with the same key config accepts it and may hold an unrelated value
//!   under it.
//! - [`Storage`](MapConfig::Storage) is the collection the slots live in,
//!   such as a `Vec`.
//! - [`WRAP_ON_OVERFLOW`](MapConfig::WRAP_ON_OVERFLOW) determines what
//!   happens to a slot whose generation runs out.
//!
//! [`DefaultMapConfig`] is the config a [`GenMap`] uses when none is named.
//! Its keys use the [`DefaultKeyConfig`], so they are a `u32` index and a
//! `u32` generation stored as two fields. Its slots live in a `Vec`, and a
//! slot retires when its generation runs out. [`DefaultKeyConfig`] is also
//! the key config a [`Key`] uses when none is named.
//!
//! A config is only used as a type parameter and never created as a value,
//! so an empty struct is enough. One type can implement both traits and name
//! itself as its key config.
//!
//! ```
//! use gen_map::{GenMap, KeyConfig, MapConfig, Split};
//!
//! /// A `u8` index and a `u8` generation, so keys are two bytes and the map
//! /// holds at most 256 slots.
//! struct Tiny;
//!
//! impl KeyConfig for Tiny {
//!     type Idx = u8;
//!     type Gen = u8;
//!     type Layout = Split;
//! }
//!
//! impl MapConfig for Tiny {
//!     type KeyConfig = Self;
//!     type Storage<S> = Vec<S>;
//! }
//!
//! let mut map = GenMap::<u64, Tiny>::new_with_config();
//! let key = map.insert(7);
//! assert_eq!(core::mem::size_of_val(&key), 2);
//! assert_eq!(map[key], 7);
//! ```
//!
//! [`GenMap::new`] only exists for the default config. Rust does not use a
//! default type parameter when it infers types, so a `new` that took any
//! config would need the config written out on every call. Any other config
//! goes through [`GenMap::new_with_config`].
//!
//! ## Key layouts
//!
//! [`Split`] stores the index and the generation as two fields, so a key is
//! as large as the two together, plus any padding their alignment needs.
//! The index and the generation can each use every value of their type.
//!
//! [`Packed`] stores them in the bits of one integer instead. `Packed<R,
//! GEN_BITS>` gives the low `GEN_BITS` bits of an `R` to the generation and
//! the bits above them to the index, so a key is as large as `R`, and the two
//! parts can have any bit counts that add up to the bits of `R`. The key
//! config's `Idx` and `Gen` only have to be wide enough for their parts, and
//! a key config whose bit counts do not add up fails to compile.
//!
//! ```
//! use gen_map::{GenMap, Key, KeyConfig, MapConfig, Packed};
//!
//! /// Four byte keys with 24 bits of index and 8 bits of generation.
//! struct Compact;
//!
//! impl KeyConfig for Compact {
//!     type Idx = u32;
//!     type Gen = u8;
//!     type Layout = Packed<u32, 8>;
//! }
//!
//! impl MapConfig for Compact {
//!     type KeyConfig = Self;
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
//! A key can also be built by hand. [`KeyLayout::pack`] packs an index and
//! an [`Odd`] generation into the layout's `Repr`, or returns `None` if
//! either does not fit, and [`Key::from_repr`] turns the `Repr` into a key.
//! [`Key::repr`] hands it back.
//!
//! ## When a generation runs out
//!
//! A slot's generation can only go up to the largest one its key can hold,
//! which is `Gen::MAX` for [`Split`] and the largest value of the generation
//! part for [`Packed`]. Each value a slot holds uses up two generations, one
//! when it is inserted and one when it is removed, so a `u32` generation lets
//! a slot hold over two billion values, while a 4 bit generation lets it hold
//! eight. What happens to a slot after that is up to
//! [`WRAP_ON_OVERFLOW`](MapConfig::WRAP_ON_OVERFLOW).
//!
//! By default, the slot retires. It stays in the storage but is never used
//! again, so no stale key can ever match a new value.
//!
//! When `WRAP_ON_OVERFLOW` is `true`, the generation wraps back to zero and
//! the slot is reused. No slot is ever lost, but a key that is old enough can
//! match a new value once the generation wraps around to it again.
//!
//! [`retire`](GenMap::retire) removes a value and retires its slot, no
//! matter how the map is configured. Code built on a map that wraps can use
//! it to keep the slots it chooses from wrapping, and
//! [`Key::is_max_generation`] can be used to determine when a slot has reached that point.
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
//! [`with_capacity_and_config`](GenMap::with_capacity_and_config), only
//! exist when the storage also implements [`ReserveStorage`], as `Vec` and
//! `SmallVec` do.
//!
//! # When the map is full
//!
//! A map is full when none of its slots are free, and it cannot add another
//! one, either because its keys have no index left for a new slot or because
//! the storage cannot make room for one. A `u8` index, for example, allows
//! 256 slots.
//!
//! [`insert`](GenMap::insert) panics on a full map. The other ways to insert
//! report a full map as an error.
//!
//! - [`try_insert`](GenMap::try_insert) returns an [`InsertError`] that hands
//!   the value back.
//! - [`vacant_entry`](GenMap::vacant_entry) picks the slot before the value
//!   exists, and returns a [`FullError`] if there is none. The entry's
//!   [`key`](VacantEntry::key) is the key the value will get, and dropping
//!   the entry inserts nothing.
//! - [`try_insert_with_key`](GenMap::try_insert_with_key) runs a closure that
//!   may fail, and returns an [`InsertWithError`] if the map is full or the
//!   closure fails.
//!
//! When a `Vec` cannot allocate, these methods also return a
//! [`StorageFull`](FullError::StorageFull) error instead of panicking.
//!
//! ```
//! use gen_map::{GenMap, InsertError, KeyConfig, MapConfig, Split};
//!
//! struct Tiny;
//!
//! impl KeyConfig for Tiny {
//!     type Idx = u8;
//!     type Gen = u8;
//!     type Layout = Split;
//! }
//!
//! impl MapConfig for Tiny {
//!     type KeyConfig = Self;
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
//! under that same key. In between, the key matches with no value and no insert
//! can take the slot. This lets code take a value out, change it while
//! borrowing the rest of the map, and put it back under the same key.
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
//! references to several values at once, after checking that the map has a
//! value for every key and that no two keys point at the same slot.
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
//! [`clear`](GenMap::clear) remove values the same way
//! [`remove`](GenMap::remove) does, so the slots stay and old keys stop
//! matching. [`reset`](GenMap::reset) removes the slots too while keeping
//! the allocation. The generations start over after a reset, so a key from
//! before the reset can match a value inserted after it.
//!
//! Iterating goes through every slot, including the ones that hold no
//! value, and so do `retain`, `drain` and `clear`. Only `reset` removes
//! slots, so the time these take follows
//! [`slots_len`](GenMap::slots_len) rather than [`len`](GenMap::len).
//!
//! # Unchecked access
//!
//! Every lookup has an `_unchecked` form, such as
//! [`get_unchecked`](GenMap::get_unchecked), that skips the checks the
//! normal form makes, for code that already knows they would pass. Calling
//! one when a check would fail is undefined behavior.
//!
//! # Cargo features
//!
//! - `alloc` is on by default. It adds the `Vec` storage,
//!   [`DefaultMapConfig`] and [`GenMap::new`], and makes
//!   [`DefaultMapConfig`] the config that [`GenMap`] uses when none is named.
//!   Without it the crate needs no allocator, and every map needs a config
//!   whose storage does not allocate, such as an `ArrayVec`.
//! - `arrayvec` lets a config use `arrayvec::ArrayVec` as its storage.
//! - `smallvec` lets a config use `smallvec::SmallVec` as its storage. It
//!   uses the 2.0 beta of `smallvec`, which needs an allocator and Rust 1.86.
//!   Until smallvec 2.0 is released, a newer smallvec beta or a new release
//!   of `gen_map` may break this feature, so it is not covered by semver.
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
mod parity;
mod slot;
mod storage;

#[cfg(feature = "alloc")]
pub use config::DefaultMapConfig;
pub use config::{DefaultKeyConfig, KeyConfig, MapConfig};
pub use error::{
    FullError, GetDisjointMutAtError, GetDisjointMutError, InsertError, InsertWithError,
};
pub use key::Key;
pub use key_layout::{KeyLayout, Packed, PackedRepr, Split, SplitRepr};
pub use key_piece::KeyPiece;
pub use map::{
    Drain, GenMap, IntoIter, Iter, IterMut, Keys, MapSlot, StorageError, VacantEntry, Values,
    ValuesMut,
};
pub use parity::{Even, Odd};
pub use slot::{Parity, Slot};
pub use storage::{ReserveStorage, SlotStorage};

// The tests use `Vec` storage and the default config, so they need `alloc`.
#[cfg(all(test, feature = "alloc"))]
mod tests;
