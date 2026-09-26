use super::{Cfg, DropTracker};
use crate::{GenMap, GetDisjointMutAtError, Key, KeyConfig, MapConfig, SlotItem, Split};
use std::vec::Vec;

type Tiny = Cfg<u8, u8>;

/// A map whose first slot has retired and whose value now sits in slot 1.
fn map_with_a_retired_slot() -> (GenMap<i32, Tiny>, Key<Tiny>) {
    let mut map = GenMap::<i32, Tiny>::new_with_config();
    let mut key = map.insert(0);
    while map.slots_len() == 1 {
        map.remove(key);
        key = map.insert(0);
    }
    assert_eq!(key.idx(), 1);
    (map, key)
}

#[test]
fn get_at_returns_the_key_and_the_value() {
    let mut map = GenMap::new();
    let a = map.insert("a");
    let b = map.insert("b");
    assert_eq!(map.get_at(0), Some((a, &"a")));
    assert_eq!(map.get_at(1), Some((b, &"b")));
}

#[test]
fn get_at_is_none_for_a_missing_slot() {
    let mut map = GenMap::<i32>::new();
    assert_eq!(map.get_at(0), None);
    map.insert(1);
    assert_eq!(map.get_at(1), None);
    assert_eq!(map.get_at(u32::MAX), None);
}

#[test]
fn get_at_is_none_for_a_vacant_slot() {
    let mut map = GenMap::new();
    let key = map.insert(1);
    map.remove(key);
    assert_eq!(map.get_at(key.idx()), None);
    assert_eq!(map.get_at_mut(key.idx()), None);
}

#[test]
fn get_at_is_none_for_a_detached_slot() {
    let mut map = GenMap::new();
    let key = map.insert(1);
    let value = map.detach(key).unwrap();
    assert_eq!(map.get_at(key.idx()), None);
    map.reattach(key, value);
    assert_eq!(map.get_at(key.idx()), Some((key, &1)));
}

#[test]
fn get_at_is_none_for_a_retired_slot() {
    let (map, key) = map_with_a_retired_slot();
    assert_eq!(map.get_at(0), None);
    assert_eq!(map.get_at(1), Some((key, &0)));
}

#[test]
fn get_at_reports_the_current_key_of_a_reused_slot() {
    let mut map = GenMap::new();
    let old = map.insert(1);
    map.remove(old);
    let new = map.insert(2);
    let (key, value) = map.get_at(old.idx()).unwrap();
    assert_eq!(key, new);
    assert_ne!(key, old);
    assert_eq!(*value, 2);
}

#[test]
fn get_at_with_an_index_that_does_not_fit_in_usize_is_none() {
    struct Wide;
    impl KeyConfig for Wide {
        type Idx = u128;
        type Gen = u32;
        type Layout = Split;
    }

    impl<S: SlotItem> MapConfig<S> for Wide {
        type KeyConfig = Self;
        type Storage = Vec<S>;
    }

    let mut map = GenMap::<i32, Wide>::new_with_config();
    let key = map.insert(1);
    assert_eq!(map.get_at(key.idx()), Some((key, &1)));
    assert_eq!(map.get_at(u128::MAX), None);
    assert_eq!(map.get_at_mut(u128::MAX), None);
    assert_eq!(map.generation_at(u128::MAX), None);
}

#[test]
fn get_at_mut_writes_through() {
    let mut map = GenMap::new();
    let key = map.insert(1);
    let (found, value) = map.get_at_mut(key.idx()).unwrap();
    assert_eq!(found, key);
    *value = 5;
    assert_eq!(map[key], 5);
}

#[test]
fn get_at_agrees_with_get() {
    let mut map = GenMap::new();
    let keys: Vec<_> = (0..30).map(|i| map.insert(i)).collect();
    for key in keys.iter().step_by(4) {
        map.remove(*key);
    }
    for (i, key) in keys.iter().enumerate() {
        match map.get(*key) {
            Some(value) => {
                let (found, at) = map.get_at(key.idx()).unwrap();
                assert_eq!(found, *key);
                assert!(core::ptr::eq(value, at));
                assert_eq!(*at, i as i32);
            }
            None => assert_eq!(map.get_at(key.idx()), None),
        }
    }
}

