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
//! Inserting, removing and looking up a value are all O(1). The crate never
//! uses `std`, and [Cargo features](#cargo-features) has information on
//! which features need an allocator.
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
//! ones, so the map only grows when no slot is free. No slot ever gets the
//! largest value of the index type as its index. The free list uses that
//! value to mean "no slot", and leaving it out keeps the number of values
//! small enough for the map to count them in the index type.
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
//! A map is configured by a [`MapConfig`] and a [`GenMapConfig`], and its
//! keys by a [`KeyConfig`].
//!
//! A [`KeyConfig`] decides the integer types of a key's index and
//! generation, and how the key stores the two. This crate provides two kinds
//! of key configs.
//!
//! - [`Split<Idx, Gen>`](Split) keeps the index and the generation as two
//!   fields of the types `Idx` and `Gen`. Any type that implements
//!   [`KeyPiece`] works, which is every unsigned integer from `u8` to
//!   `u128`, and `usize`.
//! - [`Packed<R, GEN_BITS>`](Packed) puts them in the bits of one integer
//!   `R`, with the generation in the lowest `GEN_BITS` bits. It picks the
//!   index and generation types from its bit counts.
//!
//! With either, `Option<Key>` is the same size as `Key`.
//!
//! A [`MapConfig`] has one associated type,
//! [`KeyConfig`](MapConfig::KeyConfig), which is the config of the keys the
//! map hands out. Maps whose configs have the same key config share a key
//! type. A key does not record which map handed it out, so another map with
//! the same key config accepts it and may hold an unrelated value under it.
//!
//! A [`GenMapConfig`] decides two things.
//!
//! - [`WRAP_ON_OVERFLOW`](GenMapConfig::WRAP_ON_OVERFLOW) determines what
//!   happens to a slot whose generation runs out.
//! - [`Storage`](GenMapConfig::Storage) is the collection the map keeps its
//!   slots in. `type Storage = Vec<S>;` keeps them in a `Vec`. The type must
//!   implement [`SlotStorage`]. `Vec` implements it when the `alloc` feature
//!   is on, and `alloc` is on by default. `arrayvec::ArrayVec` implements it
//!   when the `arrayvec` feature is on, and `smallvec::SmallVec` implements
//!   it when the `smallvec` feature is on.
//!
//! A [`GenMapConfig`] is implemented for slot types, usually for all of them
//! at once with `impl<S: GenSlotItem> GenMapConfig<S>`. A slot is what the
//! map keeps each value in, and [`GenSlotItem::Value`] is the type of that
//! value. The impl can put bounds on the value or on the slot to limit which
//! maps can use the config.
//! [`GenSlotItem`](GenSlotItem#bounds-on-the-value-and-the-slot) has three
//! examples of these bounds.
//!
//! [`DefaultMapConfig`] is the config a [`GenMap`] uses when none is named.
//! Its keys use the [`DefaultKeyConfig`], so they are a `u32` index and a
//! `u32` generation stored as two fields. Its slots live in a `Vec`, and a
//! slot retires when its generation runs out. [`DefaultKeyConfig`] is also
//! the key config a [`Key`] uses when none is named.
//!
//! A map config is only used as a type parameter and never created as a
//! value, so an empty struct is enough.
//!
//! ```
//! use gen_map::{GenMap, GenMapConfig, GenSlotItem, MapConfig, Split};
//!
//! /// A `u8` index and a `u8` generation, so keys are two bytes and the map
//! /// holds at most 255 slots.
//! struct Tiny;
//!
//! impl MapConfig for Tiny {
//!     type KeyConfig = Split<u8, u8>;
//! }
//!
//! impl<S: GenSlotItem> GenMapConfig<S> for Tiny {
//!     type Storage = Vec<S>;
//! }
//!
//! let mut map = GenMap::<u64, Tiny>::new_with_config();
//! let key = map.insert(7);
//! assert_eq!(core::mem::size_of_val(&key), 2);
//! assert_eq!(map[key], 7);
//! ```
//!
//! [`GenMap::new`] only exists for the default config. Any other config goes
//! through [`GenMap::new_with_config`].
//!
//! ## When a generation runs out
//!
//! A slot's generation can only go up to the largest one its key can hold,
//! which is `Gen::MAX` for [`Split`] and the largest value of the generation
//! part for [`Packed`]. Each value a slot holds uses up two generations, one
//! when it is inserted and one when it is removed, so a `u32` generation lets
//! a slot hold over two billion values, while a 4 bit generation lets it hold
//! eight. What happens to a slot after that is up to
//! [`WRAP_ON_OVERFLOW`](GenMapConfig::WRAP_ON_OVERFLOW).
//!
//! By default, the slot retires. It stays in the storage but is never used
//! again, so no stale key can ever match a new value.
//!
//! When `WRAP_ON_OVERFLOW` is `true`, the generation wraps back to zero and
//! the slot is reused. No slot is ever lost, but a key that is old enough can
//! match a new value once the generation wraps around to it again.
//!
//! [`retire`](GenMap::retire) removes a value and retires its slot, no matter
//! how the map is configured. Code built on a map that wraps can call
//! `retire` to keep the slots it chooses from wrapping.
//! [`Key::is_max_generation`] can be used to determine whether a key has the
//! largest generation its key config can hold, which is when removing the
//! key's value would wrap its slot.
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
//! The owning `into_iter` only exists when the storage also implements
//! `IntoIterator`, which `Vec`, `ArrayVec` and `SmallVec` do.
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
//! the storage cannot make room for one. The largest value of the index type
//! is never a slot's index, so a `u8` index, for example, allows 255 slots
//! instead of 256. Leaving that value out only costs a slot when a key can
//! hold it. With a [`Packed<u32, 8>`](Packed) key config, for example, the
//! index type is `u32`, but a key's index has only 24 bits and never reaches
//! `u32::MAX`, so the map still allows all 16,777,216 slots.
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
//! under that same key. In between, the map has no value for the key, but no
//! insert can take the slot. This lets code take a value out, change it while
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
//! map.reattach(a, value).unwrap();
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
//! # Looking a slot up by its index
//!
//! [`key_at`](GenMap::key_at) finds the current key of the value in the slot at
//! an index, and [`get_at`](GenMap::get_at) finds the value along with that
//! key, and requires only the index to use.
//! [`generation_at`](GenMap::generation_at) returns a slot's generation whether
//! it holds a value or not.
//!
//! # Iterating and removing in bulk
//!
//! Every iterator visits the values in slot order, which is also the order of
//! their keys, and knows its exact length, meaning it implements
//! `ExactSizeIterator`. [`iter`](GenMap::iter),
//! [`iter_mut`](GenMap::iter_mut), [`keys`](GenMap::keys),
//! [`values`](GenMap::values) and [`values_mut`](GenMap::values_mut) can also
//! run from both ends, meaning they implement `DoubleEndedIterator`.
//! [`IntoIter`], the iterator that the owning `into_iter` returns, only
//! implements `DoubleEndedIterator` when the storage's iterator implements
//! both `DoubleEndedIterator` and `ExactSizeIterator`.
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
//! Every lookup on a `GenMap` or a `SecondaryMap` has an `_unchecked` form,
//! such as [`get_unchecked`](GenMap::get_unchecked), that skips the checks
//! the normal form makes, for code that already knows they would pass.
//! Calling one when a check would fail is undefined behavior.
//!
//! # Secondary maps
//!
//! A [`SecondaryMap`] stores values under the keys a [`GenMap`] hands out,
//! to add data to the values of a `GenMap` without changing their type. Its
//! config must have the same [`KeyConfig`] as the `GenMap`'s config, so
//! that both maps use the same key type.
//!
//! ```
//! use gen_map::{GenMap, SecondaryMap};
//!
//! let mut names = GenMap::new();
//! let mut ages = SecondaryMap::new();
//! let alice = names.insert("Alice");
//! ages.insert(alice, 30).unwrap();
//! assert_eq!(ages[alice], 30);
//! ```
//!
//! Removing a value from the `GenMap` does not remove it from the
//! `SecondaryMap`. When the `GenMap` gives the slot to a new value, an
//! insert under the new key finds the old value in the `SecondaryMap`, and
//! the config's [`ReplaceStrategy`](SecondaryMapConfig::ReplaceStrategy)
//! decides whether to replace it.
//!
//! - [`NewerWins`] replaces it when the key's generation is larger.
//!   `NewerWins` is the `ReplaceStrategy` of [`DefaultMapConfig`].
//! - [`ExistingWins`] never replaces it.
//!
//! Any other type that implements [`ReplaceStrategy`] can be used instead.
//!
//! A `SecondaryMap` keeps a slot at every index up to the highest index that an
//! insert has used. Like a `GenMap`, it never uses the largest value of the
//! index type. Its `insert` returns
//! [`IndexReserved`](SecondaryInsertError::IndexReserved) for a key with that
//! index, which only a hand-built key can have. The config's
//! [`Storage`](SecondaryMapConfig::Storage) is the collection the slots live
//! in. Like a `GenMap`'s storage, it can be any [`SlotStorage`], so `Vec`,
//! `ArrayVec` and `SmallVec` all work. [`SecondaryMapConfig`] has an example of
//! a config.
//!
//! # Cargo features
//!
//! - `alloc` is on by default. It adds the `Vec` storage,
//!   [`DefaultMapConfig`], [`GenMap::new`] and [`SecondaryMap::new`], and
//!   makes [`DefaultMapConfig`] the config of a `GenMap<T>` and a
//!   `SecondaryMap<T>`. Without it, there is no default config, so every map
//!   needs a config of its own, and the crate needs no allocator unless the
//!   `smallvec` feature is on, since a `SmallVec` allocates.
//! - `arrayvec` lets a [`GenMapConfig`] or a [`SecondaryMapConfig`] use
//!   `arrayvec::ArrayVec` as its storage.
//! - `smallvec` lets a [`GenMapConfig`] or a [`SecondaryMapConfig`] use
//!   `smallvec::SmallVec` as its storage. It
//!   uses the 2.0 beta of `smallvec`, which needs an allocator and Rust 1.86.
//!   Until smallvec 2.0 is released, a newer smallvec beta or a new release
//!   of `gen_map` may break this feature, so it is not covered by semver.
//!
//! To use the crate without any allocator, turn default features off and
//! `arrayvec` on.
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
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(
    missing_docs,
    unsafe_op_in_unsafe_fn,
    clippy::undocumented_unsafe_blocks
)]

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
mod replace_strategy;
mod secondary_map;
mod slot;
mod storage;

