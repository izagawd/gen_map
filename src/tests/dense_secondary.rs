//! Tests for `DenseSecondaryMap`. The parts it shares with `SecondaryMap` are
//! checked against a `SecondaryMap` in `dense_model`, so these tests cover
//! what that check cannot, such as the order of the values, drops, panics and
//! storage errors.

use super::{key_from_parts, Bomb, DropTracker};
use crate::{
    DenseError, DenseSecondaryMap, DenseSecondaryMapConfig, GenMap, GenSlotItem, Key, MapConfig,
    NewerWins, SecondaryInsertError, Split,
};
use std::format;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::vec::Vec;

/// `u8` keys, with every storage in a `Vec`.
struct Keyed;

impl MapConfig for Keyed {
    type KeyConfig = Split<u8, u8>;
}

impl DenseSecondaryMapConfig for Keyed {
    type ReplaceStrategy = NewerWins;
    type SlotStorage<S: GenSlotItem> = Vec<S>;
    type ValueStorage<V> = Vec<V>;
    type KeyStorage<K> = Vec<K>;
}

/// Hands out `n` keys from a `GenMap`.
fn keys(n: usize) -> Vec<Key> {
    let mut map = GenMap::new();
    (0..n).map(|_| map.insert(())).collect()
}

#[test]
fn removing_a_value_moves_the_last_value_into_its_place() {
    let k = keys(4);
    let mut map = DenseSecondaryMap::new();
    for (i, key) in k.iter().enumerate() {
        assert_eq!(map.insert(*key, i).unwrap(), None);
    }
    assert_eq!(map.remove(k[0]), Some(0));
    assert_eq!(map.values().copied().collect::<Vec<_>>(), [3, 1, 2]);
    assert_eq!(map.keys().collect::<Vec<_>>(), [k[3], k[1], k[2]]);
    assert_eq!((map[k[1]], map[k[2]], map[k[3]]), (1, 2, 3));
    assert_eq!(map.remove(k[0]), None);
    // The slot stays without a value.
    assert_eq!(map.slots_len(), 4);
    assert_eq!(map.key_at(0), None);
}

#[test]
fn a_value_replaced_under_a_newer_key_keeps_its_place() {
    let old = key_from_parts::<Split<u8, u8>>(0, 1);
    let new = key_from_parts::<Split<u8, u8>>(0, 3);
    let other = key_from_parts::<Split<u8, u8>>(1, 1);
    let mut keyed = DenseSecondaryMap::<&str, Keyed>::new_with_config();
    keyed.insert(old, "old").unwrap();
    keyed.insert(other, "other").unwrap();
    assert_eq!(keyed.insert(new, "new").unwrap(), Some("old"));
    assert_eq!(
        keyed.iter().collect::<Vec<_>>(),
        [(new, &"new"), (other, &"other")]
    );
    assert_eq!(keyed.get(old), None);
    assert!(matches!(
        keyed.insert(old, "older"),
        Err(SecondaryInsertError::Refused("older"))
    ));
    // The new key moved in with the value, so removing under it works.
    assert_eq!(keyed.remove(new), Some("new"));
    assert_eq!(keyed.iter().collect::<Vec<_>>(), [(other, &"other")]);
}

#[test]
fn the_largest_index_is_reserved() {
    let mut map = DenseSecondaryMap::<u32, Keyed>::new_with_config();
    let key = key_from_parts::<Split<u8, u8>>(u8::MAX, 1);
    assert!(matches!(
        map.insert(key, 1),
        Err(SecondaryInsertError::IndexReserved(1))
    ));
    assert!(map.is_empty());
    assert_eq!(map.slots_len(), 0);
}

#[test]
fn retain_and_drain_see_every_value_once() {
    let k = keys(6);
    let mut map = DenseSecondaryMap::new();
    for (i, key) in k.iter().enumerate() {
        map.insert(*key, i).unwrap();
    }
    let mut seen = Vec::new();
    map.retain(|key, value| {
        seen.push(key);
        *value % 2 == 1
    });
    seen.sort_unstable();
    assert_eq!(seen, k);
    assert_eq!(map.len(), 3);

    let drained: Vec<_> = map.drain().collect();
    // Each rejected value made room for the last one, which leaves the
    // values in the order 5, 1, 3.
    assert_eq!(drained, [(k[3], 3), (k[1], 1), (k[5], 5)]);
    assert!(map.is_empty());
    assert_eq!(map.slots_len(), 6);
}

#[test]
fn every_value_is_dropped_once() {
    let tracker = DropTracker::new();
    let k = keys(8);
    let mut map = DenseSecondaryMap::new();
    for key in &k {
        map.insert(*key, tracker.make_item()).unwrap();
    }
    drop(map.insert(k[0], tracker.make_item()));
    drop(map.remove(k[1]));
    map.retain(|key, _| key != k[2]);
    let mut drain = map.drain();
    drop(drain.next());
    drop(drain);
    assert_eq!(tracker.total_dropped(), 9);
    map.insert(k[3], tracker.make_item()).unwrap();
    map.clear();
    map.insert(k[4], tracker.make_item()).unwrap();
    let copy = map.clone();
    drop(map);
    drop(copy);
    tracker.assert_all_dropped_exactly_once(12);
}