#[test]
fn get_at_unchecked_agrees_with_get_at() {
    let mut map = GenMap::new();
    let keys: Vec<_> = (0..10).map(|i| i * 10).map(|v| map.insert(v)).collect();
    map.remove(keys[3]);
    let reused = map.insert(99);

    for (key, value) in &map {
        let (found, at) = unsafe { map.get_at_unchecked(key.idx()) };
        assert_eq!(found, key);
        assert!(core::ptr::eq(value, at));
    }
    let (found, value) = unsafe { map.get_at_unchecked(3) };
    assert_eq!(found, reused);
    assert_eq!(*value, 99);
}

#[test]
fn get_at_unchecked_mut_writes_through() {
    let mut map = GenMap::new();
    let a = map.insert(1);
    let b = map.insert(2);
    {
        let (found, value) = unsafe { map.get_at_unchecked_mut(a.idx()) };
        assert_eq!(found, a);
        *value += 10;
    }
    {
        let (found, value) = unsafe { map.get_at_unchecked_mut(b.idx()) };
        assert_eq!(found, b);
        *value += 20;
    }
    assert_eq!(map[a], 11);
    assert_eq!(map[b], 22);
}

#[test]
fn generation_at_follows_the_life_of_a_slot() {
    let mut map = GenMap::new();
    assert_eq!(map.generation_at(0), None);

    let key = map.insert(1);
    assert_eq!(map.generation_at(0), Some(1));
    assert_eq!(map.generation_at(0), Some(key.generation().get().get()));

    map.remove(key);
    assert_eq!(map.generation_at(0), Some(2));

    let key = map.insert(2);
    assert_eq!(map.generation_at(0), Some(3));
    assert_eq!(key.generation().get().get(), 3);

    let value = map.detach(key).unwrap();
    assert_eq!(map.generation_at(0), Some(4));
    map.reattach(key, value);
    assert_eq!(map.generation_at(0), Some(3));

    assert_eq!(map.generation_at(1), None);
}

#[test]
fn generation_at_is_zero_for_a_retired_slot() {
    let (map, _) = map_with_a_retired_slot();
    assert_eq!(map.generation_at(0), Some(0));
    assert_eq!(map.generation_at(1), Some(1));
    assert_eq!(map.generation_at(2), None);
}

#[test]
fn generation_at_unchecked_agrees_with_generation_at() {
    let mut map = GenMap::new();
    let keys: Vec<_> = (0..8).map(|i| map.insert(i)).collect();
    map.remove(keys[2]);
    map.remove(keys[5]);
    map.insert(100);
    for idx in 0..8 {
        assert_eq!(
            unsafe { map.generation_at_unchecked(idx) },
            map.generation_at(idx).unwrap()
        );
    }
    let (map, _) = map_with_a_retired_slot();
    assert_eq!(unsafe { map.generation_at_unchecked(0) }, 0);
}

#[test]
fn get_disjoint_mut_at_hands_out_every_key_and_value() {
    let mut map = GenMap::new();
    let a = map.insert(1);
    let b = map.insert(2);
    let c = map.insert(3);

    let [(ka, x), (kb, y), (kc, z)] = map.get_disjoint_mut_at([0, 1, 2]).unwrap();
    assert_eq!((ka, kb, kc), (a, b, c));
    assert_eq!((*x, *y, *z), (1, 2, 3));
    *x += 10;
    *y += 20;
    *z += 30;
    assert_eq!(map[a], 11);
    assert_eq!(map[b], 22);
    assert_eq!(map[c], 33);
}

#[test]
fn get_disjoint_mut_at_accepts_any_order() {
    let mut map = GenMap::new();
    let keys: Vec<_> = (0..4).map(|i| map.insert(i)).collect();
    let [(kd, d), (kb, b), (ka, a), (kc, c)] = map.get_disjoint_mut_at([3, 1, 0, 2]).unwrap();
    assert_eq!((*a, *b, *c, *d), (0, 1, 2, 3));
    assert_eq!([ka, kb, kc, kd], [keys[0], keys[1], keys[2], keys[3]]);
}

#[test]
fn get_disjoint_mut_at_with_no_indices_is_fine() {
    let mut map = GenMap::<i32>::new();
    let [] = map.get_disjoint_mut_at([]).unwrap();
}

#[test]
fn get_disjoint_mut_at_rejects_a_missing_slot() {
    let mut map = GenMap::new();
    map.insert(1);
    assert_eq!(
        map.get_disjoint_mut_at([0, 1]),
        Err(GetDisjointMutAtError::NoValue)
    );
    assert_eq!(
        map.get_disjoint_mut_at([u32::MAX]),
        Err(GetDisjointMutAtError::NoValue)
    );
}

