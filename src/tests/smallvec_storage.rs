//! Maps whose storages are `SmallVec`s, which keep a few items inline and
//! move them to the heap once there are more. The randomized model tests
//! cover them too.

use super::{key_from_parts, Bomb, DropTracker};
use crate::{
    DenseGenMap, DenseGenMapConfig, DenseSecondaryMap, DenseSecondaryMapConfig, GenMap,
    GenMapConfig, GenSlotItem, MapConfig, NewerWins, SecondaryMap, SecondaryMapConfig,
    SecondarySlotItem, Split,
};
use smallvec::SmallVec;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::vec::Vec;

/// Room for four items inline in each storage before it moves to the heap.
struct Four;

impl MapConfig for Four {
    type KeyConfig = Split<u32, u32>;
}

impl GenMapConfig for Four {
    type Storage<S: GenSlotItem> = SmallVec<S, 4>;
}

impl SecondaryMapConfig for Four {
    type ReplaceStrategy = NewerWins;
    type Storage<S: SecondarySlotItem> = SmallVec<S, 4>;
}

impl DenseGenMapConfig for Four {
    type SlotStorage<S: GenSlotItem> = SmallVec<S, 4>;
    type ValueStorage<V> = SmallVec<V, 4>;
    type KeyStorage<K> = SmallVec<K, 4>;
}

impl DenseSecondaryMapConfig for Four {
    type ReplaceStrategy = NewerWins;
    type SlotStorage<S: GenSlotItem> = SmallVec<S, 4>;
    type ValueStorage<V> = SmallVec<V, 4>;
    type KeyStorage<K> = SmallVec<K, 4>;
}

#[test]
fn dense_maps_in_small_vecs_keep_working_after_they_move_to_the_heap() {
    let mut map = DenseGenMap::<u32, Four>::new_with_config();
    assert_eq!(map.capacity(), 4);
    let keys: Vec<_> = (0..20).map(|i| map.insert(i)).collect();
    assert!(map.capacity() >= 20);
    assert_eq!(map.remove(keys[7]), Some(7));
    assert_eq!(map[keys[19]], 19);
    map.reserve(50);
    assert!(map.capacity() >= 69);

    let mut secondary = DenseSecondaryMap::<u32, Four>::new_with_config();
    for (key, value) in &map {
        secondary.insert(key, *value * 10).unwrap();
    }
    assert_eq!(secondary.len(), 19);
    assert_eq!(secondary[keys[19]], 190);

    let owned: Vec<_> = map.into_iter().rev().collect();
    assert_eq!(owned.len(), 19);
    assert_eq!(owned[0], (keys[18], 18));
}

#[test]
fn a_map_in_a_small_vec_keeps_working_after_it_moves_to_the_heap() {
    let mut map = GenMap::<u32, Four>::new_with_config();
    assert_eq!(map.capacity(), 4);
    let keys: Vec<_> = (0..20).map(|i| map.insert(i)).collect();
    assert!(map.capacity() >= 20);

    for (i, key) in keys.iter().enumerate() {
        assert_eq!(map[*key], i as u32);
    }
    assert_eq!(map.remove(keys[7]), Some(7));
    let again = map.insert(70);
    assert_eq!(again.idx(), keys[7].idx());
    assert!(map.get(keys[7]).is_none());
    assert_eq!(map.len(), 20);
}

#[test]
fn a_small_vec_can_make_room_ahead_of_time() {
    let map = GenMap::<u32, Four>::with_capacity_and_config(64);
    assert!(map.capacity() >= 64);

    let mut map = GenMap::<u32, Four>::new_with_config();
    map.reserve(10);
    assert!(map.capacity() >= 10);
    assert!(map.try_reserve(100).is_ok());
    assert!(map.capacity() >= 100);
    assert!(map.try_reserve(usize::MAX).is_err());
}

#[test]
fn reset_keeps_the_heap_storage_of_a_small_vec() {
    let mut map = GenMap::<u32, Four>::new_with_config();
    for i in 0..20 {
        map.insert(i);
    }
    let capacity = map.capacity();
    map.reset();
    assert_eq!(map.slots_len(), 0);
    assert_eq!(map.capacity(), capacity);
    let key = map.insert(1);
    assert_eq!((key.idx(), key.generation().get().get()), (0, 1));
}

/// Runs the same drops on a map that fits inline and on one that moved to
/// the heap.
#[test]
fn values_in_a_small_vec_are_dropped_exactly_once() {
    for count in [3, 10] {
        let tracker = DropTracker::new();
        let mut map = GenMap::<_, Four>::new_with_config();
        let keys: Vec<_> = (0..count)
            .map(|_| map.insert(tracker.make_item()))
            .collect();
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
}

#[test]
fn a_small_vec_drops_every_value_once_when_a_drop_panics() {
    for count in [3, 10] {
        let tracker = DropTracker::new();
        let mut map = GenMap::<_, Four>::new_with_config();
        for i in 0..count {
            map.insert(Bomb::new(&tracker, i == 1));
        }
        assert!(catch_unwind(AssertUnwindSafe(move || drop(map))).is_err());
        tracker.assert_all_dropped_exactly_once(count);
    }
}

#[test]
fn a_small_vec_iterator_drops_every_value_once_when_a_drop_panics() {
    for count in [3, 10] {
        let tracker = DropTracker::new();
        let mut map = GenMap::<_, Four>::new_with_config();
        for i in 0..count {
            map.insert(Bomb::new(&tracker, i == 1));
        }
        let mut iter = map.into_iter();
        drop(iter.next());
        assert!(catch_unwind(AssertUnwindSafe(move || drop(iter))).is_err());
        tracker.assert_all_dropped_exactly_once(count);
    }
}

#[test]
fn a_secondary_map_in_a_small_vec_keeps_working_after_it_moves_to_the_heap() {
    let mut map = SecondaryMap::<u32, Four>::new_with_config();
    assert_eq!(map.capacity(), 4);
    // The keys use every third index, so the map holds empty slots between
    // them.
    let keys: Vec<_> = (0..20)
        .map(|i| key_from_parts::<Split<u32, u32>>(i * 3, 1))
        .collect();
    for (i, &key) in keys.iter().enumerate() {
        assert_eq!(map.insert(key, i as u32).unwrap(), None);
    }
    assert_eq!(map.slots_len(), 58);
    for (i, &key) in keys.iter().enumerate() {
        assert_eq!(map[key], i as u32);
    }
    assert_eq!(map.remove(keys[7]), Some(7));
    let values: Vec<u32> = map.into_iter().map(|(_, value)| value).collect();
    assert_eq!(values.len(), 19);
    assert!(!values.contains(&7));
}

#[test]
fn a_secondary_map_in_a_small_vec_can_make_room_ahead_of_time() {
    let map = SecondaryMap::<u32, Four>::with_capacity_and_config(20);
    assert!(map.capacity() >= 20);

    let mut map = SecondaryMap::<u32, Four>::new_with_config();
    map.reserve(64);
    let capacity = map.capacity();
    assert!(capacity >= 64);
    assert!(map.try_reserve(8).is_ok());

    let key = key_from_parts::<Split<u32, u32>>(63, 1);
    map.insert(key, 1).unwrap();
    assert_eq!(map[key], 1);
    assert_eq!(map.slots_len(), 64);

    // The storage's own iterator runs from both ends, so the map's does too.
    map.insert(key_from_parts::<Split<u32, u32>>(2, 1), 2)
        .unwrap();
    let backward: Vec<u32> = map.into_iter().rev().map(|(_, value)| value).collect();
    assert_eq!(backward, [1, 2]);
}
