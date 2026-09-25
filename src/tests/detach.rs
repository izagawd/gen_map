use super::{Cfg, DropTracker};
use crate::GenMap;
use std::vec::Vec;

#[test]
fn detach_takes_the_value_out_and_reattach_puts_one_back() {
    let mut map = GenMap::new();
    let key = map.insert(1);
    assert_eq!(map.detach(key), Some(1));
    assert_eq!(map.get(key), None);
    assert!(!map.contains_key(key));
    assert_eq!(map.len(), 0);
    assert_eq!(map.slots_len(), 1);

    map.reattach(key, 2);
    assert_eq!(map.get(key), Some(&2));
    assert_eq!(map.len(), 1);
}

#[test]
fn detach_of_an_invalid_key_is_none() {
    let mut map = GenMap::new();
    let key = map.insert(1);
    map.remove(key);
    assert_eq!(map.detach(key), None);
    let other = map.insert(2);
    assert_eq!(map.detach(key), None);
    assert_eq!(map[other], 2);
}

#[test]
fn detaching_twice_is_none() {
    let mut map = GenMap::new();
    let key = map.insert(1);
    assert_eq!(map.detach(key), Some(1));
    assert_eq!(map.detach(key), None);
    assert_eq!(map.len(), 0);
}

#[test]
fn a_detached_slot_is_never_handed_out_by_insert() {
    let mut map = GenMap::new();
    let key = map.insert(1);
    map.detach(key).unwrap();
    let others: Vec<_> = (0..10).map(|i| map.insert(i)).collect();
    for other in others {
        assert_ne!(other.idx(), key.idx());
    }
    map.reattach(key, 1);
    assert_eq!(map[key], 1);
}

#[test]
fn a_detached_slot_is_reused_after_reattach_and_remove() {
    let mut map = GenMap::new();
    let key = map.insert(1);
    let value = map.detach(key).unwrap();
    map.reattach(key, value);
    map.remove(key);
    let reused = map.insert(2);
    assert_eq!(reused.idx(), key.idx());
    assert!(map.get(key).is_none());
}

#[test]
fn remove_and_get_mut_ignore_a_detached_key() {
    let mut map = GenMap::new();
    let key = map.insert(1);
    map.detach(key).unwrap();
    assert_eq!(map.remove(key), None);
    assert!(map.get_mut(key).is_none());
    assert_eq!(map.len(), 0);
    map.reattach(key, 5);
    assert_eq!(map.remove(key), Some(5));
}

#[test]
#[should_panic(expected = "not detached")]
fn reattach_panics_for_a_live_key() {
    let mut map = GenMap::new();
    let key = map.insert(1);
    map.reattach(key, 2);
}

#[test]
#[should_panic(expected = "not detached")]
fn reattach_panics_for_a_removed_key() {
    let mut map = GenMap::new();
    let key = map.insert(1);
    map.remove(key);
    // Like a detached slot, the removed slot now has a generation one above
    // the key's. It is the only slot on the free list, so its link to the
    // next free slot is `None`. A detached slot links to itself instead,
    // which is how `reattach` tells the two apart.
    map.reattach(key, 2);
}

#[test]
#[should_panic(expected = "not detached")]
fn reattach_panics_for_a_removed_key_deeper_in_the_free_list() {
    let mut map = GenMap::new();
    let a = map.insert(1);
    let b = map.insert(2);
    map.remove(a);
    map.remove(b);
    map.reattach(a, 3);
}

#[test]
#[should_panic(expected = "not detached")]
fn reattach_panics_for_a_reused_slot() {
    let mut map = GenMap::new();
    let key = map.insert(1);
    map.remove(key);
    map.insert(2);
    map.reattach(key, 3);
}

#[test]
#[should_panic(expected = "not detached")]
fn reattach_panics_after_reset() {
    let mut map = GenMap::new();
    let key = map.insert(1);
    map.detach(key).unwrap();
    map.reset();
    map.reattach(key, 2);
}

