use super::DropTracker;
use crate::{GenMap, GetDisjointMutError};
use std::vec::Vec;

#[test]
fn get_disjoint_mut_hands_out_every_value() {
    let mut map = GenMap::new();
    let a = map.insert(1);
    let b = map.insert(2);
    let c = map.insert(3);

    let [x, y, z] = map.get_disjoint_mut([a, b, c]).unwrap();
    assert_eq!((*x, *y, *z), (1, 2, 3));
    *x += 10;
    *y += 20;
    *z += 30;
    assert_eq!(map[a], 11);
    assert_eq!(map[b], 22);
    assert_eq!(map[c], 33);
}

#[test]
fn get_disjoint_mut_can_swap_two_values() {
    let mut map = GenMap::new();
    let a = map.insert("a");
    let b = map.insert("b");
    let [x, y] = map.get_disjoint_mut([a, b]).unwrap();
    core::mem::swap(x, y);
    assert_eq!(map[a], "b");
    assert_eq!(map[b], "a");
}

#[test]
fn get_disjoint_mut_accepts_any_order() {
    let mut map = GenMap::new();
    let keys: Vec<_> = (0..4).map(|i| map.insert(i)).collect();
    let [d, b, a, c] = map
        .get_disjoint_mut([keys[3], keys[1], keys[0], keys[2]])
        .unwrap();
    assert_eq!((*a, *b, *c, *d), (0, 1, 2, 3));
}

#[test]
fn get_disjoint_mut_with_one_key_is_get_mut() {
    let mut map = GenMap::new();
    let a = map.insert(1);
    let [x] = map.get_disjoint_mut([a]).unwrap();
    *x = 5;
    assert_eq!(map[a], 5);
}

#[test]
fn get_disjoint_mut_with_no_keys_is_fine() {
    let mut map = GenMap::<i32>::new();
    let [] = map.get_disjoint_mut([]).unwrap();
}

#[test]
fn get_disjoint_mut_rejects_a_stale_key() {
    let mut map = GenMap::new();
    let a = map.insert(1);
    let b = map.insert(2);
    map.remove(b);
    assert_eq!(
        map.get_disjoint_mut([a, b]),
        Err(GetDisjointMutError::InvalidKey)
    );
    assert_eq!(map[a], 1);
}

#[test]
fn get_disjoint_mut_rejects_a_stale_key_for_a_reused_slot() {
    let mut map = GenMap::new();
    let old = map.insert(1);
    map.remove(old);
    let new = map.insert(2);
    assert_eq!(
        map.get_disjoint_mut([old, new]),
        Err(GetDisjointMutError::InvalidKey)
    );
    assert_eq!(
        map.get_disjoint_mut([new, old]),
        Err(GetDisjointMutError::InvalidKey)
    );
}

#[test]
fn get_disjoint_mut_rejects_a_detached_key() {
    let mut map = GenMap::new();
    let a = map.insert(1);
    let b = map.insert(2);
    let value = map.detach(b).unwrap();
    assert_eq!(
        map.get_disjoint_mut([a, b]),
        Err(GetDisjointMutError::InvalidKey)
    );
    map.reattach(b, value);
    assert!(map.get_disjoint_mut([a, b]).is_ok());
}

#[test]
fn get_disjoint_mut_rejects_the_same_key_twice() {
    let mut map = GenMap::new();
    let a = map.insert(1);
    let b = map.insert(2);
    assert_eq!(
        map.get_disjoint_mut([a, a]),
        Err(GetDisjointMutError::OverlappingKeys)
    );
    assert_eq!(
        map.get_disjoint_mut([a, b, a]),
        Err(GetDisjointMutError::OverlappingKeys)
    );
}

#[test]
fn get_disjoint_mut_borrows_nothing_on_error() {
    let mut map = GenMap::new();
    let a = map.insert(1);
    let b = map.insert(2);
    let error = map.get_disjoint_mut([a, b, a]).unwrap_err();
    assert_eq!(error, GetDisjointMutError::OverlappingKeys);
    // Nothing stays borrowed after the error, so the map can be used right
    // away.
    assert_eq!(map.get(a), Some(&1));
    assert_eq!(map.get(b), Some(&2));
}

#[test]
fn get_disjoint_mut_unchecked_hands_out_every_value() {
    let mut map = GenMap::new();
    let a = map.insert(1);
    let b = map.insert(2);
    let [x, y] = unsafe { map.get_disjoint_mut_unchecked([a, b]) };
    *x = 10;
    *y = 20;
    assert_eq!(map[a], 10);
    assert_eq!(map[b], 20);
}

#[test]
fn get_disjoint_mut_references_point_at_the_slots() {
    let mut map = GenMap::new();
    let a = map.insert(1);
    let b = map.insert(2);
    let expected_a = map.get(a).unwrap() as *const i32;
    let expected_b = map.get(b).unwrap() as *const i32;
    let [x, y] = map.get_disjoint_mut([a, b]).unwrap();
    assert!(core::ptr::eq(x, expected_a));
    assert!(core::ptr::eq(y, expected_b));
}

#[test]
fn values_touched_through_get_disjoint_mut_drop_exactly_once() {
    let tracker = DropTracker::new();
    let mut map = GenMap::new();
    let a = map.insert(tracker.make_item());
    let b = map.insert(tracker.make_item());
    {
        let [x, y] = map.get_disjoint_mut([a, b]).unwrap();
        core::mem::swap(x, y);
    }
    tracker.assert_none_dropped();
    drop(map);
    tracker.assert_all_dropped_exactly_once(2);
}

#[test]
fn the_error_prints_something_readable() {
    let text = std::format!("{}", GetDisjointMutError::InvalidKey);
    assert!(text.contains("invalid"));
    let text = std::format!("{}", GetDisjointMutError::OverlappingKeys);
    assert!(text.contains("same slot"));
}