#[test]
fn get_disjoint_mut_at_rejects_a_vacant_detached_or_retired_slot() {
    let mut map = GenMap::new();
    let a = map.insert(1);
    let b = map.insert(2);
    let c = map.insert(3);
    map.remove(b);
    let value = map.detach(c).unwrap();
    assert_eq!(
        map.get_disjoint_mut_at([a.idx(), b.idx()]),
        Err(GetDisjointMutAtError::NoValue)
    );
    assert_eq!(
        map.get_disjoint_mut_at([a.idx(), c.idx()]),
        Err(GetDisjointMutAtError::NoValue)
    );
    map.reattach(c, value);
    assert!(map.get_disjoint_mut_at([a.idx(), c.idx()]).is_ok());

    let (mut map, key) = map_with_a_retired_slot();
    assert_eq!(
        map.get_disjoint_mut_at([key.idx(), 0]),
        Err(GetDisjointMutAtError::NoValue)
    );
}

#[test]
fn get_disjoint_mut_at_rejects_the_same_index_twice() {
    let mut map = GenMap::new();
    map.insert(1);
    map.insert(2);
    assert_eq!(
        map.get_disjoint_mut_at([0, 0]),
        Err(GetDisjointMutAtError::OverlappingIndices)
    );
    assert_eq!(
        map.get_disjoint_mut_at([0, 1, 0]),
        Err(GetDisjointMutAtError::OverlappingIndices)
    );
}

#[test]
fn get_disjoint_mut_at_borrows_nothing_on_error() {
    let mut map = GenMap::new();
    let a = map.insert(1);
    let b = map.insert(2);
    let error = map.get_disjoint_mut_at([0, 1, 0]).unwrap_err();
    assert_eq!(error, GetDisjointMutAtError::OverlappingIndices);
    assert_eq!(map.get(a), Some(&1));
    assert_eq!(map.get(b), Some(&2));
}

#[test]
fn get_disjoint_mut_at_reports_the_current_keys_of_reused_slots() {
    let mut map = GenMap::new();
    let old_a = map.insert(1);
    let b = map.insert(2);
    map.remove(old_a);
    let new_a = map.insert(3);
    let [(ka, x), (kb, _)] = map.get_disjoint_mut_at([0, 1]).unwrap();
    assert_eq!(ka, new_a);
    assert_ne!(ka, old_a);
    assert_eq!(kb, b);
    assert_eq!(*x, 3);
}

#[test]
fn get_disjoint_mut_at_unchecked_hands_out_every_key_and_value() {
    let mut map = GenMap::new();
    let a = map.insert(1);
    let b = map.insert(2);
    let [(ka, x), (kb, y)] = unsafe { map.get_disjoint_mut_at_unchecked([0, 1]) };
    assert_eq!((ka, kb), (a, b));
    *x = 10;
    *y = 20;
    assert_eq!(map[a], 10);
    assert_eq!(map[b], 20);
}

#[test]
fn get_disjoint_mut_at_references_point_at_the_slots() {
    let mut map = GenMap::new();
    let a = map.insert(1);
    let b = map.insert(2);
    let expected_a = map.get(a).unwrap() as *const i32;
    let expected_b = map.get(b).unwrap() as *const i32;
    let [(_, x), (_, y)] = map.get_disjoint_mut_at([0, 1]).unwrap();
    assert!(core::ptr::eq(x, expected_a));
    assert!(core::ptr::eq(y, expected_b));
}

#[test]
fn values_touched_through_get_disjoint_mut_at_drop_exactly_once() {
    let tracker = DropTracker::new();
    let mut map = GenMap::new();
    map.insert(tracker.make_item());
    map.insert(tracker.make_item());
    {
        let [(_, x), (_, y)] = map.get_disjoint_mut_at([0, 1]).unwrap();
        core::mem::swap(x, y);
    }
    tracker.assert_none_dropped();
    drop(map);
    tracker.assert_all_dropped_exactly_once(2);
}

#[test]
fn the_error_prints_something_readable() {
    let text = std::format!("{}", GetDisjointMutAtError::NoValue);
    assert!(text.contains("no value"));
    let text = std::format!("{}", GetDisjointMutAtError::OverlappingIndices);
    assert!(text.contains("same"));
}
