//! Checks each built-in storage against what `SlotStorage` promises,
//! without a map, so the methods a map seldom calls are covered too.

use super::{DropItem, DropTracker};
use crate::{ReserveStorage, SlotStorage};
use std::vec::Vec;

/// The ids of the items in `storage`, in order.
fn ids<St: SlotStorage<DropItem>>(storage: &St) -> Vec<u32> {
    storage.as_slice().iter().map(|item| item.id).collect()
}

/// Pushes `count` items and checks their order, then checks that `clear`
/// drops every one of them exactly once. `count` must fit the storage.
fn check_storage<St: SlotStorage<DropItem>>(count: u32) {
    let tracker = DropTracker::new();
    let mut storage = St::EMPTY;
    assert!(storage.is_empty());
    assert!(St::with_capacity(count as usize).is_empty());

    for _ in 0..count {
        assert!(storage.ensure_room().is_ok());
        assert!(storage.try_push(tracker.make_item()).is_ok());
    }
    assert_eq!(storage.len(), count as usize);
    assert!(storage.capacity() >= count as usize);
    assert_eq!(ids(&storage), (0..count).collect::<Vec<_>>());

    storage.as_mut_slice().swap(0, 1);
    assert_eq!(ids(&storage)[..2], [1, 0]);
    storage.as_mut_slice().swap(0, 1);
    tracker.assert_none_dropped();

    storage.clear();
    assert!(storage.is_empty());
    tracker.assert_all_dropped_exactly_once(count);

    for _ in 0..3 {
        assert!(storage.try_push(tracker.make_item()).is_ok());
    }
    let backwards: Vec<_> = storage.into_iter().rev().map(|item| item.id).collect();
    assert_eq!(backwards, [count + 2, count + 1, count]);
    tracker.assert_all_dropped_exactly_once(count + 3);
}

/// Checks that after `try_reserve` makes room for `n` more items, the next
/// `n` pushes succeed.
fn check_reserve<St: ReserveStorage<u32>>() {
    let mut storage = St::EMPTY;
    assert!(storage.try_reserve(10).is_ok());
    assert!(storage.capacity() >= 10);
    for i in 0..10 {
        assert!(storage.try_push(i).is_ok());
    }
    storage.reserve(20);
    assert!(storage.capacity() >= 30);
}

#[test]
fn a_vec_keeps_the_storage_contract() {
    check_storage::<Vec<DropItem>>(6);
    check_reserve::<Vec<u32>>();
}

#[cfg(feature = "arrayvec")]
#[test]
fn an_array_vec_keeps_the_storage_contract() {
    check_storage::<arrayvec::ArrayVec<DropItem, 8>>(6);
    check_storage::<arrayvec::ArrayVec<DropItem, 6>>(6);
}

#[cfg(feature = "arrayvec")]
#[test]
fn a_full_array_vec_hands_the_item_back() {
    let tracker = DropTracker::new();
    let mut storage = arrayvec::ArrayVec::<DropItem, 2>::EMPTY;
    for _ in 0..2 {
        assert!(storage.try_push(tracker.make_item()).is_ok());
    }
    assert_eq!(SlotStorage::capacity(&storage), 2);
    assert!(storage.ensure_room().is_err());

    let item = tracker.make_item();
    let back = SlotStorage::try_push(&mut storage, item).unwrap_err();
    assert_eq!(back.id, 2);
    tracker.assert_none_dropped();
    drop(back);
    drop(storage);
    tracker.assert_all_dropped_exactly_once(3);
}

#[cfg(feature = "smallvec")]
#[test]
fn a_small_vec_keeps_the_storage_contract() {
    // Three items fit inline, and six move the storage to the heap.
    check_storage::<smallvec::SmallVec<DropItem, 4>>(3);
    check_storage::<smallvec::SmallVec<DropItem, 4>>(6);
    check_reserve::<smallvec::SmallVec<u32, 4>>();
}
