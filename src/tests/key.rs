use super::Cfg;
use crate::{Config, DefaultConfig, GenMap, Key};
use core::mem::size_of;
use core::num::NonZero;

#[test]
fn default_key_matches_default_config() {
    let mut map = GenMap::new();
    let key: Key = map.insert(1);
    let same: Key<DefaultConfig> = key;
    assert_eq!(map[same], 1);
}

#[test]
fn key_sizes_follow_the_config() {
    assert_eq!(size_of::<Key<Cfg<u8, u8>>>(), 2);
    assert_eq!(size_of::<Key<Cfg<u16, u16>>>(), 4);
    assert_eq!(size_of::<Key<Cfg<u32, u32>>>(), 8);
    assert_eq!(size_of::<Key<Cfg<u32, u16>>>(), 8);
    assert_eq!(size_of::<Key<Cfg<u64, u64>>>(), 16);
    assert_eq!(size_of::<Key>(), 8);
}

#[test]
fn option_of_key_costs_nothing_extra() {
    assert_eq!(size_of::<Option<Key<Cfg<u8, u8>>>>(), 2);
    assert_eq!(size_of::<Option<Key<Cfg<u16, u16>>>>(), 4);
    assert_eq!(size_of::<Option<Key<Cfg<u32, u32>>>>(), 8);
    assert_eq!(size_of::<Option<Key<Cfg<u64, u64>>>>(), 16);
}

#[test]
fn keys_are_ordered_by_index_then_generation() {
    let a = Key::<DefaultConfig> {
        idx: 1,
        generation: NonZero::new(3).unwrap(),
    };
    let b = Key::<DefaultConfig> {
        idx: 1,
        generation: NonZero::new(5).unwrap(),
    };
    let c = Key::<DefaultConfig> {
        idx: 2,
        generation: NonZero::new(1).unwrap(),
    };
    assert!(a < b);
    assert!(b < c);
    let mut sorted = [c, b, a];
    sorted.sort();
    assert_eq!(sorted, [a, b, c]);
}

#[test]
fn key_debug_prints_both_parts() {
    let k = Key::<DefaultConfig> {
        idx: 4,
        generation: NonZero::new(7).unwrap(),
    };
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
    assert_eq!(k.idx, 0u128);
    assert_eq!(k.generation.get(), 1usize);
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
    let too_wide = (1u128 << 64) | k.idx;
    let bogus = Key::<Wide> {
        idx: too_wide,
        generation: k.generation,
    };

    assert!(map.get(bogus).is_none());
    assert!(map.get_mut(bogus).is_none());
    assert!(!map.contains_key(bogus));
    assert!(map.remove(bogus).is_none());
    assert_eq!(map.len(), 1);
    assert_eq!(map[k], 1);
}