#[test]
fn iterators_debug_and_into_iter_follow_the_values() {
    let k = keys(3);
    let mut map = DenseSecondaryMap::new();
    for (i, key) in k.iter().enumerate() {
        map.insert(*key, i).unwrap();
    }
    map.remove(k[0]);
    for (_, value) in &mut map {
        *value += 10;
    }
    let pairs: Vec<_> = (&map)
        .into_iter()
        .map(|(key, value)| (key, *value))
        .collect();
    assert_eq!(pairs, [(k[2], 12), (k[1], 11)]);
    assert_eq!(
        format!("{map:?}"),
        format!("{{{:?}: 11, {:?}: 12}}", k[1], k[2])
    );
    let owned: Vec<_> = map.clone().into_iter().rev().collect();
    assert_eq!(owned, [(k[1], 11), (k[2], 12)]);
}

#[test]
fn capacity_and_reserve_cover_every_storage() {
    let mut map = DenseSecondaryMap::<u32>::with_capacity(8);
    assert!(map.capacity() >= 8);
    map.reserve(20);
    assert!(map.capacity() >= 20);
    assert!(matches!(
        map.try_reserve(usize::MAX),
        Err(DenseError::Slots(_))
    ));
}

#[test]
fn values_mut_and_index_mut_change_the_values_in_place() {
    let k = keys(3);
    let mut map = DenseSecondaryMap::<u32>::default();
    assert!(map.is_empty());
    for (i, key) in (0..).zip(&k) {
        map.insert(*key, i).unwrap();
    }
    for value in map.values_mut() {
        *value *= 10;
    }
    map[k[1]] += 1;
    assert_eq!(map.values().copied().collect::<Vec<_>>(), [0, 11, 20]);
}

#[test]
#[should_panic(expected = "DenseSecondaryMap cannot make room for")]
fn reserve_panics_when_a_storage_cannot_make_room() {
    DenseSecondaryMap::<u32>::new().reserve(usize::MAX);
}

#[cfg(feature = "arrayvec")]
mod capped {
    use super::*;
    use arrayvec::ArrayVec;

    /// A dense secondary config whose slot, value and key storages hold
    /// `S`, `V` and `K` items.
    struct Caps<const S: usize, const V: usize, const K: usize>;

    impl<const S: usize, const V: usize, const K: usize> MapConfig for Caps<S, V, K> {
        type KeyConfig = Split<u8, u8>;
    }

    impl<const S: usize, const V: usize, const K: usize> DenseSecondaryMapConfig for Caps<S, V, K> {
        type ReplaceStrategy = NewerWins;
        type SlotStorage<T: GenSlotItem> = ArrayVec<T, S>;
        type ValueStorage<T> = ArrayVec<T, V>;
        type KeyStorage<T> = ArrayVec<T, K>;
    }

    /// Stores values under indices 0, 1, 2 and so on until an insert fails,
    /// and returns how many went in and which storage refused the next one.
    fn fill<const S: usize, const V: usize, const K: usize>() -> (usize, &'static str) {
        let mut map = DenseSecondaryMap::<u32, Caps<S, V, K>>::new_with_config();
        for i in 0..u8::MAX {
            let key = key_from_parts::<Split<u8, u8>>(i, 1);
            match map.insert(key, u32::from(i)) {
                Ok(_) => {}
                Err(SecondaryInsertError::StorageFull(value, error)) => {
                    assert_eq!(value, u32::from(i));
                    assert_eq!(map.len(), usize::from(i));
                    let storage = match error {
                        DenseError::Slots(_) => "slots",
                        DenseError::Values(_) => "values",
                        DenseError::Keys(_) => "keys",
                    };
                    return (map.len(), storage);
                }
                Err(_) => unreachable!(),
            }
        }
        unreachable!()
    }

    #[test]
    fn the_error_says_which_storage_is_full() {
        assert_eq!(fill::<2, 4, 4>(), (2, "slots"));
        assert_eq!(fill::<4, 2, 4>(), (2, "values"));
        assert_eq!(fill::<4, 4, 2>(), (2, "keys"));
    }

    #[test]
    fn an_insert_that_fails_adds_no_slots() {
        let mut map = DenseSecondaryMap::<u32, Caps<8, 2, 8>>::new_with_config();
        for i in 0..2 {
            let key = key_from_parts::<Split<u8, u8>>(i, 1);
            map.insert(key, u32::from(i)).unwrap();
        }
        let far = key_from_parts::<Split<u8, u8>>(6, 1);
        assert!(matches!(
            map.insert(far, 6),
            Err(SecondaryInsertError::StorageFull(6, DenseError::Values(_)))
        ));
        assert_eq!(map.slots_len(), 2);
    }
}

#[test]
fn a_panicking_drop_in_clear_leaves_an_empty_map() {
    let tracker = DropTracker::new();
    let mut keys = GenMap::new();
    let all: Vec<Key> = (0..4).map(|_| keys.insert(())).collect();
    let mut map = DenseSecondaryMap::new();
    for (i, &key) in all[..3].iter().enumerate() {
        map.insert(key, Bomb::new(&tracker, i == 1)).unwrap();
    }
    assert!(catch_unwind(AssertUnwindSafe(|| map.clear())).is_err());
    assert!(map.is_empty());
    assert!(all[..3].iter().all(|&key| map.get(key).is_none()));
    map.insert(all[3], Bomb::new(&tracker, false)).unwrap();
    assert!(map.get(all[3]).is_some());
    drop(map);
    tracker.assert_all_dropped_exactly_once(4);
}
