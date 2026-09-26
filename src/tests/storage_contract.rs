//! Checks each built-in storage against what `SlotStorage` promises,
//! without a map, so the methods a map seldom calls are covered too.

use super::{DropItem, DropTracker};
use crate::{ReserveStorage, SlotStorage};
use std::vec::Vec;

/// The ids of the items in `storage`, in order.
fn ids<St: SlotStorage<Item = DropItem>>(storage: &St) -> Vec<u32> {
    storage.as_slice().iter().map(|item| item.id).collect()
}

/// Pushes `count` items and checks their order, then checks that `clear`
/// drops every one of them exactly once. `count` must fit the storage.
fn check_storage<St>(count: u32)
where
    St: SlotStorage<Item = DropItem> + IntoIterator<Item = DropItem>,
    St::IntoIter: DoubleEndedIterator,
{
    let tracker = DropTracker::new();
    let mut storage = St::EMPTY;
    assert!(storage.is_empty());
    assert!(St::with_capacity(count as usize).is_empty());

    // One call to `ensure_room` makes room for every push that follows.
    assert!(storage.ensure_room(count as usize).is_ok());
    for _ in 0..count {
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
fn check_reserve<St: ReserveStorage<Item = u32>>() {
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
    assert!(storage.ensure_room(1).is_err());
    assert!(storage.ensure_room(0).is_ok());

    let item = tracker.make_item();
    let back = SlotStorage::try_push(&mut storage, item).unwrap_err();
    assert_eq!(back.id, 2);
    tracker.assert_none_dropped();
    drop(back);
    drop(storage);
    tracker.assert_all_dropped_exactly_once(3);
}

#[cfg(feature = "arrayvec")]
#[test]
fn an_array_vec_makes_room_only_for_what_fits() {
    let mut storage = arrayvec::ArrayVec::<u32, 4>::EMPTY;
    assert!(storage.try_push(0).is_ok());
    assert!(storage.ensure_room(3).is_ok());
    assert!(storage.ensure_room(4).is_err());
    assert!(storage.ensure_room(usize::MAX).is_err());
}

#[test]
fn a_vec_cannot_make_room_for_usize_max_more_items() {
    let mut storage = Vec::<u32>::EMPTY;
    assert!(storage.ensure_room(usize::MAX).is_err());
    assert_eq!(storage.capacity(), 0);
}

#[cfg(feature = "smallvec")]
#[test]
fn a_small_vec_keeps_the_storage_contract() {
    // Three items fit inline, and six move the storage to the heap.
    check_storage::<smallvec::SmallVec<DropItem, 4>>(3);
    check_storage::<smallvec::SmallVec<DropItem, 4>>(6);
    check_reserve::<smallvec::SmallVec<u32, 4>>();
}
