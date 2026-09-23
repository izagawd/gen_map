use crate::{GenMap, Key};
use core::num::NonZero;
use std::collections::HashSet;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::string::{String, ToString};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::vec::Vec;

type Map<T> = GenMap<T>;

#[test]
fn new_map_is_empty() {
    let map: Map<i32> = Map::new();
    assert_eq!(map.len(), 0);
    assert!(map.is_empty());
    assert_eq!(map.slots_len(), 0);
    assert_eq!(map.capacity(), 0);
}

#[test]
fn with_capacity_reserves_slots() {
    let map: Map<i32> = Map::with_capacity(64);
    assert!(map.capacity() >= 64);
    assert_eq!(map.slots_len(), 0);
}

#[test]
fn insert_then_get() {
    let mut map = Map::new();
    let k = map.insert(5);
    assert_eq!(map.get(k), Some(&5));
    assert!(map.contains_key(k));
    assert_eq!(map.len(), 1);
    assert!(!map.is_empty());
}

#[test]
fn first_key_is_slot_zero_generation_one() {
    let mut map = Map::new();
    let k = map.insert(());
    assert_eq!(k.idx, 0);
    assert_eq!(k.generation.get(), 1);
}

#[test]
fn get_mut_writes_through() {
    let mut map = Map::new();
    let k = map.insert(1);
    *map.get_mut(k).unwrap() += 41;
    assert_eq!(map[k], 42);
    map[k] = 7;
    assert_eq!(map.get(k), Some(&7));
}

#[test]
fn remove_returns_value_and_invalidates_key() {
    let mut map = Map::new();
    let k = map.insert(10);
    assert_eq!(map.remove(k), Some(10));
    assert_eq!(map.len(), 0);
    assert!(map.get(k).is_none());
    assert!(map.get_mut(k).is_none());
    assert!(!map.contains_key(k));
    assert!(map.remove(k).is_none());
}

#[test]
fn remove_reuses_slot_with_bumped_generation() {
    let mut map = Map::new();
    let k1 = map.insert(10);
    assert_eq!(map.remove(k1), Some(10));

    let k2 = map.insert(20);
    assert_eq!(map.get(k2), Some(&20));
    assert_eq!(map.len(), 1);
    assert_eq!(map.slots_len(), 1);

    assert_eq!(k1.idx, k2.idx);
    assert_ne!(k1.generation.get(), k2.generation.get());
    assert_eq!(k2.generation.get(), k1.generation.get() + 2);
    assert!(map.get(k1).is_none());
}

#[test]
fn free_list_is_last_in_first_out() {
    let mut map = Map::new();
    let a = map.insert("a");
    let b = map.insert("b");
    let c = map.insert("c");

    map.remove(a);
    map.remove(c);

    let d = map.insert("d");
    let e = map.insert("e");
    assert_eq!(d.idx, c.idx);
    assert_eq!(e.idx, a.idx);
    assert_eq!(map[b], "b");
    assert_eq!(map[d], "d");
    assert_eq!(map[e], "e");
    assert_eq!(map.slots_len(), 3);
}

#[test]
fn remove_then_mass_insert_keeps_old_key_invalid() {
    let mut map = Map::new();
    let k1 = map.insert(1);
    assert_eq!(map.remove(k1), Some(1));
    assert!(map.get(k1).is_none());

    for i in 0..1_000 {
        map.insert(i);
    }
    assert!(map.get(k1).is_none());
}

#[test]
fn remove_with_bogus_key_returns_none() {
    let mut map = Map::new();

    let bogus: Key = Key {
        idx: 999_999,
        generation: NonZero::new(41).unwrap(),
    };
    assert!(map.remove(bogus).is_none());
    assert!(map.get(bogus).is_none());

    let k = map.insert(1);
    let wrong_generation = Key {
        idx: k.idx,
        generation: k.generation.checked_add(2).unwrap(),
    };
    assert!(map.remove(wrong_generation).is_none());
    assert_eq!(map.len(), 1);
    assert_eq!(map[k], 1);
}

#[test]
fn len_tracks_insert_remove_and_clear() {
    let mut map = Map::new();
    assert_eq!(map.len(), 0);

    let k1 = map.insert(10);
    assert_eq!(map.len(), 1);
    let k2 = map.insert(20);
    assert_eq!(map.len(), 2);

    assert_eq!(map.remove(k1), Some(10));
    assert_eq!(map.len(), 1);

    assert!(map.remove(k1).is_none());
    assert_eq!(map.len(), 1);

    let stale = Key {
        idx: k2.idx,
        generation: k2.generation.checked_add(2).unwrap(),
    };
    assert!(map.remove(stale).is_none());
    assert_eq!(map.len(), 1);

    map.clear();
    assert_eq!(map.len(), 0);
}

#[test]
fn clear_keeps_slots_and_invalidates_every_key() {
    let mut map = Map::new();
    let keys: Vec<_> = (0..256).map(|i| map.insert(i)).collect();
    assert_eq!(map.len(), 256);

    map.clear();
    assert_eq!(map.len(), 0);
    assert_eq!(map.slots_len(), 256);

    for &k in &keys {
        assert!(map.get(k).is_none());
    }
}

#[test]
fn clear_does_not_reuse_keys() {
    let mut map = Map::new();
    let old_keys: HashSet<_> = (0..256).map(|i| map.insert(i)).collect();

    map.clear();

    let new_keys: HashSet<_> = (0..256).map(|i| map.insert(10_000 + i)).collect();
    assert!(old_keys.is_disjoint(&new_keys));
}

