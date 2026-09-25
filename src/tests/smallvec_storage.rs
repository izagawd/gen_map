//! Maps whose slots live in a `SmallVec`, which keeps a few slots inline and
//! moves them to the heap once there are more. The randomized model test
//! covers them too.

use super::{Bomb, DropTracker};
use crate::{GenMap, KeyConfig, MapConfig, Split};
use smallvec::SmallVec;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::vec::Vec;

/// Room for four slots inline before the storage moves to the heap.
struct Four;

impl KeyConfig for Four {
    type Idx = u32;
    type Gen = u32;
    type Layout = Split;
}

impl MapConfig for Four {
    type KeyConfig = Self;
    type Storage<S> = SmallVec<S, 4>;
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
fn a_map_in_a_small_vec_can_be_made_in_a_const() {
    const EMPTY: GenMap<u32, Four> = GenMap::new_with_config();
    let mut map = EMPTY;
    let key = map.insert(1);
    assert_eq!(map[key], 1);
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
