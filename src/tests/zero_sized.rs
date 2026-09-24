//! Maps whose values are zero-sized. The slots still hold a generation, so
//! the values take no room but everything else works as usual.

use crate::GenMap;
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
    map.reattach(again, ());
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
