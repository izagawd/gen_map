//! [`GenMap`] is a configurable generational map that stores each value in a slot
//! and hands out a [`Key`] that points at that slot.
//!
//! When a value is removed, the map frees its slot and may reuse
//! that slot for a value inserted later. The key of the removed value stops
//! matching the slot when the value is removed, so it does not match the value
//! the map later puts in that slot either. That makes keys safe to hold on to
//! where plain indices or references are not, such as in graphs, entity
//! systems and anything else that refers to values by handle.
//!
//! Inserting, removing and looking up a value are all O(1). Only the `std`
//! feature uses `std`, and [Cargo features](#cargo-features) shows how to use
//! the crate without `std` or without an allocator.
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
//! // The map puts "c" in the slot that "a" was removed from. The key `a`
//! // stopped matching that slot when "a" was removed, so it does not match
//! // "c".
//! let c = map.insert("c");
//! assert_eq!(c.idx(), a.idx());
//! assert!(map.get(a).is_none());
//! assert_eq!(map[c], "c");
//!
//! // The iterator yields each value with its key, in slot order. "c" is in the
//! // first slot, so it comes before "b".
//! let pairs: Vec<_> = map.iter().collect();
//! assert_eq!(pairs, [(c, &"c"), (b, &"b")]);
//! ```
//!
//! A [`SecondaryMap`] stores values under the keys a [`GenMap`] hands out.
//!
//! ```
//! use gen_map::{GenMap, SecondaryMap};
//!
//! let mut people = GenMap::new();
//! let mut ages = SecondaryMap::new();
//! let alice = people.insert("Alice");
//! ages.insert(alice, 30).unwrap();
//! assert_eq!(ages[alice], 30);
//! ```
//!
//! A config decides the size of a map's keys and the collection the map keeps
//! its slots in.
//!
//! ```
//! use gen_map::{GenMap, GenMapConfig, GenSlotItem, MapConfig, Split};
//!
//! /// Maps with this config hand out keys with a `u8` index and a `u8`
//! /// generation, so each key is two bytes. A map with this config never gives
//! /// a slot the index `u8::MAX`, so it holds at most 255 slots.
//! struct Tiny;
//!
//! impl MapConfig for Tiny {
//!     type KeyConfig = Split<u8, u8>;
//! }
//!
//! impl GenMapConfig for Tiny {
//!     type Storage<S: GenSlotItem> = Vec<S>;
//! }
//!
//! let mut map = GenMap::<u64, Tiny>::new_with_config();
//! let key = map.insert(7);
//! assert_eq!(core::mem::size_of_val(&key), 2);
//! assert_eq!(map[key], 7);
//! ```
//!
//! # How it works
//!
//! A key holds the index of its value's slot and the generation that slot had
//! when the key was handed out. A key only matches its slot while the slot's
//! generation equals the key's. A new slot starts at generation zero, and the
//! map adds one to a slot's generation when it inserts a value into the slot
//! and again when [`remove`](GenMap::remove) takes the value out. So the
//! generation is odd while the slot holds a value and even while it does not.
//! Once a value is removed, no copy of its key matches the slot, and the key
//! the map hands out for the next value in that slot has a newer generation.
//!
//! The map keeps a slot after its value is removed, and it reuses freed slots
//! before it adds new ones. Only [`reset`](GenMap::reset) removes slots. The
//! iterators, [`retain`](GenMap::retain), [`drain`](GenMap::drain) and
//! [`clear`](GenMap::clear) go through every slot, including the ones that
//! hold no value, so the time they take follows
//! [`slots_len`](GenMap::slots_len) rather than [`len`](GenMap::len).
//!
//! The map adds one to a slot's generation twice for each value, so a `u32`
//! generation lets a slot hold over two billion values one after another, and
//! a 4-bit generation lets it hold eight. When the map removes a value whose
//! key has the largest generation its key config can hold, the slot has no
//! generations left. By default, the map then retires the slot and never uses
//! it again. When [`WRAP_ON_OVERFLOW`](GenMapConfig::WRAP_ON_OVERFLOW) is
//! `true`, the map instead starts the slot's generation over at zero and keeps
//! using the slot. The map then counts the slot's generation up through the
//! same values again, so a key from before the wrap can match a new value once
//! the slot's generation is back at the key's.
//!
//! Two methods also let a key match a value other than the one it was handed
//! out for. [`reset`](GenMap::reset) removes every slot and starts the
//! generations over, so a key from before the reset can match a value inserted
//! after it. [`reattach`](GenMap::reattach) puts any value under a key whose
//! value [`detach`](GenMap::detach) took out, and the key then matches that
//! value.
//!
//! # Configuring the map
//!
//! A map's config is a type that implements [`MapConfig`] and
//! [`GenMapConfig`]. The map only uses the config as a type and never creates
//! a value of it, so an empty struct is enough. A config decides three things.
//!
//! - [`KeyConfig`](MapConfig::KeyConfig) decides the integer types of a key's
//!   index and generation, and how the key stores the two. [`Split`] keeps
//!   them as two fields, and [`Packed`] puts them in the bits of one integer.
//!   With either key config, `Option<Key>` is the same size as `Key`.
//! - [`WRAP_ON_OVERFLOW`](GenMapConfig::WRAP_ON_OVERFLOW) decides whether the
//!   map retires a slot with no generations left or starts its generation over
//!   at zero.
//! - [`Storage`](GenMapConfig::Storage) is the collection the map keeps its
//!   slots in. It can be a [`LenVec`], a `Vec`, an `ArrayVec`, a `SmallVec`
//!   or any other type that implements [`SliceStorage`].
//!
//! When its `C` parameter is left out, as in `GenMap<T>`, a map uses
//! [`DefaultMapConfig`]. Maps with that config hand out keys with a `u32`
//! index and a `u32` generation, keep their slots in a [`LenVec`] with a `u32`
//! length and retire a slot that has no generations left. [`GenMap::new`] only
//! exists for the default config, so use [`GenMap::new_with_config`] for any
//! other.
//!
//! Maps whose configs have the same key config share a key type. A key does
//! not record which map handed it out, so a map also accepts keys from another
//! map with the same key config, and it may hold an unrelated value under such
//! a key.
//!
//! # Secondary maps
//!
//! Use a [`SecondaryMap`] to add data to the values of a [`GenMap`] without
//! changing their type. A `SecondaryMap` can only use a `GenMap`'s keys when
//! both configs have the same [`KeyConfig`](MapConfig::KeyConfig), because
//! only then do the two maps share a key type.
//!
//! When a value is removed from the `GenMap`, the value stored under its key
//! stays in the `SecondaryMap`. If the `GenMap` later puts a new value in the
//! removed value's slot, the new value's key has the same index as the old key
//! and a newer generation. The
//! [`ReplaceStrategy`](SecondaryMapConfig::ReplaceStrategy) of the
//! `SecondaryMap`'s config decides whether a value inserted under that newer
//! key replaces the old value.
//!
//! A `SecondaryMap` keeps a slot at every index up to the highest index that
//! an insert has used. A [`SparseSecondaryMap`] keeps its values in a `HashMap`
//! under the indices of their keys instead, so it uses less memory when only
//! a few of a `GenMap`'s keys have a value in it. Its lookups hash the key's
//! index, so they take longer than a `SecondaryMap`'s. It needs the `std`
//! feature.
//!
//! # Dense maps
//!
//! A [`DenseGenMap`] works like a [`GenMap`], but it keeps its values one
//! after another in a storage of their own, and each slot only stores the
//! position of its value. Iterating over the values is then as fast as
//! iterating over a slice, and a lookup takes one more step. A
//! [`DenseSecondaryMap`] does the same for a [`SecondaryMap`].
//!
//! Removing a value from a dense map moves its last value into the place of
//! the removed one. The map reads the moved value's key to point that value's
//! slot at its new position, so a dense map also keeps the key of each value.
//! It keeps the keys and values in a [`PairStorage`], which its config picks.
//! The default config picks a [`PairVec`], which keeps the keys and the
//! values in two buffers that share one length and one capacity.
//!
//! ```
//! use gen_map::DenseGenMap;
//!
//! let mut map = DenseGenMap::new();
//! let a = map.insert(1);
//! map.insert(2);
//! map.insert(3);
//! map.remove(a);
//! assert_eq!(map.values().copied().collect::<Vec<_>>(), [3, 2]);
//! ```
//!
//! # Cargo features
//!
//! - `std` is on by default, and it turns `alloc` on too. It adds
//!   [`SparseSecondaryMap`], which keeps its values in std's `HashMap`.
//! - `alloc` adds the [`LenVec`], `Vec` and [`PairVec`] storages and
//!   [`DefaultMapConfig`]. Without it, every map needs a config of its own.
//! - `arrayvec` lets a config use `arrayvec::ArrayVec` as a storage. An
//!   `ArrayVec` has a fixed capacity and never allocates.
//! - `smallvec` lets a config use `smallvec::SmallVec` as a storage. A
//!   `SmallVec` keeps its first items inline and allocates when it needs room
//!   for more. This feature uses the 2.0 beta of `smallvec`. Until `smallvec`
//!   2.0 is released, a newer beta or a new release of `gen_map` may break
//!   this feature, so it is not covered by semver.
//!
//! To use the crate without `std`, turn default features off and `alloc` on.
//! The crate only needs an allocator when `alloc` or `smallvec` is on, so to
//! use it without any allocator, turn default features off and `arrayvec` on.
//!
//! ```toml
//! [dependencies]
//! gen_map = { version = "0.3", default-features = false, features = ["arrayvec"] }
//! ```
//!
//! # Minimum supported Rust version
//!
//! The crate builds on Rust 1.86 and later.

