use super::{Cfg, DropTracker};
use crate::{GenMap, GenMapConfig, GenSlotItem, Key, KeyConfig, MapConfig, Packed};
use std::vec::Vec;

/// Keys whose generation is 4 bits, so the largest one is 15, well below the
/// largest `u8`, in a map that wraps generations.
struct Wrap4;

impl KeyConfig for Wrap4 {
    type Idx = u16;
    type Gen = u8;
    type Layout = Packed<u16, 4>;
}

impl<T> MapConfig<T> for Wrap4 {
    type KeyConfig = Self;
}

impl<S: GenSlotItem> GenMapConfig<S> for Wrap4 {
    const WRAP_ON_OVERFLOW: bool = true;
    type Storage = Vec<S>;
}

/// `u8` keys that use the whole generation type, in a map that wraps
/// generations.
struct WrapU8;

impl KeyConfig for WrapU8 {
    type Idx = u8;
    type Gen = u8;
    type Layout = crate::Split;
}

impl<T> MapConfig<T> for WrapU8 {
    type KeyConfig = Self;
}

impl<S: GenSlotItem> GenMapConfig<S> for WrapU8 {
    const WRAP_ON_OVERFLOW: bool = true;
    type Storage = Vec<S>;
}

/// A map whose only slot holds a value under the largest generation its key
/// can hold, and the key of that value.
fn at_the_largest_generation<K>() -> (GenMap<i32, K>, Key<K>)
where
    K: KeyConfig + MapConfig<i32, KeyConfig = K> + GenMapConfig<crate::MapSlot<i32, K>>,
{
    let mut map = GenMap::<i32, K>::new_with_config();
    let mut key = map.insert(0);
    while !key.is_max_generation() {
        map.remove(key);
        key = map.insert(0);
    }
    (map, key)
}

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
    // `old` names the same slot with an older generation.
    map.reattach(old, 3);
}

#[test]
fn detach_and_reattach_work_at_the_largest_generation() {
    let (mut map, key) = at_the_largest_generation::<Cfg<u8, u8>>();
    assert_eq!(key.generation().get().get(), u8::MAX);

    // The detached slot's generation wraps to zero, so the key finds nothing.
    assert_eq!(map.detach(key), Some(0));
    assert_eq!(map.get(key), None);
    assert_eq!(map.key_at(key.idx()), None);
    assert_eq!(map.generation_at(key.idx()), Some(0));
    assert_eq!(map.len(), 0);

    // No insert takes the detached slot.
    let other = map.insert(1);
    assert_ne!(other.idx(), key.idx());

    // Reattaching gives the slot the key's generation back.
    map.reattach(key, 5);
    assert_eq!(map[key], 5);
    assert_eq!(map.generation_at(key.idx()), Some(u8::MAX));
    assert_eq!(map.len(), 2);

    // Removing the value now retires the slot, as it would have without the
    // detach, so the next value gets a slot of its own.
    assert_eq!(map.remove(key), Some(5));
    let fresh = map.insert(2);
    assert_ne!(fresh.idx(), key.idx());
}

#[test]
fn a_packed_key_detaches_at_the_largest_generation_of_its_layout() {
    let (mut map, key) = at_the_largest_generation::<Wrap4>();
    assert_eq!(key.generation().get().get(), 15);

    // The generation goes to 16, one past the largest a key can hold. It is
    // even, so no key matches the slot.
    assert_eq!(map.detach(key), Some(0));
    assert_eq!(map.generation_at(key.idx()), Some(16));
    assert_eq!(map.get(key), None);
    map.reattach(key, 7);
    assert_eq!(map[key], 7);
    assert_eq!(map.generation_at(key.idx()), Some(15));

    // The config wraps, so removing the value frees the slot, and the next
    // value takes it under generation 1.
    assert_eq!(map.remove(key), Some(7));
    let next = map.insert(8);
    assert_eq!(next.idx(), key.idx());
    assert_eq!(next.generation().get().get(), 1);
}

#[test]
#[should_panic(expected = "not detached")]
fn reattach_panics_for_a_retired_slot_at_the_largest_generation() {
    // A retired slot has generation zero, as a slot detached at the largest
    // generation does, but it links to no slot, so it is not detached.
    let (mut map, key) = at_the_largest_generation::<Cfg<u8, u8>>();
    map.remove(key);
    assert_eq!(map.generation_at(key.idx()), Some(0));
    map.reattach(key, 1);
}

#[test]
#[should_panic(expected = "not detached")]
fn reattach_panics_for_a_wrapped_slot_at_the_largest_generation() {
    // A slot that wrapped has generation zero, the same as a slot detached at
    // the largest `u8`, but it is on the free list, which never makes it link
    // to itself.
    let (mut map, key) = at_the_largest_generation::<WrapU8>();
    map.remove(key);
    assert_eq!(map.generation_at(key.idx()), Some(0));
    map.reattach(key, 1);
}

#[test]
#[should_panic(expected = "not detached")]
fn reattach_panics_for_another_key_of_a_slot_detached_at_the_largest_generation() {
    let (mut map, key) = at_the_largest_generation::<Wrap4>();
    map.detach(key);
    // A key with the same index and an older generation does not own the
    // detached slot.
    let older = crate::tests::key_from_parts::<Wrap4>(key.idx(), 13);
    map.reattach(older, 1);
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
