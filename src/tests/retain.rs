use super::{Bomb, DropTracker};
use crate::GenMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::vec::Vec;

#[test]
fn retain_can_keep_everything_and_mutate() {
    let mut map = GenMap::new();
    let k1 = map.insert(10);
    let k2 = map.insert(20);

    map.retain(|_, v| {
        *v += 1;
        true
    });

    assert_eq!(map[k1], 11);
    assert_eq!(map[k2], 21);
    assert_eq!(map.len(), 2);
}

#[test]
fn retain_removes_rejected_values() {
    let mut map = GenMap::new();
    let k1 = map.insert(1);
    let k2 = map.insert(2);
    let k3 = map.insert(3);

    map.retain(|_, v| *v % 2 == 0);

    assert!(map.get(k1).is_none());
    assert!(map.get(k3).is_none());
    assert_eq!(map[k2], 2);
    assert_eq!(map.len(), 1);
}

#[test]
fn retain_passes_the_right_key() {
    let mut map = GenMap::new();
    let k1 = map.insert("a");
    let k2 = map.insert("b");

    map.retain(|key, v| {
        if key == k1 {
            assert_eq!(*v, "a");
        } else {
            assert_eq!(key, k2);
            assert_eq!(*v, "b");
        }
        key != k1
    });

    assert!(map.get(k1).is_none());
    assert_eq!(map[k2], "b");
}

#[test]
fn retain_frees_slots_for_reuse_with_bumped_generation() {
    let mut map = GenMap::new();
    let k1 = map.insert(42);

    map.retain(|key, _| key != k1);
    assert_eq!(map.len(), 0);

    let k2 = map.insert(99);
    assert_eq!(k2.idx(), k1.idx());
    assert_ne!(k2.generation(), k1.generation());
    assert_eq!(map[k2], 99);
}

#[test]
fn retain_drops_removed_values_exactly_once() {
    let tracker = DropTracker::new();
    let mut map = GenMap::new();
    let keys: Vec<_> = (0..6).map(|_| map.insert(tracker.make_item())).collect();

    map.retain(|key, _| key != keys[1] && key != keys[4]);

    assert_eq!(tracker.total_dropped(), 2);
    assert_eq!(map.len(), 4);
    drop(map);
    tracker.assert_all_dropped_exactly_once(6);
}

#[test]
fn retain_stays_consistent_when_the_closure_panics() {
    let mut map = GenMap::new();
    let keys: Vec<_> = (0..6).map(|i| map.insert(i)).collect();

    let result = catch_unwind(AssertUnwindSafe(|| {
        map.retain(|_, v| {
            if *v == 4 {
                panic!("boom");
            }
            *v % 2 == 0
        })
    }));
    assert!(result.is_err());

    // The values before the panic were filtered and the rest were never
    // looked at.
    assert_eq!(map.len(), 4);
    assert!(map.get(keys[1]).is_none());
    assert!(map.get(keys[3]).is_none());
    for i in [0, 2, 4, 5] {
        assert_eq!(map[keys[i]], i as i32);
    }

    // The two freed slots are on the free list.
    let mut reused = [map.insert(10).idx(), map.insert(11).idx()];
    reused.sort();
    assert_eq!(reused, [keys[1].idx(), keys[3].idx()]);
    assert_eq!(map.slots_len(), 6);
}

#[test]
fn retain_stays_consistent_when_a_drop_panics() {
    let tracker = DropTracker::new();
    let mut map = GenMap::new();
    let keys: Vec<_> = (0..5)
        .map(|i| map.insert(Bomb::new(&tracker, i == 2)))
        .collect();

    assert!(catch_unwind(AssertUnwindSafe(|| map.retain(|_, _| false))).is_err());

    // The panic stopped `retain` right after it took out the value at 2.
    assert_eq!(map.len(), 2);
    for key in &keys[..3] {
        assert!(map.get(*key).is_none());
    }
    for key in &keys[3..] {
        assert!(map.contains_key(*key));
    }
    assert_eq!(tracker.total_dropped(), 3);

    drop(map);
    tracker.assert_all_dropped_exactly_once(5);
}
