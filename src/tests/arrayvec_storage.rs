//! Maps whose slots live in an `ArrayVec`, which has a fixed capacity and
//! never allocates. The randomized model test covers them too.

use super::{key_from_parts, Bomb, DropTracker};
use crate::{
    FullError, GenMap, GenMapConfig, GenSlotItem, InsertError, InsertWithError, KeyConfig,
    MapConfig, NewerWinsWrapping, Packed, SecondaryInsertError, SecondaryMap, SecondaryMapConfig,
    SecondarySlotItem, Split,
};
use arrayvec::ArrayVec;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::vec::Vec;

/// Room for four slots, with a `u8` index that could address many more.
struct Four;

impl KeyConfig for Four {
    type Idx = u8;
    type Gen = u8;
    type Layout = Split;
}

impl<T> MapConfig<T> for Four {
    type KeyConfig = Self;
}

impl<S: GenSlotItem> GenMapConfig<S> for Four {
    type Storage = ArrayVec<S, 4>;
}

impl<S: SecondarySlotItem> SecondaryMapConfig<S> for Four {
    type ReplaceStrategy = NewerWinsWrapping;
    type Storage = ArrayVec<S, 4>;
}

/// Sixteen slots in the storage and sixteen indices in the keys, so both run
/// out at the same time.
struct Sixteen;

impl KeyConfig for Sixteen {
    type Idx = u8;
    type Gen = u8;
    type Layout = Packed<u8, 4>;
}

impl<T> MapConfig<T> for Sixteen {
    type KeyConfig = Self;
}

impl<S: GenSlotItem> GenMapConfig<S> for Sixteen {
    type Storage = ArrayVec<S, 16>;
}

/// Fills every slot of a [`Four`] map.
fn full_map() -> GenMap<u32, Four> {
    let mut map = GenMap::new_with_config();
    for i in 0..4 {
        map.insert(i);
    }
    map
}

#[test]
fn a_map_in_an_array_vec_works_like_any_other() {
    let mut map = GenMap::<u32, Four>::new_with_config();
    assert_eq!(map.capacity(), 4);
    let keys: Vec<_> = (0..3).map(|i| map.insert(i)).collect();
    assert_eq!(map[keys[1]], 1);

    assert_eq!(map.remove(keys[1]), Some(1));
    assert!(map.get(keys[1]).is_none());
    let again = map.insert(10);
    assert_eq!(again.idx(), keys[1].idx());
    assert!(map.get(keys[1]).is_none());

    let values: Vec<_> = map.values().copied().collect();
    assert_eq!(values, [0, 10, 2]);
    assert_eq!(map.capacity(), 4);
}

#[test]
fn a_full_array_vec_reports_storage_full() {
    let mut map = full_map();
    assert_eq!(map.slots_len(), 4);

    assert!(matches!(
        map.try_insert(4),
        Err(InsertError::StorageFull(4, _))
    ));
    assert!(matches!(map.vacant_entry(), Err(FullError::StorageFull(_))));
    let mut called = false;
    let result = map.try_insert_with_key(|_| {
        called = true;
        Ok::<_, ()>(4)
    });
    assert!(matches!(
        result,
        Err(InsertWithError::Full(FullError::StorageFull(_)))
    ));
    assert!(!called);
    assert_eq!(map.len(), 4);
}

#[test]
#[should_panic(expected = "its storage cannot make room for more than 4 slots")]
fn insert_panics_when_the_array_vec_is_full() {
    let mut map = full_map();
    map.insert(4);
}

#[test]
fn a_freed_slot_is_reused_when_the_array_vec_is_full() {
    let mut map = full_map();
    let key = map.key_at(2).unwrap();
    assert_eq!(map.remove(key), Some(2));

    let again = map.insert(20);
    assert_eq!(again.idx(), 2);
    assert_eq!(map[again], 20);
    assert_eq!(map.slots_len(), 4);
    assert!(map.try_insert(5).is_err());
}

#[test]
fn the_index_is_reported_when_it_runs_out_with_the_array_vec() {
    let mut map = GenMap::<u32, Sixteen>::new_with_config();
    for i in 0..16 {
        map.insert(i);
    }
    assert!(matches!(
        map.try_insert(16),
        Err(InsertError::IndexExhausted(16))
    ));
}

