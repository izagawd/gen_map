use crate::tests::Cfg;
use crate::{GenMap, GenMapConfig, GenSlotItem, MapConfig, Split};
use std::vec::Vec;

/// A `u8` index and a `u8` generation, so a slot retires after holding 128
/// values.
type Retire = Cfg<u8, u8>;

/// The same keys as [`Retire`], but a slot whose generation runs out wraps
/// and is used again.
struct Wrap;

impl MapConfig for Wrap {
    type KeyConfig = Split<u8, u8>;
}

impl GenMapConfig for Wrap {
    const WRAP_ON_OVERFLOW: bool = true;
    type Storage<S: GenSlotItem> = Vec<S>;
}

#[test]
fn retiring_is_the_default_policy() {
    fn policy<C: GenMapConfig>() -> bool {
        C::WRAP_ON_OVERFLOW
    }
    assert!(!policy::<Retire>());
    assert!(!policy::<crate::DefaultMapConfig>());
    assert!(policy::<Wrap>());
}

#[test]
fn retired_slot_is_never_reused() {
    let mut map = GenMap::<u32, Retire>::new_with_config();

    let first = map.insert(1000);
    assert_eq!(first.idx(), 0);
    assert_eq!(first.generation().get().get(), 1);

    let mut last = first;
    loop {
        let generation = last.generation().get().get();
        map.remove(last);
        if generation == u8::MAX {
            break;
        }
        last = map.insert(0);
        assert_eq!(last.idx(), 0);
    }
    assert_eq!(map.slots_len(), 1);

    let after = map.insert(2222);
    assert_eq!(after.idx(), 1);
    assert_eq!(after.generation().get().get(), 1);
    assert_eq!(map.slots_len(), 2);
    assert!(map.get(first).is_none());
    assert_eq!(map.len(), 1);
    assert!(map.keys().eq([after]));
}

#[test]
fn wrap_config_reuses_slot_and_reissues_key_values() {
    let mut map = GenMap::<u32, Wrap>::new_with_config();

    let first = map.insert(1000);
    assert_eq!(first.idx(), 0);
    assert_eq!(first.generation().get().get(), 1);

    let mut last = first;
    let mut expected = 1000;
    loop {
        let generation = last.generation().get().get();
        assert_eq!(map.remove(last), Some(expected));
        assert_eq!(map.slots_len(), 1);
        if generation == u8::MAX {
            break;
        }
        last = map.insert(7);
        expected = 7;
        assert_eq!(last.idx(), 0);
    }

    let revived = map.insert(2222);
    assert_eq!(revived, first);
    assert_eq!(map.slots_len(), 1);

    // The stale key now finds the new value. That is the documented cost of
    // wrapping.
    assert_eq!(map.get(first), Some(&2222));
}

#[test]
fn retired_slots_survive_clear_and_clone() {
    let mut map = GenMap::<u32, Retire>::new_with_config();

    let mut last = map.insert(0);
    loop {
        let generation = last.generation().get().get();
        map.remove(last);
        if generation == u8::MAX {
            break;
        }
        last = map.insert(0);
    }

    let live = map.insert(5);
    assert_eq!(live.idx(), 1);

    let clone = map.clone();
    assert_eq!(clone[live], 5);
    assert_eq!(clone.len(), 1);
    assert!(clone.keys().eq([live]));

    map.clear();
    let next = map.insert(6);
    assert_eq!(next.idx(), 1, "clear must not revive a retired slot");
}


#[test]
#[should_panic(expected = "GenMap is full")]
fn inserting_into_a_full_map_panics() {
    let mut map = GenMap::<u16, Retire>::new_with_config();
    for i in 0..=255u16 {
        map.insert(i);
    }
}

#[test]
fn full_map_still_accepts_inserts_after_a_remove() {
    let mut map = GenMap::<u16, Retire>::new_with_config();
    let keys: std::vec::Vec<_> = (0..255u16).map(|i| map.insert(i)).collect();
    map.remove(keys[100]);
    let key = map.insert(1234);
    assert_eq!(key.idx(), 100);
    assert_eq!(map[key], 1234);
}
