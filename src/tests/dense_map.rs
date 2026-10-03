//! Tests for `DenseGenMap`. The parts it shares with `GenMap` are checked
//! against a `GenMap` in `dense_model`, so these tests cover the order of the
//! values, the iterators, drops and storage errors.

use super::{Bomb, DropTracker};
use crate::{
    DenseError, DenseGenMap, DenseGenMapConfig, GenSlotItem, InsertWithError, Key, MapConfig, Split,
};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::vec::Vec;
use std::{format, vec};

/// `u8` keys, so a map runs out of indices once it has 255 slots.
struct Small;

impl MapConfig for Small {
    type KeyConfig = Split<u8, u8>;
}

impl DenseGenMapConfig for Small {
    type SlotStorage<S: GenSlotItem> = Vec<S>;
    type ValueStorage<V> = Vec<V>;
    type KeyStorage<K> = Vec<K>;
}

fn values<T: Copy, C: DenseGenMapConfig>(map: &DenseGenMap<T, C>) -> Vec<T> {
    map.values().copied().collect()
}

#[test]
fn removing_a_value_moves_the_last_value_into_its_place() {
    let mut map = DenseGenMap::new();
    let a = map.insert('a');
    let b = map.insert('b');
    let c = map.insert('c');
    let d = map.insert('d');
    assert_eq!(values(&map), ['a', 'b', 'c', 'd']);

    assert_eq!(map.remove(b), Some('b'));
    assert_eq!(values(&map), ['a', 'd', 'c']);
    assert_eq!(map.keys().collect::<Vec<_>>(), [a, d, c]);
    assert_eq!((map[a], map[c], map[d]), ('a', 'c', 'd'));
    assert!(map.get(b).is_none());

    // Removing the last value moves nothing.
    assert_eq!(map.remove(c), Some('c'));
    assert_eq!(values(&map), ['a', 'd']);
    assert_eq!(map.remove(c), None);
    assert_eq!(map.len(), 2);
}

#[test]
fn a_freed_slot_is_reused_under_a_newer_key() {
    let mut map = DenseGenMap::new();
    let a = map.insert(1);
    map.remove(a);
    let b = map.insert(2);
    assert_eq!(b.idx(), a.idx());
    assert!(b.generation() > a.generation());
    assert!(map.get(a).is_none());
    assert_eq!(map[b], 2);
}

#[test]
fn detach_reattach_and_release_move_values_like_remove_and_insert() {
    let mut map = DenseGenMap::new();
    let a = map.insert(1);
    let b = map.insert(2);
    let c = map.insert(3);

    assert_eq!(map.detach(a), Some(1));
    assert_eq!(values(&map), [3, 2]);
    // An insert does not take the detached slot.
    let d = map.insert(4);
    assert_ne!(d.idx(), a.idx());

    // The value goes back at the end of the values.
    map.reattach(a, 10).unwrap();
    assert_eq!(values(&map), [3, 2, 4, 10]);
    assert_eq!(map[a], 10);
    assert_eq!(map.reattach(a, 11), Err(11));

    assert_eq!(map.detach(b), Some(2));
    assert!(map.release(b));
    assert!(!map.release(b));
    assert_eq!(map.reattach(b, 12), Err(12));
    assert_eq!(map.insert(5).idx(), b.idx());
    assert_eq!(map[c], 3);
}

#[test]
fn clear_keeps_the_slots_and_leaves_a_detached_slot_detached() {
    let mut map = DenseGenMap::new();
    let a = map.insert(1);
    let b = map.insert(2);
    map.detach(b).unwrap();
    map.clear();
    assert!(map.is_empty());
    assert_eq!(map.slots_len(), 2);
    assert!(map.get(a).is_none());
    map.reattach(b, 3).unwrap();
    assert_eq!(map[b], 3);
    assert_eq!(values(&map), [3]);

    map.reset();
    assert_eq!(map.slots_len(), 0);
    assert!(map.is_empty());
}

#[test]
fn retain_sees_every_value_once() {
    let mut map = DenseGenMap::new();
    let keys: Vec<Key> = (0..10).map(|i| map.insert(i)).collect();
    let mut seen = Vec::new();
    map.retain(|key, value| {
        seen.push((key, *value));
        *value *= 10;
        *value % 20 == 0
    });
    seen.sort_unstable();
    assert_eq!(seen, keys.iter().copied().zip(0..10).collect::<Vec<_>>());
    let mut kept = values(&map);
    kept.sort_unstable();
    assert_eq!(kept, [0, 20, 40, 60, 80]);
    for (i, key) in keys.iter().enumerate() {
        assert_eq!(map.get(*key).copied(), (i % 2 == 0).then_some(i * 10));
    }
}

