use super::DropTracker;
use crate::GenMap;
use std::string::{String, ToString};
use std::vec::Vec;

type Map = GenMap<i32>;

#[test]
fn clone_of_empty_map_is_empty() {
    let map = Map::new();
    let clone = map.clone();
    assert_eq!(clone.len(), 0);
    assert_eq!(clone.slots_len(), 0);
}

#[test]
fn clone_copies_values_and_stays_independent() {
    let mut map = Map::new();
    let k1 = map.insert(10);
    let k2 = map.insert(20);
    let k3 = map.insert(30);

    let clone = map.clone();
    assert_eq!(clone.len(), 3);
    assert_eq!(clone.get(k1), Some(&10));
    assert_eq!(clone.get(k2), Some(&20));
    assert_eq!(clone.get(k3), Some(&30));

    assert_eq!(map.remove(k2), Some(20));
    assert_eq!(map.get(k2), None);
    assert_eq!(clone.get(k2), Some(&20));

    let k4 = map.insert(40);
    assert_eq!(map.get(k4), Some(&40));
    assert_eq!(clone.get(k4), None);
}

#[test]
fn clone_deep_copies_values() {
    let mut map = GenMap::<String>::new();
    let keys: Vec<_> = (0..100)
        .map(|i| map.insert(std::format!("val-{i}")))
        .collect();
    map.remove(keys[3]);
    map.remove(keys[10]);

    let clone = map.clone();
    assert_eq!(clone.len(), map.len());

    for (key, value) in &map {
        assert_eq!(clone.get(key), Some(value));
        assert!(!core::ptr::eq(value, clone.get(key).unwrap()));
    }
    assert!(clone.get(keys[3]).is_none());
    assert!(clone.get(keys[10]).is_none());

    *map.get_mut(keys[0]).unwrap() = "mutated".to_string();
    assert_eq!(clone[keys[0]], "val-0");
}

#[test]
fn clone_preserves_free_list_and_generations() {
    let mut map = Map::new();
    let k1 = map.insert(10);
    let k2 = map.insert(20);
    let k3 = map.insert(30);
    map.remove(k2);

    let mut clone = map.clone();
    assert_eq!(clone.len(), 2);
    assert_eq!(clone.slots_len(), 3);
    assert!(clone.get(k2).is_none());

    // Both maps hand out the same next key, since the clone copied the free
    // list and the generations.
    let from_map = map.insert(99);
    let from_clone = clone.insert(99);
    assert_eq!(from_map, from_clone);
    assert_eq!(from_map.idx(), k2.idx());
    assert_ne!(
        from_map.generation().get().get(),
        k2.generation().get().get()
    );

    assert_eq!(clone[k1], 10);
    assert_eq!(clone[k3], 30);
}

#[test]
fn clone_preserves_free_list_order() {
    let mut map = Map::new();
    let keys: Vec<_> = (0..5).map(|i| map.insert(i)).collect();
    map.remove(keys[1]);
    map.remove(keys[3]);
    map.remove(keys[0]);

    let mut clone = map.clone();
    let expected: Vec<_> = (0..3).map(|_| map.insert(0).idx()).collect();
    let got: Vec<_> = (0..3).map(|_| clone.insert(0).idx()).collect();
    assert_eq!(expected, got);
    assert_eq!(expected, [0, 3, 1]);
}

#[test]
fn clone_and_drop_are_drop_balanced() {
    let tracker = DropTracker::new();
    let mut map = GenMap::new();
    let keys: Vec<_> = (0..5).map(|_| map.insert(tracker.make_item())).collect();
    map.remove(keys[1]);
    map.remove(keys[3]);
    assert_eq!(tracker.total_dropped(), 2);

    let clone = map.clone();
    let after_clone = tracker.total_made();
    assert_eq!(after_clone, 8, "three live items were cloned");

    drop(map);
    drop(clone);
    tracker.assert_all_dropped_exactly_once(8);
}

#[test]
fn clone_from_matches_clone() {
    let mut source = Map::new();
    let keys: Vec<_> = (0..50).map(|i| source.insert(i)).collect();
    for &k in keys.iter().step_by(3) {
        source.remove(k);
    }

    let mut target = Map::new();
    for i in 0..10 {
        target.insert(i * 100);
    }
    let old_capacity = target.capacity();

    target.clone_from(&source);

    assert_eq!(target.len(), source.len());
    assert_eq!(target.slots_len(), source.slots_len());
    for (key, value) in &source {
        assert_eq!(target.get(key), Some(value));
    }
    for &k in keys.iter().step_by(3) {
        assert!(target.get(k).is_none());
    }
    assert!(target.capacity() >= old_capacity);

    let a = source.insert(-1);
    let b = target.insert(-1);
    assert_eq!(a, b);
}

#[test]
fn clone_from_overwrites_a_larger_target() {
    let mut source = Map::new();
    source.insert(1);
    let mut target = Map::new();
    for i in 0..100 {
        target.insert(i);
    }

    target.clone_from(&source);

    assert_eq!(target.len(), 1);
    assert_eq!(target.slots_len(), 1);
}

#[test]
fn clone_from_sizes_a_small_target_once() {
    let mut source = Map::new();
    let keys: Vec<_> = (0..100).map(|i| source.insert(i)).collect();
    let mut target = Map::new();
    target.insert(-1);

    target.clone_from(&source);
    assert_eq!(target.capacity(), source.slots_len());
    for (i, key) in keys.iter().enumerate() {
        assert_eq!(target[*key], i as i32);
    }
}

#[test]
fn clone_from_keeps_an_allocation_that_is_large_enough() {
    let mut source = Map::new();
    let key = source.insert(1);
    let mut target = Map::with_capacity(64);
    let capacity = target.capacity();

    target.clone_from(&source);
    assert_eq!(target.capacity(), capacity);
    assert_eq!(target[key], 1);
}

#[test]
fn clone_from_is_drop_balanced() {
    let tracker = DropTracker::new();
    let mut source = GenMap::new();
    for _ in 0..3 {
        source.insert(tracker.make_item());
    }
    let mut target = GenMap::new();
    for _ in 0..4 {
        target.insert(tracker.make_item());
    }

    target.clone_from(&source);
    assert_eq!(
        tracker.total_dropped(),
        4,
        "the old contents of the target are dropped"
    );
    assert_eq!(tracker.total_made(), 10, "three items were cloned");

    drop(source);
    drop(target);
    tracker.assert_all_dropped_exactly_once(10);
}

#[test]
fn clone_from_leaves_an_empty_map_if_a_value_panics_while_cloning() {
    struct PanicsOnClone(u32);

    impl Clone for PanicsOnClone {
        fn clone(&self) -> Self {
            if self.0 == 2 {
                panic!("no cloning the third one");
            }
            PanicsOnClone(self.0)
        }
    }

    let mut source = GenMap::new();
    for i in 0..4 {
        source.insert(PanicsOnClone(i));
    }
    let mut target = GenMap::new();
    target.insert(PanicsOnClone(100));

    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        target.clone_from(&source);
    }));
    assert!(res.is_err());

    assert_eq!(target.len(), 0);
    assert_eq!(target.slots_len(), 0);
    assert_eq!(target.iter().count(), 0);
    let k = target.insert(PanicsOnClone(7));
    assert_eq!(k.idx(), 0);
    assert_eq!(k.generation().get().get(), 1);
}