#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(
    missing_docs,
    unsafe_op_in_unsafe_fn,
    clippy::undocumented_unsafe_blocks
)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(any(test, feature = "std"))]
extern crate std;

#[cfg(feature = "alloc")]
mod buffer;
mod config;
mod dense_map;
mod dense_secondary_map;
mod error;
mod key;
mod key_layout;
mod key_piece;
#[cfg(feature = "alloc")]
mod len_vec;
mod map;
mod pair_storage;
#[cfg(feature = "alloc")]
mod pair_vec;
mod parity;
mod replace_strategy;
mod secondary_map;
mod slot;
#[cfg(feature = "std")]
mod sparse_secondary_map;
mod storage;

#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
pub use config::DefaultMapConfig;
#[cfg(feature = "std")]
#[cfg_attr(docsrs, doc(cfg(feature = "std")))]
pub use config::SparseSecondaryMapConfig;
pub use config::{
    DefaultKeyConfig, DenseGenMapConfig, DenseSecondaryMapConfig, GenMapConfig, KeyConfig,
    MapConfig, SecondaryMapConfig,
};
pub use dense_map::{
    DenseDrain, DenseGenMap, DenseGenMapRawParts, DenseIntoIter, DenseIter, DenseIterMut,
    DenseKeys, DenseMapSlot, DenseStorageError, DenseVacantEntry, DenseValues, DenseValuesMut,
};
pub use dense_secondary_map::{
    DenseSecondaryDrain, DenseSecondaryMap, DenseSecondaryMapRawParts, DenseSecondaryMapSlot,
    DenseSecondaryStorageError,
};
#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
pub use error::ReserveError;
pub use error::{
    DenseError, FullError, GetDisjointMutAtError, GetDisjointMutError, InsertError,
    InsertWithError, SecondaryInsertError,
};
pub use key::Key;
pub use key_layout::{Packed, Split};
pub use key_piece::KeyPiece;
#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
pub use len_vec::{LenVec, LenVecIntoIter};
pub use map::{
    Drain, GenMap, GenMapRawParts, IntoIter, Iter, IterMut, Keys, MapGen, MapIdx, MapKeyConfig,
    MapSlot, StorageError, VacantEntry, Values, ValuesMut,
};
pub use pair_storage::{
    PairStorage, ReservePairStorage, SplitPair, SplitPairError, SplitPairIntoIter,
};
#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
pub use pair_vec::{PairVec, PairVecIntoIter};
pub use parity::{Even, Odd};
pub use replace_strategy::{ExistingWins, NewerWins, ReplaceStrategy};
pub use secondary_map::{
    SecondaryDrain, SecondaryIntoIter, SecondaryIter, SecondaryIterMut, SecondaryKeys,
    SecondaryMap, SecondaryMapRawParts, SecondaryMapSlot, SecondaryStorageError, SecondaryValues,
    SecondaryValuesMut,
};
pub use slot::{GenSlotItem, Parity, ParityMut, ParityRef, SecondarySlotItem, Slot};
#[cfg(feature = "std")]
#[cfg_attr(docsrs, doc(cfg(feature = "std")))]
pub use sparse_secondary_map::{
    SparseSecondaryDrain, SparseSecondaryIntoIter, SparseSecondaryIter, SparseSecondaryIterMut,
    SparseSecondaryKeys, SparseSecondaryMap, SparseSecondaryMapRawParts, SparseSecondaryMapSlot,
    SparseSecondaryValues, SparseSecondaryValuesMut, SparseSlot,
};
pub use storage::{ReserveStorage, SliceStorage};

// The tests use `Vec` storage and the default config, so they need `alloc`.
#[cfg(all(test, feature = "alloc"))]
mod tests;