#[test]
#[should_panic(expected = "not detached")]
fn reattach_panics_with_the_wrong_generation_for_a_detached_slot() {
    let mut map = GenMap::new();
    let old = map.insert(1);
    map.remove(old);
    let new = map.insert(2);
    map.detach(new).unwrap();
    // `old` refers to the same slot with an older generation.
    map.reattach(old, 3);
}

#[test]
fn detach_refuses_when_the_generation_would_overflow() {
    let mut map = GenMap::<i32, Cfg<u8, u8>>::new_with_config();
    let mut key = map.insert(0);
    while key.generation().get().get() != u8::MAX {
        map.remove(key);
        key = map.insert(0);
    }
    assert_eq!(map.detach(key), None);
    assert_eq!(map.get(key), Some(&0));
    assert_eq!(map.len(), 1);
}

#[test]
fn clear_and_retain_leave_a_detached_slot_alone() {
    let mut map = GenMap::new();
    let a = map.insert(1);
    let b = map.insert(2);
    let value = map.detach(a).unwrap();

    map.retain(|_, _| true);
    map.clear();
    assert_eq!(map.len(), 0);
    assert!(map.get(b).is_none());

    map.reattach(a, value);
    assert_eq!(map[a], 1);
    assert_eq!(map.len(), 1);
}

#[test]
fn drain_and_iteration_skip_a_detached_slot() {
    let mut map = GenMap::new();
    let a = map.insert(1);
    let b = map.insert(2);
    map.detach(a).unwrap();

    assert_eq!(map.iter().map(|(k, _)| k).collect::<Vec<_>>(), [b]);
    assert_eq!(map.keys().count(), 1);
    assert_eq!(map.drain().collect::<Vec<_>>(), [(b, 2)]);
    map.reattach(a, 1);
    assert_eq!(map.into_iter().collect::<Vec<_>>(), [(a, 1)]);
}

#[test]
fn a_clone_keeps_the_slot_detached() {
    let mut map = GenMap::new();
    let a = map.insert(1);
    let value = map.detach(a).unwrap();

    let mut clone = map.clone();
    assert!(clone.get(a).is_none());
    let other = clone.insert(9);
    assert_ne!(other.idx(), a.idx());
    clone.reattach(a, value);
    assert_eq!(clone[a], 1);

    let mut copy = GenMap::new();
    copy.clone_from(&map);
    copy.reattach(a, 7);
    assert_eq!(copy[a], 7);
}

#[test]
fn a_detached_value_is_dropped_once_and_only_by_its_owner() {
    let tracker = DropTracker::new();
    let mut map = GenMap::new();
    let a = map.insert(tracker.make_item());
    let b = map.insert(tracker.make_item());

    let detached = map.detach(a).unwrap();
    drop(map);
    // Only `b` was dropped with the map.
    assert_eq!(tracker.total_dropped(), 1);
    drop(detached);
    tracker.assert_all_dropped_exactly_once(2);
    let _ = b;
}

#[test]
fn a_reattached_value_is_dropped_with_the_map() {
    let tracker = DropTracker::new();
    let mut map = GenMap::new();
    let a = map.insert(tracker.make_item());
    let value = map.detach(a).unwrap();
    map.reattach(a, value);
    tracker.assert_none_dropped();
    drop(map);
    tracker.assert_all_dropped_exactly_once(1);
}

#[test]
fn detach_lets_a_value_use_the_map() {
    struct Node {
        children: Vec<crate::Key>,
        total: i32,
    }

    let mut map = GenMap::new();
    let leaf_a = map.insert(Node {
        children: Vec::new(),
        total: 1,
    });
    let leaf_b = map.insert(Node {
        children: Vec::new(),
        total: 2,
    });
    let root = map.insert(Node {
        children: std::vec![leaf_a, leaf_b],
        total: 0,
    });

    let mut node = map.detach(root).unwrap();
    node.total = node.children.iter().map(|child| map[*child].total).sum();
    map.reattach(root, node);
    assert_eq!(map[root].total, 3);
}