#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
pub use config::DefaultMapConfig;
pub use config::{
    DefaultKeyConfig, GenMapConfig, KeyConfig, MapConfig, MapConfigFor, SecondaryMapConfig,
    SecondaryMapConfigFor,
};
pub use error::{
    FullError, GetDisjointMutAtError, GetDisjointMutError, InsertError, InsertWithError,
    SecondaryInsertError,
};
pub use key::Key;
pub use key_layout::{Packed, Split};
pub use key_piece::KeyPiece;
pub use map::{
    Drain, GenMap, IntoIter, Iter, IterMut, Keys, MapGen, MapIdx, MapKeyConfig, MapSlot,
    StorageError, VacantEntry, Values, ValuesMut,
};
pub use parity::{Even, Odd};
pub use replace_strategy::{ExistingWins, NewerWins, ReplaceStrategy};
pub use secondary_map::{
    SecondaryDrain, SecondaryIntoIter, SecondaryIter, SecondaryIterMut, SecondaryKeys,
    SecondaryMap, SecondaryMapSlot, SecondaryStorageError, SecondaryValues, SecondaryValuesMut,
};
pub use slot::{GenSlotItem, Parity, SecondarySlot, SecondarySlotItem, Slot};
pub use storage::{ReserveStorage, SlotStorage};

// The tests use `Vec` storage and the default config, so they need `alloc`.
#[cfg(all(test, feature = "alloc"))]
mod tests;
