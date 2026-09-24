#[cfg(feature = "arrayvec")]
mod arrayvec_storage;
mod basic;
mod capped_storage;
mod clone;
mod detach;
mod disjoint;
mod drain;
mod get_at;
mod iter;
mod key;
mod key_at;
mod key_piece;
mod model;
mod overflow;
mod packed;
mod reset;
mod retain;
#[cfg(feature = "smallvec")]
mod smallvec_storage;
mod storage_contract;
mod try_insert;
mod unchecked;
mod vacant_entry;
mod zero_sized;

use crate::{Config, KeyPiece, Split};
use core::marker::PhantomData;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::vec::Vec;

/// A config made of the two integer types it is given. It is only used as a
/// type parameter, never created as a value.
#[allow(dead_code)]
pub(crate) struct Cfg<Idx, Gen>(PhantomData<(Idx, Gen)>);

impl<Idx: KeyPiece, Gen: KeyPiece> Config for Cfg<Idx, Gen> {
    type Idx = Idx;
    type Gen = Gen;
    type Layout = Split;
    type Storage<S> = Vec<S>;
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
