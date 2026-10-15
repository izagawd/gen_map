// The tests call the unchecked methods and implement the unsafe traits on
// purpose, so their unsafe code is not documented block by block.
#![allow(clippy::undocumented_unsafe_blocks)]

mod dense;
mod key;
mod len;
mod map;
mod model;
mod panic_fuzz;
mod raw_parts;
mod secondary;
mod slot;
mod storage;
mod zero_sized;

use crate::{
    DenseGenMapConfig, DenseSecondaryMapConfig, GenMap, GenMapConfig, GenSlotItem, KeepLen, Key,
    KeyConfig, KeyPiece, Len, MapConfig, NewerWins, NoLen, Odd, PairVec, SecondaryMap,
    SecondaryMapConfig, SecondarySlotItem, Split,
};
use core::marker::PhantomData;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::vec::Vec;

/// The keys of this config are a [`Split`] of the two [`KeyPiece`] types it is
/// given. Every map except a `SparseSecondaryMap` keeps its slots in a `Vec`,
/// the dense maps keep their keys and values in a `PairVec`, and the secondary
/// maps use `NewerWins`. It is only used as a type parameter, never created as
/// a value.
#[allow(dead_code)]
pub(crate) struct Cfg<Idx, Gen>(PhantomData<(Idx, Gen)>);

impl<Idx: KeyPiece, Gen: KeyPiece> MapConfig for Cfg<Idx, Gen> {
    type KeyConfig = Split<Idx, Gen>;
}

impl<Idx: KeyPiece, Gen: KeyPiece> GenMapConfig for Cfg<Idx, Gen> {
    type Len = KeepLen;
    type Storage<S: GenSlotItem> = Vec<S>;
}

impl<Idx: KeyPiece, Gen: KeyPiece> SecondaryMapConfig for Cfg<Idx, Gen> {
    type ReplaceStrategy = NewerWins;
    type Len = KeepLen;
    type Storage<S: SecondarySlotItem> = Vec<S>;
}

impl<Idx: KeyPiece, Gen: KeyPiece> DenseGenMapConfig for Cfg<Idx, Gen> {
    type SlotStorage<S: GenSlotItem> = Vec<S>;
    type PairStorage<K, V> = PairVec<K, V>;
}

impl<Idx: KeyPiece, Gen: KeyPiece> DenseSecondaryMapConfig for Cfg<Idx, Gen> {
    type ReplaceStrategy = NewerWins;
    type SlotStorage<S: GenSlotItem> = Vec<S>;
    type PairStorage<K, V> = PairVec<K, V>;
}

#[cfg(feature = "std")]
impl<Idx: KeyPiece, Gen: KeyPiece> crate::SparseSecondaryMapConfig for Cfg<Idx, Gen> {
    type ReplaceStrategy = NewerWins;
}

/// Checks that `reattach` refuses `key` because nothing is detached under
/// it, hands `value` back, and leaves the map's length and the generation of
/// the key's slot as they were.
pub(crate) fn assert_not_detached<C: GenMapConfig<Len = KeepLen>>(
    map: &mut crate::GenMap<i32, C>,
    key: Key<crate::MapKeyConfig<C>>,
    value: i32,
) {
    let len = map.len();
    let generation = map.generation_at(key.idx());
    assert_eq!(map.reattach(key, value), Err(value));
    assert_eq!(map.len(), len);
    assert_eq!(map.generation_at(key.idx()), generation);
}

/// The methods of this trait check what a map and its iterators report about
/// the number of values. A map whose config keeps no count has no `len` or
/// `is_empty`, so for such a map they only check the size hints of its
/// iterators.
pub(crate) trait LenCheck: Len + Sized {
    /// Checks that `len` and `is_empty` of `map` agree with `expected`, if
    /// the config keeps a count.
    fn assert_len<T, C: GenMapConfig<Len = Self>>(map: &GenMap<T, C>, expected: usize);

    /// Checks that `len` and `is_empty` of `map` agree with `expected`, if
    /// the config keeps a count.
    fn assert_secondary_len<T, C: SecondaryMapConfig<Len = Self>>(
        map: &SecondaryMap<T, C>,
        expected: usize,
    );

    /// Checks that the size hint `hint` allows `expected` values. If the
    /// config keeps a count, the hint must be exactly `expected`.
    fn assert_hint(hint: (usize, Option<usize>), expected: usize);
}

impl LenCheck for KeepLen {
    fn assert_len<T, C: GenMapConfig<Len = Self>>(map: &GenMap<T, C>, expected: usize) {
        assert_eq!(map.len(), expected);
        assert_eq!(map.is_empty(), expected == 0);
    }

    fn assert_secondary_len<T, C: SecondaryMapConfig<Len = Self>>(
        map: &SecondaryMap<T, C>,
        expected: usize,
    ) {
        assert_eq!(map.len(), expected);
        assert_eq!(map.is_empty(), expected == 0);
    }

    fn assert_hint(hint: (usize, Option<usize>), expected: usize) {
        assert_eq!(hint, (expected, Some(expected)));
    }
}