#[test]
fn drain_starts_at_the_last_value_and_frees_every_slot() {
    let mut map = DenseGenMap::new();
    let keys: Vec<Key> = (0..4).map(|i| map.insert(i)).collect();
    let drained: Vec<_> = map.drain().collect();
    assert_eq!(
        drained,
        [(keys[3], 3), (keys[2], 2), (keys[1], 1), (keys[0], 0)]
    );
    assert!(map.is_empty());
    assert_eq!(map.slots_len(), 4);

    // Dropping a drain part way removes the rest.
    let keys: Vec<Key> = (0..4).map(|i| map.insert(i)).collect();
    let mut drain = map.drain();
    assert_eq!(drain.len(), 4);
    assert_eq!(drain.next(), Some((keys[3], 3)));
    drop(drain);
    assert!(map.is_empty());
    assert!(keys.iter().all(|key| map.get(*key).is_none()));
}

#[test]
fn iterators_agree_with_each_other_and_run_both_ways() {
    let mut map = DenseGenMap::new();
    let keys: Vec<Key> = (0..5).map(|i| map.insert(i)).collect();
    map.remove(keys[1]);
    let expected = vec![(keys[0], 0), (keys[4], 4), (keys[2], 2), (keys[3], 3)];

    let pairs: Vec<_> = map.iter().map(|(key, value)| (key, *value)).collect();
    assert_eq!(pairs, expected);
    let backwards: Vec<_> = map.iter().rev().map(|(key, value)| (key, *value)).collect();
    assert_eq!(
        backwards,
        expected.iter().rev().copied().collect::<Vec<_>>()
    );
    assert_eq!(map.iter().len(), 4);
    assert_eq!(map.keys().next_back(), Some(keys[3]));
    assert_eq!(map.values().next_back(), Some(&3));

    for (_, value) in map.iter_mut() {
        *value += 100;
    }
    for value in map.values_mut() {
        *value += 100;
    }
    for (_, value) in &mut map {
        *value += 100;
    }
    let shifted: Vec<_> = (&map)
        .into_iter()
        .map(|(key, value)| (key, *value))
        .collect();
    assert_eq!(
        shifted,
        expected
            .iter()
            .map(|&(key, value)| (key, value + 300))
            .collect::<Vec<_>>()
    );

    let owned: Vec<_> = map.clone().into_iter().collect();
    assert_eq!(owned, shifted);
    let mut owned_backwards = map.into_iter();
    assert_eq!(owned_backwards.len(), 4);
    assert_eq!(owned_backwards.next_back(), Some(shifted[3]));
    assert_eq!(owned_backwards.next(), Some(shifted[0]));
    assert_eq!(owned_backwards.len(), 2);
}

#[test]
fn every_value_is_dropped_once() {
    let tracker = DropTracker::new();
    let mut map = DenseGenMap::new();
    let keys: Vec<Key> = (0..12).map(|_| map.insert(tracker.make_item())).collect();

    drop(map.remove(keys[0]));
    drop(map.retire(keys[1]));
    drop(map.detach(keys[2]));
    map.retain(|key, _| key != keys[3]);
    assert_eq!(tracker.total_dropped(), 4);

    let mut drain = map.drain();
    drop(drain.next());
    drop(drain);
    assert_eq!(tracker.total_dropped(), 12);

    for _ in 0..3 {
        map.insert(tracker.make_item());
    }
    map.clear();
    map.insert(tracker.make_item());
    drop(map);
    tracker.assert_all_dropped_exactly_once(16);
}

#[test]
fn a_panicking_drop_in_clear_leaves_an_empty_map() {
    let tracker = DropTracker::new();
    let mut map = DenseGenMap::new();
    let a = map.insert(Bomb::new(&tracker, false));
    map.insert(Bomb::new(&tracker, true));
    map.insert(Bomb::new(&tracker, false));
    assert!(catch_unwind(AssertUnwindSafe(|| map.clear())).is_err());
    assert!(map.is_empty());
    assert!(map.get(a).is_none());
    let b = map.insert(Bomb::new(&tracker, false));
    assert!(map.get(b).is_some());
    drop(map);
    tracker.assert_all_dropped_exactly_once(4);
}

#[test]
fn a_dropped_entry_or_a_rejected_value_inserts_nothing() {
    let mut map = DenseGenMap::new();
    let promised = map.vacant_entry().unwrap().key();
    assert!(map.is_empty());
    let result = map.try_insert_with_key(|_| Err::<u32, _>("no"));
    assert_eq!(result, Err(InsertWithError::Rejected("no")));
    assert_eq!(map.len(), 0);
    let key = map.insert_with_key(|key| key.idx());
    assert_eq!(key, promised);
    assert_eq!(map[key], 0);
}

