use crate::GenMap;
use std::collections::HashSet;
use std::vec::Vec;

type Map = GenMap<i32>;

#[test]
fn iter_on_empty_map_is_empty() {
    let map = Map::new();
    assert_eq!(map.iter().count(), 0);
    assert_eq!(map.iter().len(), 0);
}

#[test]
fn iter_covers_every_value_in_slot_order() {
    let mut map = Map::new();
    let keys: Vec<_> = (0..13).map(|i| map.insert(i)).collect();

    let seen: Vec<_> = map.iter().collect();
    assert_eq!(seen.len(), keys.len());
    for (i, ((key, value), expected)) in seen.iter().zip(&keys).enumerate() {
        assert_eq!(key, expected);
        assert_eq!(**value, i as i32);
        assert!(core::ptr::eq(*value, map.get(*key).unwrap()));
    }
}

#[test]
fn iter_skips_vacant_slots() {
    let mut map = Map::new();
    let keys: Vec<_> = (0..10).map(|i| map.insert(i)).collect();
    for &k in keys.iter().step_by(2) {
        map.remove(k);
    }

    let values: Vec<_> = map.values().copied().collect();
    assert_eq!(values, [1, 3, 5, 7, 9]);
}

#[test]
fn iter_is_exact_size_even_with_gaps() {
    let mut map = Map::new();
    let keys: Vec<_> = (0..10).map(|i| map.insert(i)).collect();
    map.remove(keys[0]);
    map.remove(keys[9]);

    let mut iter = map.iter();
    assert_eq!(iter.len(), 8);
    assert_eq!(iter.size_hint(), (8, Some(8)));
    iter.next();
    assert_eq!(iter.len(), 7);
    assert_eq!(map.keys().len(), 8);
    assert_eq!(map.values().len(), 8);
}

#[test]
fn iter_is_double_ended() {
    let mut map = Map::new();
    for i in 0..6 {
        map.insert(i);
    }
    let backwards: Vec<_> = map.values().rev().copied().collect();
    assert_eq!(backwards, [5, 4, 3, 2, 1, 0]);

    let mut iter = map.values();
    assert_eq!(iter.next(), Some(&0));
    assert_eq!(iter.next_back(), Some(&5));
    assert_eq!(iter.len(), 4);
}

#[test]
fn keys_and_values_match_iter() {
    let mut map = Map::new();
    for i in 0..20 {
        map.insert(i * 10);
    }

    let from_iter: HashSet<_> = map.iter().map(|(k, _)| k).collect();
    let from_keys: HashSet<_> = map.keys().collect();
    assert_eq!(from_iter, from_keys);

    let from_iter: Vec<_> = map.iter().map(|(_, v)| *v).collect();
    let from_values: Vec<_> = map.values().copied().collect();
    assert_eq!(from_iter, from_values);
}

#[test]
fn iter_mut_yields_valid_keys_and_can_modify() {
    let mut map = Map::new();
    let keys: Vec<_> = (0..40).map(|i| map.insert(i)).collect();

    for (key, value) in &mut map {
        assert!(map_contains(&keys, key));
        *value += 1000;
    }
    for value in map.values_mut() {
        *value += 1;
    }

    let values: Vec<_> = map.values().copied().collect();
    assert_eq!(values, (1001..1041).collect::<Vec<_>>());
    assert_eq!(map.iter_mut().len(), 40);
    assert_eq!(map.values_mut().next_back().copied(), Some(1040));
}

fn map_contains(keys: &[crate::Key], key: crate::Key) -> bool {
    keys.contains(&key)
}

#[test]
fn into_iter_yields_owned_values_with_keys() {
    let mut map = Map::new();
    let keys: Vec<_> = (0..40).map(|i| map.insert(i)).collect();
    map.remove(keys[3]);

    let iter = map.into_iter();
    assert_eq!(iter.len(), 39);
    let items: Vec<_> = iter.collect();
    assert_eq!(items.len(), 39);
    assert_eq!(items[0], (keys[0], 0));
    assert_eq!(items[3], (keys[4], 4));
    assert_eq!(items[38], (keys[39], 39));
}

#[test]
fn into_iter_is_double_ended() {
    let mut map = Map::new();
    for i in 0..5 {
        map.insert(i);
    }
    let backwards: Vec<_> = map.into_iter().rev().map(|(_, v)| v).collect();
    assert_eq!(backwards, [4, 3, 2, 1, 0]);
}

#[test]
fn into_iter_drops_each_value_exactly_once() {
    let tracker = super::DropTracker::new();
    let mut map = GenMap::new();
    let keys: Vec<_> = (0..6).map(|_| map.insert(tracker.make_item())).collect();
    map.remove(keys[2]);
    assert_eq!(tracker.total_dropped(), 1);

    let mut iter = map.into_iter();
    drop(iter.next().unwrap());
    assert_eq!(tracker.total_dropped(), 2);
    drop(iter);

    tracker.assert_all_dropped_exactly_once(6);
}

#[test]
fn iterators_are_fused() {
    let mut map = Map::new();
    map.insert(1);
    let mut iter = map.iter();
    assert!(iter.next().is_some());
    assert!(iter.next().is_none());
    assert!(iter.next().is_none());
}
