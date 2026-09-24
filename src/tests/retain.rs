use crate::GenMap;

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
    let tracker = super::DropTracker::new();
    let mut map = GenMap::new();
    let keys: std::vec::Vec<_> = (0..6).map(|_| map.insert(tracker.make_item())).collect();

    map.retain(|key, _| key != keys[1] && key != keys[4]);

    assert_eq!(tracker.total_dropped(), 2);
    assert_eq!(map.len(), 4);
    drop(map);
    tracker.assert_all_dropped_exactly_once(6);
}