#[test]
fn clear_and_reset_empty_an_array_vec() {
    let mut map = full_map();
    let old = map.key_at(0).unwrap();

    map.clear();
    assert!(map.is_empty());
    assert_eq!(map.slots_len(), 4);
    assert!(map.get(old).is_none());

    map.reset();
    assert_eq!(map.slots_len(), 0);
    let keys: Vec<_> = (0..4).map(|i| map.insert(i)).collect();
    assert_eq!(keys.len(), 4);
    assert!(map.try_insert(4).is_err());
}

#[test]
fn values_in_an_array_vec_are_dropped_exactly_once() {
    let tracker = DropTracker::new();
    let mut map = GenMap::<_, Four>::new_with_config();
    let keys: Vec<_> = (0..4).map(|_| map.insert(tracker.make_item())).collect();
    drop(map.remove(keys[0]));

    let mut copy = map.clone();
    copy.clone_from(&map);
    let mut iter = copy.into_iter();
    drop(iter.next());
    drop(iter);

    map.retain(|key, _| key != keys[1]);
    drop(map.drain().next());
    drop(map);
    tracker.assert_all_dropped_exactly_once(tracker.total_made());
}

#[test]
fn an_array_vec_drops_every_value_once_when_a_drop_panics() {
    let tracker = DropTracker::new();
    let mut map = GenMap::<_, Four>::new_with_config();
    for i in 0..4 {
        map.insert(Bomb::new(&tracker, i == 1));
    }
    assert!(catch_unwind(AssertUnwindSafe(move || drop(map))).is_err());
    tracker.assert_all_dropped_exactly_once(4);

    let tracker = DropTracker::new();
    let mut map = GenMap::<_, Four>::new_with_config();
    for i in 0..4 {
        map.insert(Bomb::new(&tracker, i == 2));
    }
    let mut iter = map.into_iter();
    drop(iter.next());
    assert!(catch_unwind(AssertUnwindSafe(move || drop(iter))).is_err());
    tracker.assert_all_dropped_exactly_once(4);
}

#[test]
fn a_secondary_map_in_an_array_vec_holds_the_first_four_indices() {
    let mut map = SecondaryMap::<u32, Four>::new_with_config();
    assert_eq!(map.capacity(), 4);
    for i in 0..4 {
        let key = key_from_parts::<Four>(i, 1);
        assert_eq!(map.insert(key, u32::from(i)).unwrap(), None);
    }
    assert_eq!(map.slots_len(), 4);

    // The fifth index does not fit, and the map stays as it was.
    let fifth = key_from_parts::<Four>(4, 1);
    assert!(matches!(
        map.insert(fifth, 4),
        Err(SecondaryInsertError::StorageFull(4, _))
    ));
    assert_eq!(map.len(), 4);
    assert_eq!(map.slots_len(), 4);

    // The storage's own iterator runs from both ends, so the map's does too.
    let backward: Vec<u8> = map
        .clone()
        .into_iter()
        .rev()
        .map(|(key, _)| key.idx())
        .collect();
    assert_eq!(backward, [3, 2, 1, 0]);
    let pairs: Vec<(u8, u32)> = map
        .into_iter()
        .map(|(key, value)| (key.idx(), value))
        .collect();
    assert_eq!(pairs, [(0, 0), (1, 1), (2, 2), (3, 3)]);
}

#[test]
fn a_secondary_map_in_an_array_vec_adds_no_slots_for_an_index_that_does_not_fit() {
    let mut map = SecondaryMap::<u32, Four>::new_with_config();
    let far = key_from_parts::<Four>(200, 1);
    assert!(matches!(
        map.insert(far, 1),
        Err(SecondaryInsertError::StorageFull(1, _))
    ));
    assert_eq!(map.slots_len(), 0);

    // The storage still has room for all four slots.
    let last = key_from_parts::<Four>(3, 1);
    assert_eq!(map.insert(last, 3).unwrap(), None);
    assert_eq!(map.slots_len(), 4);
    assert_eq!(map[last], 3);
}
