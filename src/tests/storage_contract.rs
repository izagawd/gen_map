//! Checks each built-in storage against what `SliceStorage` promises,
//! without a map, so the methods a map seldom calls are covered too.

use super::{DropItem, DropTracker};
use crate::{ReserveStorage, SingleVec, SliceStorage};
use std::vec::Vec;

/// The ids of the items in `storage`, in order.
fn ids<St: SliceStorage<Item = DropItem>>(storage: &St) -> Vec<u32> {
    storage.as_slice().iter().map(|item| item.id).collect()
}

/// Pushes `count` items and checks their order, then checks that `clear`
/// drops every one of them exactly once. `count` must fit the storage.
fn check_storage<St>(count: u32)
where
    St: SliceStorage<Item = DropItem> + IntoIterator<Item = DropItem>,
    St::IntoIter: DoubleEndedIterator,
{
    let tracker = DropTracker::new();
    let mut storage = St::empty();
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

/// Checks that `pop` takes out the last item, and that a storage with no
/// items hands back `None` and stays empty.
fn check_pop<St: SliceStorage<Item = u32>>() {
    let mut storage = St::empty();
    assert_eq!(storage.pop(), None);
    for i in 0..3 {
        assert!(storage.try_push(i).is_ok());
    }
    storage.as_mut_slice().swap(0, 2);
    assert_eq!(storage.pop(), Some(0));
    assert_eq!(storage.as_slice(), [2, 1]);
    assert_eq!(storage.pop(), Some(1));
    assert_eq!(storage.pop(), Some(2));
    assert_eq!(storage.pop(), None);
    assert!(storage.is_empty());
}

/// Checks that a storage marked as one that can grow makes room for more
/// items than it was created with, and that the pushes after that succeed.
fn check_growth<St: ReserveStorage<Item = u32>>() {
    let mut storage = St::with_capacity(2);
    let wanted = storage.capacity() + 10;
    assert!(storage.ensure_room(wanted).is_ok());
    assert!(storage.capacity() >= wanted);
    for item in (0u32..).take(wanted) {
        assert!(storage.try_push(item).is_ok());
    }
}

#[test]
fn a_vec_keeps_the_storage_contract() {
    check_storage::<Vec<DropItem>>(6);
    check_growth::<Vec<u32>>();
    check_pop::<Vec<u32>>();
}

#[test]
fn a_single_vec_keeps_the_storage_contract() {
    check_storage::<SingleVec<DropItem>>(6);
    check_storage::<SingleVec<DropItem, u8>>(6);
    check_growth::<SingleVec<u32>>();
    check_growth::<SingleVec<u32, u16>>();
    check_pop::<SingleVec<u32, u8>>();
}

#[cfg(feature = "arrayvec")]
#[test]
fn an_array_vec_keeps_the_storage_contract() {
    check_storage::<arrayvec::ArrayVec<DropItem, 8>>(6);
    check_storage::<arrayvec::ArrayVec<DropItem, 6>>(6);
    check_pop::<arrayvec::ArrayVec<u32, 3>>();
}

#[cfg(feature = "arrayvec")]
#[test]
fn a_full_array_vec_hands_the_item_back() {
    let tracker = DropTracker::new();
    let mut storage = arrayvec::ArrayVec::<DropItem, 2>::empty();
    for _ in 0..2 {
        assert!(storage.try_push(tracker.make_item()).is_ok());
    }
    assert_eq!(SliceStorage::capacity(&storage), 2);
    assert!(storage.ensure_room(1).is_err());
    assert!(storage.ensure_room(0).is_ok());

    let item = tracker.make_item();
    let back = SliceStorage::try_push(&mut storage, item).unwrap_err();
    assert_eq!(back.id, 2);
    tracker.assert_none_dropped();
    drop(back);
    drop(storage);
    tracker.assert_all_dropped_exactly_once(3);
}

#[cfg(feature = "arrayvec")]
#[test]
fn an_array_vec_makes_room_only_for_what_fits() {
    let mut storage = arrayvec::ArrayVec::<u32, 4>::empty();
    assert!(storage.try_push(0).is_ok());
    assert!(storage.ensure_room(3).is_ok());
    assert!(storage.ensure_room(4).is_err());
    assert!(storage.ensure_room(usize::MAX).is_err());
}

#[test]
fn a_vec_cannot_make_room_for_usize_max_more_items() {
    let mut storage = Vec::<u32>::empty();
    assert!(storage.ensure_room(usize::MAX).is_err());
    assert_eq!(storage.capacity(), 0);
}

#[cfg(feature = "smallvec")]
#[test]
fn a_small_vec_keeps_the_storage_contract() {
    // Three items fit inline, and six move the storage to the heap.
    check_storage::<smallvec::SmallVec<DropItem, 4>>(3);
    check_storage::<smallvec::SmallVec<DropItem, 4>>(6);
    check_growth::<smallvec::SmallVec<u32, 4>>();
    check_pop::<smallvec::SmallVec<u32, 2>>();
}

/// An item whose drop panics if it was made with `true`.
struct Bomb(bool);

impl Drop for Bomb {
    fn drop(&mut self) {
        if self.0 {
            panic!("the drop of this item panics");
        }
    }
}

/// Pushes `count` items, the second of which panics when it is dropped, and
/// checks that `clear` leaves the storage empty although that drop panics.
/// `count` must fit the storage and be at least two.
fn check_clear_with_a_panicking_drop<St: SliceStorage<Item = Bomb>>(count: usize) {
    let mut storage = St::empty();
    for i in 0..count {
        assert!(storage.try_push(Bomb(i == 1)).is_ok());
    }
    let cleared = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| storage.clear()));
    assert!(cleared.is_err());
    assert!(storage.is_empty());
}

#[test]
fn clear_leaves_each_built_in_storage_empty_when_a_drop_panics() {
    check_clear_with_a_panicking_drop::<Vec<Bomb>>(3);
    check_clear_with_a_panicking_drop::<SingleVec<Bomb, u8>>(3);
    #[cfg(feature = "arrayvec")]
    check_clear_with_a_panicking_drop::<arrayvec::ArrayVec<Bomb, 4>>(3);
    // Three items fit inline, and six move the storage to the heap.
    #[cfg(feature = "smallvec")]
    check_clear_with_a_panicking_drop::<smallvec::SmallVec<Bomb, 4>>(3);
    #[cfg(feature = "smallvec")]
    check_clear_with_a_panicking_drop::<smallvec::SmallVec<Bomb, 4>>(6);
}
