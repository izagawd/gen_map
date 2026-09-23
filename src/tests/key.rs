use crate::{Config, DefaultConfig, GenMap, Key, KeyOf};
use core::mem::size_of;

#[test]
fn default_key_matches_default_config() {
    let mut map = GenMap::new();
    let key: Key = map.insert(1);
    let same: KeyOf<DefaultConfig> = key;
    assert_eq!(map[same], 1);
}

#[test]
fn key_sizes_follow_the_config() {
    assert_eq!(size_of::<Key<u8, u8>>(), 2);
    assert_eq!(size_of::<Key<u16, u16>>(), 4);
    assert_eq!(size_of::<Key<u32, u32>>(), 8);
    assert_eq!(size_of::<Key<u32, u16>>(), 8);
    assert_eq!(size_of::<Key<u64, u64>>(), 16);
    assert_eq!(size_of::<Key>(), 8);
}

#[test]
fn option_of_key_costs_nothing_extra() {
    assert_eq!(size_of::<Option<Key<u8, u8>>>(), 2);
    assert_eq!(size_of::<Option<Key<u16, u16>>>(), 4);
    assert_eq!(size_of::<Option<Key<u32, u32>>>(), 8);
    assert_eq!(size_of::<Option<Key<u64, u64>>>(), 16);
}

#[test]
fn from_parts_round_trips_a_real_key() {
    let mut map = GenMap::new();
    let k = map.insert("v");
    let rebuilt = Key::from_parts(k.index(), k.generation()).unwrap();
    assert_eq!(rebuilt, k);
    assert_eq!(map[rebuilt], "v");
}

#[test]
fn from_parts_rejects_even_generations() {
    assert!(Key::<u32, u32>::from_parts(0, 0).is_none());
    assert!(Key::<u32, u32>::from_parts(0, 2).is_none());
    assert!(Key::<u32, u32>::from_parts(3, 1).is_some());
    assert!(Key::<u8, u8>::from_parts(3, u8::MAX).is_some());
}

#[test]
fn key_accessors_return_the_parts() {
    let k = Key::<u16, u16>::from_parts(5, 9).unwrap();
    assert_eq!(k.index(), 5);
    assert_eq!(k.generation(), 9);
}

#[test]
fn keys_are_ordered_by_index_then_generation() {
    let a = Key::<u32, u32>::from_parts(1, 3).unwrap();
    let b = Key::<u32, u32>::from_parts(1, 5).unwrap();
    let c = Key::<u32, u32>::from_parts(2, 1).unwrap();
    assert!(a < b);
    assert!(b < c);
    let mut sorted = [c, b, a];
    sorted.sort();
    assert_eq!(sorted, [a, b, c]);
}

#[test]
fn key_debug_prints_both_parts() {
    let k = Key::<u32, u32>::from_parts(4, 7).unwrap();
    let text = std::format!("{k:?}");
    assert!(text.contains("idx: 4"));
    assert!(text.contains("generation: 7"));
}

#[test]
fn every_integer_type_works_as_a_config() {
    struct Mixed;
    impl Config for Mixed {
        type Idx = u64;
        type Gen = u8;
    }

    struct Wide;
    impl Config for Wide {
        type Idx = u128;
        type Gen = usize;
    }

    let mut mixed = GenMap::<i32, Mixed>::new_with_config();
    let k = mixed.insert(1);
    assert_eq!(mixed[k], 1);

    let mut wide = GenMap::<i32, Wide>::default();
    let k = wide.insert(2);
    assert_eq!(wide[k], 2);
    assert_eq!(k.index(), 0u128);
    assert_eq!(k.generation(), 1usize);
}

#[test]
fn map_is_send_and_sync_when_its_values_are() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<GenMap<i32>>();
    assert_send_sync::<Key>();
    assert_send_sync::<crate::Iter<'static, i32, DefaultConfig>>();
}

#[test]
fn an_index_that_does_not_fit_in_usize_matches_nothing() {
    struct Wide;
    impl Config for Wide {
        type Idx = u128;
        type Gen = u32;
    }

    let mut map = GenMap::<i32, Wide>::new_with_config();
    let k = map.insert(1);
    // Same low bits as `k`, so a truncating conversion would land on its slot.
    let too_wide = (1u128 << 64) | k.index();
    let bogus = Key::<u128, u32>::from_parts(too_wide, k.generation()).unwrap();

    assert!(map.get(bogus).is_none());
    assert!(map.get_mut(bogus).is_none());
    assert!(!map.contains_key(bogus));
    assert!(map.remove(bogus).is_none());
    assert_eq!(map.len(), 1);
    assert_eq!(map[k], 1);
}
