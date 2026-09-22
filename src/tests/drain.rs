use super::{DropItem, DropTracker};
use crate::GenMap;
use std::vec::Vec;

type Map = GenMap<i32>;

#[test]
fn drain_empty_map_yields_nothing() {
    let mut map = Map::new();
    let items: Vec<_> = map.drain().collect();
    assert!(items.is_empty());
    assert_eq!(map.len(), 0);
}

#[test]
fn drain_yields_every_value_with_its_key() {
    let mut map = Map::new();
    let k1 = map.insert(10);
    let k2 = map.insert(20);
    let k3 = map.insert(30);

    let items: Vec<_> = map.drain().collect();
    assert_eq!(items, [(k1, 10), (k2, 20), (k3, 30)]);
    assert_eq!(map.len(), 0);
}

#[test]
fn drain_is_exact_size() {
    let mut map = Map::new();
    for i in 0..5 {
        map.insert(i);
    }
    let mut drain = map.drain();
    assert_eq!(drain.len(), 5);
    drain.next();
    assert_eq!(drain.len(), 4);
}

#[test]
fn dropping_drain_unconsumed_empties_map() {
    let mut map = Map::new();
    let k1 = map.insert(1);
    let k2 = map.insert(2);

    drop(map.drain());

    assert_eq!(map.len(), 0);
    assert!(map.get(k1).is_none());
    assert!(map.get(k2).is_none());
}

#[test]
fn drain_invalidates_old_keys() {
    let mut map = Map::new();
    let k1 = map.insert(42);
    let k2 = map.insert(99);

    let _: Vec<_> = map.drain().collect();

    assert!(map.get(k1).is_none());
    assert!(map.get(k2).is_none());
}

#[test]
fn drain_frees_slots_for_reuse() {
    let mut map = Map::new();
    let k1 = map.insert(100);

    let _: Vec<_> = map.drain().collect();

    let k2 = map.insert(200);
    assert_eq!(k2.index(), k1.index());
    assert_ne!(k2.generation(), k1.generation());
    assert_eq!(map[k2], 200);
    assert_eq!(map.len(), 1);
    assert_eq!(map.slots_len(), 1);
}

#[test]
fn drain_skips_gaps_from_prior_removes() {
    let mut map = Map::new();
    let k1 = map.insert(1);
    let k2 = map.insert(2);
    let k3 = map.insert(3);

    map.remove(k1);

    let items: Vec<_> = map.drain().collect();
    assert_eq!(items, [(k2, 2), (k3, 3)]);
    assert_eq!(map.len(), 0);
}

#[test]
fn drain_then_insert_works() {
    let mut map = Map::new();
    map.insert(1);
    map.insert(2);

    let _: Vec<_> = map.drain().collect();

    let k = map.insert(42);
    assert_eq!(map.len(), 1);
    assert_eq!(map[k], 42);
}

#[test]
fn drain_dropped_unconsumed_drops_each_value_once() {
    let tracker = DropTracker::new();
    let mut map = GenMap::<DropItem>::new();
    for _ in 0..3 {
        map.insert(tracker.make_item());
    }
    tracker.assert_none_dropped();

    drop(map.drain());

    tracker.assert_all_dropped_exactly_once(3);
    assert_eq!(map.len(), 0);
}

#[test]
fn drain_partially_consumed_drops_each_value_once() {
    let tracker = DropTracker::new();
    let mut map = GenMap::<DropItem>::new();
    for _ in 0..5 {
        map.insert(tracker.make_item());
    }

    {
        let mut drain = map.drain();
        drop(drain.next().unwrap());
        drop(drain.next().unwrap());
        assert_eq!(tracker.total_dropped(), 2);
    }

    tracker.assert_all_dropped_exactly_once(5);
    assert_eq!(map.len(), 0);
}

#[test]
fn drain_fully_consumed_hands_ownership_to_the_caller() {
    let tracker = DropTracker::new();
    let mut map = GenMap::<DropItem>::new();
    for _ in 0..4 {
        map.insert(tracker.make_item());
    }

    let collected: Vec<_> = map.drain().collect();
    tracker.assert_none_dropped();

    drop(map);
    tracker.assert_none_dropped();

    drop(collected);
    tracker.assert_all_dropped_exactly_once(4);
}

#[test]
fn drain_with_gaps_drops_only_occupied_slots() {
    let tracker = DropTracker::new();
    let mut map = GenMap::<DropItem>::new();
    let k0 = map.insert(tracker.make_item());
    map.insert(tracker.make_item());
    map.insert(tracker.make_item());

    map.remove(k0);
    assert_eq!(tracker.total_dropped(), 1);

    drop(map.drain());

    tracker.assert_all_dropped_exactly_once(3);
    assert_eq!(map.len(), 0);
}