#[test]
fn a_clone_is_independent_of_the_original() {
    let mut map = DenseGenMap::new();
    let a = map.insert(1);
    let b = map.insert(2);
    let mut copy = map.clone();
    copy.remove(a);
    copy[b] = 20;
    assert_eq!((map[a], map[b]), (1, 2));
    assert_eq!(copy.get(a), None);
    assert_eq!(copy[b], 20);
}

#[test]
fn debug_lists_each_key_with_its_value_in_slot_order() {
    let mut map = DenseGenMap::new();
    let a = map.insert("a");
    let b = map.insert("b");
    let c = map.insert("c");
    map.remove(a);
    assert_eq!(
        format!("{map:?}"),
        format!("{{{b:?}: \"b\", {c:?}: \"c\"}}")
    );
}

#[test]
#[should_panic(expected = "invalid DenseGenMap key")]
fn indexing_with_a_removed_key_panics() {
    let mut map = DenseGenMap::new();
    let key = map.insert(1);
    map.remove(key);
    let _ = map[key];
}

#[test]
fn capacity_and_reserve_cover_every_storage() {
    let mut map = DenseGenMap::<u32>::with_capacity(8);
    assert!(map.capacity() >= 8);
    map.reserve(20);
    assert!(map.capacity() >= 20);
    assert!(map.try_reserve(4).is_ok());
    assert!(matches!(
        map.try_reserve(usize::MAX),
        Err(DenseError::Slots(_))
    ));
}

#[test]
#[should_panic(expected = "DenseGenMap is full")]
fn insert_panics_when_the_keys_run_out() {
    let mut map = DenseGenMap::<u32, Small>::new_with_config();
    for i in 0..256 {
        map.insert(i);
    }
}

#[cfg(feature = "arrayvec")]
mod capped {
    use super::*;
    use crate::InsertError;
    use arrayvec::ArrayVec;

    /// A dense config whose slot, value and key storages hold `S`, `V` and
    /// `K` items.
    struct Caps<const S: usize, const V: usize, const K: usize>;

    impl<const S: usize, const V: usize, const K: usize> MapConfig for Caps<S, V, K> {
        type KeyConfig = Split<u8, u8>;
    }

    impl<const S: usize, const V: usize, const K: usize> DenseGenMapConfig for Caps<S, V, K> {
        type SlotStorage<T: GenSlotItem> = ArrayVec<T, S>;
        type ValueStorage<T> = ArrayVec<T, V>;
        type KeyStorage<T> = ArrayVec<T, K>;
    }

    /// Inserts until the map is full, and returns the map with the storage
    /// that refused the last insert.
    fn fill<const S: usize, const V: usize, const K: usize>(
    ) -> (DenseGenMap<u32, Caps<S, V, K>>, &'static str) {
        let mut map = DenseGenMap::<u32, Caps<S, V, K>>::new_with_config();
        assert_eq!(map.capacity(), S.min(V).min(K));
        for i in 0.. {
            match map.try_insert(i) {
                Ok(_) => {}
                Err(InsertError::StorageFull(value, error)) => {
                    assert_eq!(value, i);
                    let storage = match error {
                        DenseError::Slots(_) => "slots",
                        DenseError::Values(_) => "values",
                        DenseError::Keys(_) => "keys",
                    };
                    return (map, storage);
                }
                Err(InsertError::IndexExhausted(_)) => return (map, "index"),
            }
        }
        unreachable!()
    }

    #[test]
    fn the_error_says_which_storage_is_full() {
        let (map, storage) = fill::<2, 4, 4>();
        assert_eq!((map.len(), storage), (2, "slots"));
        let (map, storage) = fill::<4, 2, 4>();
        assert_eq!((map.len(), storage), (2, "values"));
        let (map, storage) = fill::<4, 4, 2>();
        assert_eq!((map.len(), storage), (2, "keys"));
    }

    #[test]
    fn a_full_map_reuses_a_freed_slot() {
        let (mut map, _) = fill::<3, 3, 3>();
        let key = map.key_at(1).unwrap();
        assert_eq!(map.remove(key), Some(1));
        let next = map.try_insert(9).unwrap();
        assert_eq!(next.idx(), 1);
        assert!(map.try_insert(10).is_err());
    }

    #[test]
    fn reattach_hands_the_value_back_when_the_values_are_full() {
        let mut map = DenseGenMap::<u32, Caps<4, 2, 4>>::new_with_config();
        let a = map.insert(1);
        map.insert(2);
        map.detach(a).unwrap();
        map.insert(3);
        assert_eq!(map.reattach(a, 4), Err(4));
        assert!(map.release(a));
    }
}
