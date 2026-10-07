//! Maps whose values are zero-sized. The values take no room, but each slot
//! still holds a generation, so the map works as usual.

use crate::{DenseGenMap, DenseSecondaryMap, GenMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::vec::Vec;

#[test]
fn unit_values_work_like_any_other() {
    let mut map = GenMap::<()>::new();
    let keys: Vec<_> = (0..10).map(|_| map.insert(())).collect();
    assert_eq!(map.len(), 10);
    for key in &keys {
        assert_eq!(map.get(*key), Some(&()));
    }

    assert_eq!(map.remove(keys[3]), Some(()));
    assert!(map.get(keys[3]).is_none());
    let again = map.insert(());
    assert_eq!(again.idx(), keys[3].idx());
    assert_ne!(again, keys[3]);
    assert_eq!(map.iter().count(), 10);

    assert_eq!(map.detach(again), Some(()));
    assert!(map.get(again).is_none());
    map.reattach(again, ()).unwrap();
    assert_eq!(map.get(again), Some(&()));

    assert!(map.get_disjoint_mut([keys[0], keys[1]]).is_ok());
    assert_eq!(map.get_at(keys[5].idx()), Some((keys[5], &())));

    let copy = map.clone();
    assert!(keys.iter().all(|key| copy.get(*key) == map.get(*key)));

    assert_eq!(map.drain().count(), 10);
    assert!(map.is_empty());
}

#[test]
fn zero_sized_values_are_dropped_exactly_once() {
    static DROPS: AtomicUsize = AtomicUsize::new(0);

    #[derive(Clone)]
    struct Unit;

    impl Drop for Unit {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }

    let mut map = GenMap::new();
    let keys: Vec<_> = (0..6).map(|_| map.insert(Unit)).collect();
    drop(map.remove(keys[0]));
    map.retain(|key, _| key != keys[1]);
    assert_eq!(DROPS.load(Ordering::SeqCst), 2);

    // Four clones are made here and dropped with the copy.
    drop(map.clone());
    assert_eq!(DROPS.load(Ordering::SeqCst), 6);

    let mut iter = map.into_iter();
    drop(iter.next());
    drop(iter);
    assert_eq!(DROPS.load(Ordering::SeqCst), 10);
}

#[test]
fn a_dense_map_of_unit_values_keeps_a_key_for_each_value() {
    // The values take no room, so the second slice of the map's pairs needs
    // no buffer, but the first slice still holds a key for each value.
    let mut map = DenseGenMap::<()>::new();
    let keys: Vec<_> = (0..10).map(|_| map.insert(())).collect();
    assert!(map.capacity() >= 10);
    assert_eq!(map.remove(keys[2]), Some(()));
    assert!(map.get(keys[2]).is_none());
    assert_eq!(map.detach(keys[4]), Some(()));
    map.reattach(keys[4], ()).unwrap();
    map.retain(|key, _| key != keys[7]);
    assert_eq!(map.len(), 8);

    let mut left: Vec<_> = map.keys().collect();
    left.sort();
    let mut expected: Vec<_> = keys
        .iter()
        .copied()
        .filter(|key| *key != keys[2] && *key != keys[7])
        .collect();
    expected.sort();
    assert_eq!(left, expected);

    let copy = map.clone();
    assert!(keys.iter().all(|key| copy.get(*key) == map.get(*key)));
    assert_eq!(copy.into_iter().count(), 8);
    assert_eq!(map.drain().count(), 8);
    assert!(map.is_empty());
}

#[test]
fn a_dense_secondary_map_of_unit_values_keeps_a_key_for_each_value() {
    let mut primary = GenMap::<()>::new();
    let keys: Vec<_> = (0..6).map(|_| primary.insert(())).collect();
    let mut secondary = DenseSecondaryMap::<()>::new();
    for key in &keys {
        assert!(matches!(secondary.insert(*key, ()), Ok(None)));
    }
    assert_eq!(secondary.remove(keys[1]), Some(()));
    assert_eq!(secondary.len(), 5);
    assert!(keys
        .iter()
        .all(|key| secondary.contains_key(*key) == (*key != keys[1])));
    assert_eq!(secondary.into_iter().count(), 5);
}