#[test]
fn clear_does_not_reuse_any_key_even_after_prior_removes() {
    let mut map = Map::new();
    let mut before_clear = HashSet::new();

    let keys: Vec<_> = (0..256).map(|i| map.insert(i)).collect();
    before_clear.extend(keys.iter().copied());

    for (i, &k) in keys.iter().enumerate() {
        if i % 2 == 0 {
            assert!(map.remove(k).is_some());
        }
    }

    for i in 0..256 {
        before_clear.insert(map.insert(1_000 + i));
    }

    map.clear();
    assert_eq!(map.len(), 0);

    let after_clear: HashSet<_> = (0..256).map(|i| map.insert(20_000 + i)).collect();
    assert!(before_clear.is_disjoint(&after_clear));
}

#[test]
fn insert_with_key_hands_the_value_its_own_key() {
    let mut map = Map::new();
    let k = map.insert_with_key(|key| key);
    assert_eq!(map[k], k);
}

#[test]
fn insert_and_insert_with_key_agree() {
    let mut map = Map::new();
    let k1 = map.insert("X".to_string());
    let k2 = map.insert_with_key(|_| "Y".to_string());
    assert_eq!(map[k1], "X");
    assert_eq!(map[k2], "Y");
    assert_ne!(k1.idx, k2.idx);
}

#[test]
fn try_insert_with_key_ok_stores_the_value() {
    let mut map = Map::new();
    let k = map.try_insert_with_key(|_| Ok::<_, &str>(10)).unwrap();
    assert_eq!(map[k], 10);
    assert_eq!(map.len(), 1);
}

#[test]
fn try_insert_with_key_err_leaves_map_untouched_on_fresh_slot() {
    let mut map: Map<i32> = Map::new();

    let res = map.try_insert_with_key(|_| Err::<i32, _>("nope"));
    assert_eq!(res, Err("nope"));
    assert_eq!(map.len(), 0);
    assert_eq!(map.slots_len(), 0);

    let k = map.insert(123);
    assert_eq!(k.idx, 0);
    assert_eq!(k.generation.get(), 1);
    assert_eq!(map[k], 123);
}

#[test]
fn try_insert_with_key_err_leaves_map_untouched_on_reused_slot() {
    let mut map = Map::new();
    let k1 = map.insert(1);
    assert_eq!(map.remove(k1), Some(1));

    let res = map.try_insert_with_key(|_| Err::<i32, _>("nope"));
    assert!(res.is_err());
    assert_eq!(map.len(), 0);
    assert_eq!(map.slots_len(), 1);

    let k2 = map.try_insert_with_key(|_| Ok::<_, ()>(99)).unwrap();
    assert_eq!(k2.idx, k1.idx);
    assert_ne!(k2.generation.get(), k1.generation.get());
    assert_eq!(map[k2], 99);
    assert!(map.get(k1).is_none());
}

#[test]
fn try_insert_with_key_gives_the_key_the_value_will_get() {
    let mut map = Map::new();
    let mut seen = None;
    let k = map
        .try_insert_with_key(|key| {
            seen = Some(key);
            Ok::<_, ()>(1)
        })
        .unwrap();
    assert_eq!(seen, Some(k));
}

#[test]
fn panic_inside_insert_with_key_leaves_map_untouched() {
    let mut map: Map<i32> = Map::new();

    let res = catch_unwind(AssertUnwindSafe(|| {
        map.insert_with_key(|_| -> i32 { panic!("boom") });
    }));
    assert!(res.is_err());
    assert_eq!(map.len(), 0);
    assert_eq!(map.slots_len(), 0);

    let k = map.insert(123);
    assert_eq!(k.idx, 0);
    assert_eq!(map[k], 123);
    assert_eq!(map.len(), 1);
}

#[test]
fn panic_inside_insert_with_key_keeps_freed_slot_on_free_list() {
    let mut map = Map::new();
    let k1 = map.insert(1);
    map.remove(k1);

    let res = catch_unwind(AssertUnwindSafe(|| {
        map.insert_with_key(|_| -> i32 { panic!("boom") });
    }));
    assert!(res.is_err());
    assert_eq!(map.len(), 0);

    let k2 = map.insert(2);
    assert_eq!(k2.idx, k1.idx);
    assert_eq!(map.slots_len(), 1);
}

#[test]
#[should_panic]
fn index_with_stale_key_panics() {
    let mut map = Map::new();
    let k = map.insert(1);
    map.remove(k);
    let _ = map[k];
}

#[test]
fn drop_is_called_exactly_once_per_value() {
    static DROPS: AtomicUsize = AtomicUsize::new(0);

    struct Counted;
    impl Drop for Counted {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }

    {
        let mut map = Map::new();
        let keys: Vec<_> = (0..100).map(|_| map.insert(Counted)).collect();
        for (i, &k) in keys.iter().enumerate() {
            if i % 2 == 0 {
                assert!(map.remove(k).is_some());
            }
        }
        assert_eq!(DROPS.load(Ordering::SeqCst), 50);
        for _ in 0..50 {
            map.insert(Counted);
        }
        assert_eq!(DROPS.load(Ordering::SeqCst), 50);
    }

    assert_eq!(DROPS.load(Ordering::SeqCst), 150);
}

#[test]
fn reserve_grows_capacity() {
    let mut map: Map<String> = Map::new();
    map.reserve(100);
    assert!(map.capacity() >= 100);
    assert!(map.try_reserve(10).is_ok());
}

#[test]
fn default_is_empty() {
    let map: Map<u8> = Default::default();
    assert!(map.is_empty());
}

#[test]
fn debug_output_lists_entries() {
    let mut map = Map::new();
    let k = map.insert(5);
    let text = std::format!("{map:?}");
    assert!(text.contains("5"));
    assert!(text.contains(&std::format!("{}", k.idx)));
}