impl LenCheck for NoLen {
    fn assert_len<T, C: GenMapConfig<Len = Self>>(_map: &GenMap<T, C>, _expected: usize) {}

    fn assert_secondary_len<T, C: SecondaryMapConfig<Len = Self>>(
        _map: &SecondaryMap<T, C>,
        _expected: usize,
    ) {
    }

    fn assert_hint(hint: (usize, Option<usize>), expected: usize) {
        assert_eq!(hint.0, 0);
        assert!(hint.1.is_some_and(|upper| upper >= expected));
    }
}

/// The key of `K` with index `idx` and generation `generation`. Panics if
/// the generation is even or either part does not fit the key config.
pub(crate) fn key_from_parts<K: KeyConfig>(idx: K::Idx, generation: K::Gen) -> Key<K> {
    let generation = Odd::new(generation).unwrap();
    Key::from_repr(K::pack(idx, generation).unwrap())
}

/// Hands out numbered items and records how many times each one has been
/// dropped. A second drop of the same item panics on the spot, so a failure
/// shows up where it happens rather than as a wrong total later.
#[derive(Clone)]
pub(crate) struct DropTracker {
    state: Rc<RefCell<TrackerState>>,
}

struct TrackerState {
    /// How many times each id has been dropped.
    drops: HashMap<u32, u32>,
    /// The id the next item gets.
    next_id: u32,
}

impl DropTracker {
    pub(crate) fn new() -> Self {
        Self {
            state: Rc::new(RefCell::new(TrackerState {
                drops: HashMap::new(),
                next_id: 0,
            })),
        }
    }

    pub(crate) fn make_item(&self) -> DropItem {
        let mut state = self.state.borrow_mut();
        let id = state.next_id;
        state.next_id += 1;
        DropItem {
            id,
            state: self.state.clone(),
        }
    }

    /// The number of distinct items that have been dropped so far.
    pub(crate) fn total_dropped(&self) -> usize {
        self.state.borrow().drops.len()
    }

    /// How many items have been made so far, clones included.
    pub(crate) fn total_made(&self) -> u32 {
        self.state.borrow().next_id
    }

    /// Asserts that every item with an id below `n` was dropped exactly once,
    /// and that no other item was dropped.
    pub(crate) fn assert_all_dropped_exactly_once(&self, n: u32) {
        let state = self.state.borrow();
        for id in 0..n {
            let count = state.drops.get(&id).copied().unwrap_or(0);
            assert_eq!(count, 1, "item {id} was dropped {count} times");
        }
        assert_eq!(state.drops.len(), n as usize);
    }

    pub(crate) fn assert_none_dropped(&self) {
        let dropped = self.total_dropped();
        assert_eq!(
            dropped, 0,
            "expected no drops, but {dropped} items were dropped"
        );
    }
}

pub(crate) struct DropItem {
    id: u32,
    state: Rc<RefCell<TrackerState>>,
}

impl Clone for DropItem {
    /// A clone is a new item with its own id, so a clone and its original are
    /// each expected to drop exactly once.
    fn clone(&self) -> Self {
        let mut state = self.state.borrow_mut();
        let id = state.next_id;
        state.next_id += 1;
        DropItem {
            id,
            state: self.state.clone(),
        }
    }
}

impl Drop for DropItem {
    fn drop(&mut self) {
        let mut state = self.state.borrow_mut();
        let count = state.drops.entry(self.id).or_insert(0);
        *count += 1;
        assert!(*count <= 1, "item {} was dropped {} times", self.id, *count);
    }
}

/// A value whose drop panics when it is armed. It holds a [`DropItem`], and
/// the item is still dropped while the panic unwinds, so a [`DropTracker`]
/// sees every bomb dropped exactly once, whether its drop panicked or not.
/// An armed bomb does not panic while the thread is already panicking, since
/// a second panic would abort the tests.
pub(crate) struct Bomb {
    armed: bool,
    _item: DropItem,
}

impl Bomb {
    pub(crate) fn new(tracker: &DropTracker, armed: bool) -> Self {
        Self {
            armed,
            _item: tracker.make_item(),
        }
    }
}

impl Drop for Bomb {
    fn drop(&mut self) {
        if self.armed && !std::thread::panicking() {
            panic!("boom");
        }
    }
}

/// A value whose `clone` panics when it is armed. It holds a [`DropItem`], and
/// its clone holds a clone of that item, so a [`DropTracker`] sees whether
/// the clones made before a panic are dropped exactly once.
pub(crate) struct CloneBomb {
    armed: bool,
    item: DropItem,
}

impl CloneBomb {
    pub(crate) fn new(tracker: &DropTracker, armed: bool) -> Self {
        Self {
            armed,
            item: tracker.make_item(),
        }
    }
}

impl Clone for CloneBomb {
    fn clone(&self) -> Self {
        assert!(!self.armed, "boom");
        Self {
            armed: false,
            item: self.item.clone(),
        }
    }
}
