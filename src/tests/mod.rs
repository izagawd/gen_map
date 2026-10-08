// The tests call the unchecked methods and implement the unsafe traits on
// purpose, so their unsafe code is not documented block by block.
#![allow(clippy::undocumented_unsafe_blocks)]

mod dense;
mod key;
mod map;
mod model;
mod panic_fuzz;
mod raw_parts;
mod secondary;
// The `size` tests expect the sizes that types have on 64-bit targets.
#[cfg(target_pointer_width = "64")]
mod size;
mod slot;
mod storage;
mod zero_sized;

use crate::{
    DenseGenMapConfig, DenseSecondaryMapConfig, GenMapConfig, GenSlotItem, Key, KeyConfig,
    KeyPiece, MapConfig, NewerWins, Odd, PairVec, SecondaryMapConfig, SecondarySlotItem, Split,
};
use core::marker::PhantomData;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::vec::Vec;

/// The keys of this config are a [`Split`] of the two integer types it is
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
    type Storage<S: GenSlotItem> = Vec<S>;
}

impl<Idx: KeyPiece, Gen: KeyPiece> SecondaryMapConfig for Cfg<Idx, Gen> {
    type ReplaceStrategy = NewerWins;
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
pub(crate) fn assert_not_detached<C: crate::GenMapConfig>(
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
