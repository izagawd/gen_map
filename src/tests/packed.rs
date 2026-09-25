use super::Cfg;
use crate::{
    FullError, GenMap, InsertError, Key, KeyConfig, KeyLayout, MapConfig, Odd, Packed, Split,
};
use core::mem::size_of;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::vec::Vec;

/// Four byte keys with 24 bits of index and 8 bits of generation. `Gen` is
/// exactly as wide as its field, and `Idx` is wider than its field.
struct Compact;

impl KeyConfig for Compact {
    type Idx = u32;
    type Gen = u8;
    type Layout = Packed<u32, 8>;
}

impl MapConfig for Compact {
    type KeyConfig = Self;
    type Storage<S> = Vec<S>;
}

/// Two byte keys with 12 bits of index and 4 bits of generation, so a slot
/// retires after eight uses.
struct Tiny;

impl KeyConfig for Tiny {
    type Idx = u16;
    type Gen = u8;
    type Layout = Packed<u16, 4>;
}

impl MapConfig for Tiny {
    type KeyConfig = Self;
    type Storage<S> = Vec<S>;
}

/// The same as [`Tiny`], but a slot wraps instead of retiring.
struct TinyWrap;

impl KeyConfig for TinyWrap {
    type Idx = u16;
    type Gen = u8;
    type Layout = Packed<u16, 4>;
}

impl MapConfig for TinyWrap {
    type KeyConfig = Self;
    type Storage<S> = Vec<S>;
    const WRAP_ON_OVERFLOW: bool = true;
}

/// One byte keys with 4 bits of index, so the map holds sixteen slots.
struct Byte;

impl KeyConfig for Byte {
    type Idx = u8;
    type Gen = u8;
    type Layout = Packed<u8, 4>;
}

impl MapConfig for Byte {
    type KeyConfig = Self;
    type Storage<S> = Vec<S>;
}

/// Sixteen byte keys with a `u128` field and 64 bits for each part, with
/// `Idx` and `Gen` twice as wide as their fields.
struct Huge;

impl KeyConfig for Huge {
    type Idx = u128;
    type Gen = u128;
    type Layout = Packed<u128, 64>;
}

impl MapConfig for Huge {
    type KeyConfig = Self;
    type Storage<S> = Vec<S>;
}

/// Keys the size of a pointer, with 8 bits of generation.
struct Native;

impl KeyConfig for Native {
    type Idx = usize;
    type Gen = u8;
    type Layout = Packed<usize, 8>;
}

impl MapConfig for Native {
    type KeyConfig = Self;
    type Storage<S> = Vec<S>;
}

fn key<K: KeyConfig>(idx: K::Idx, generation: K::Gen) -> Key<K> {
    // SAFETY: every test passes an odd generation and parts that fit its
    // layout.
    unsafe { Key::from_raw_parts(idx, Odd::new(generation).unwrap()) }
}

#[test]
fn packed_keys_are_the_size_of_their_field() {
    assert_eq!(size_of::<Key<Compact>>(), 4);
    assert_eq!(size_of::<Key<Tiny>>(), 2);
    assert_eq!(size_of::<Key<Byte>>(), 1);
    assert_eq!(size_of::<Key<Huge>>(), 16);
    assert_eq!(size_of::<Key<Native>>(), size_of::<usize>());
}

#[test]
fn option_of_a_packed_key_costs_nothing_extra() {
    assert_eq!(size_of::<Option<Key<Compact>>>(), 4);
    assert_eq!(size_of::<Option<Key<Tiny>>>(), 2);
    assert_eq!(size_of::<Option<Key<Byte>>>(), 1);
    assert_eq!(size_of::<Option<Key<Huge>>>(), 16);
}

#[test]
fn limits_follow_the_bit_counts() {
    assert_eq!(
        <Packed<u32, 8> as KeyLayout<u32, u8>>::max_idx(),
        (1 << 24) - 1
    );
    assert_eq!(
        <Packed<u32, 8> as KeyLayout<u32, u8>>::max_generation()
            .get()
            .get(),
        u8::MAX
    );
    assert_eq!(<Packed<u16, 4> as KeyLayout<u16, u8>>::max_idx(), 4095);
    assert_eq!(
        <Packed<u16, 4> as KeyLayout<u16, u8>>::max_generation()
            .get()
            .get(),
        15
    );
    assert_eq!(<Packed<u8, 4> as KeyLayout<u8, u8>>::max_idx(), 15);
    assert_eq!(
        <Packed<u128, 64> as KeyLayout<u128, u128>>::max_idx(),
        u64::MAX as u128
    );
    assert_eq!(
        <Packed<u128, 64> as KeyLayout<u128, u128>>::max_generation()
            .get()
            .get(),
        u64::MAX as u128
    );
    assert_eq!(
        <Packed<usize, 8> as KeyLayout<usize, u8>>::max_idx(),
        (1usize << (usize::BITS - 8)) - 1
    );
}

#[test]
fn split_limits_are_the_integer_limits() {
    assert_eq!(<Split as KeyLayout<u8, u16>>::max_idx(), u8::MAX);
    assert_eq!(
        <Split as KeyLayout<u8, u16>>::max_generation().get().get(),
        u16::MAX
    );
    assert_eq!(<Split as KeyLayout<u128, usize>>::max_idx(), u128::MAX);
    assert_eq!(
        <Split as KeyLayout<u128, usize>>::max_generation()
            .get()
            .get(),
        usize::MAX
    );
}

#[test]
fn every_layout_has_an_odd_largest_generation() {
    assert_eq!(
        <Packed<u32, 8> as KeyLayout<u32, u8>>::max_generation()
            .get()
            .get()
            % 2,
        1
    );
    assert_eq!(
        <Packed<u16, 4> as KeyLayout<u16, u8>>::max_generation()
            .get()
            .get()
            % 2,
        1
    );
    assert_eq!(
        <Packed<u16, 1> as KeyLayout<u16, u8>>::max_generation()
            .get()
            .get(),
        1
    );
    assert_eq!(
        <Split as KeyLayout<u8, u8>>::max_generation().get().get() % 2,
        1
    );
}

#[test]
fn parts_round_trip_through_a_packed_key() {
    let max_idx = <Packed<u32, 8> as KeyLayout<u32, u8>>::max_idx();
    for (idx, generation) in [
        (0, 1),
        (1, 1),
        (7, 255),
        (max_idx, 1),
        (max_idx, 255),
        (12345, 99),
    ] {
        let k = key::<Compact>(idx, generation);
        assert_eq!(k.idx(), idx);
        assert_eq!(k.generation().get().get(), generation);
    }

    let k = key::<Huge>(u64::MAX as u128, u64::MAX as u128);
    assert_eq!(k.idx(), u64::MAX as u128);
    assert_eq!(k.generation().get().get(), u64::MAX as u128);

    let k = key::<Byte>(15, 15);
    assert_eq!((k.idx(), k.generation().get().get()), (15, 15));
}

#[test]
fn the_parts_do_not_bleed_into_each_other() {
    let a = key::<Compact>(1, 1);
    let b = key::<Compact>(0, 255);
    let c = key::<Compact>(1, 255);
    assert_eq!(a.idx(), 1);
    assert_eq!(b.idx(), 0);
    assert_eq!(b.generation().get().get(), 255);
    assert_eq!(c.idx(), 1);
    assert_eq!(c.generation().get().get(), 255);
    assert_ne!(a, b);
    assert_ne!(a, c);
    assert_ne!(b, c);
}

#[test]
fn packed_keys_work_with_the_map() {
    let mut map = GenMap::<&str, Compact>::new_with_config();
    let a = map.insert("a");
    let b = map.insert("b");
    assert_eq!((a.idx(), a.generation().get().get()), (0, 1));
    assert_eq!((b.idx(), b.generation().get().get()), (1, 1));
    assert_eq!(map[a], "a");
    assert_eq!(map.get(b), Some(&"b"));

    assert_eq!(map.remove(a), Some("a"));
    assert!(map.get(a).is_none());
    assert!(!map.contains_key(a));

    let c = map.insert("c");
    assert_eq!((c.idx(), c.generation().get().get()), (0, 3));
    assert_ne!(c, a);
    assert!(map.get(a).is_none());
    assert_eq!(map[c], "c");
    assert_eq!(map.len(), 2);

    let rebuilt = key::<Compact>(c.idx(), c.generation().get().get());
    assert_eq!(rebuilt, c);
    assert_eq!(map.get(rebuilt), Some(&"c"));
}

#[test]
fn packed_keys_are_ordered_by_index_then_generation() {
    let a = key::<Compact>(1, 3);
    let b = key::<Compact>(1, 5);
    let c = key::<Compact>(2, 1);
    assert!(a < b);
    assert!(b < c);
    let mut sorted = [c, b, a];
    sorted.sort();
    assert_eq!(sorted, [a, b, c]);
}

#[test]
fn packed_keys_hash_and_compare_by_their_parts() {
    fn hash_of<K: KeyConfig>(key: Key<K>) -> u64 {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        hasher.finish()
    }

    let a = key::<Compact>(4, 7);
    let same = key::<Compact>(4, 7);
    let other = key::<Compact>(4, 9);
    assert_eq!(a, same);
    assert_eq!(hash_of(a), hash_of(same));
    assert_ne!(a, other);
}

#[test]
fn debug_prints_both_parts_of_a_packed_key() {
    let k = key::<Compact>(4, 7);
    let text = std::format!("{k:?}");
    assert!(text.contains("idx: 4"));
    assert!(text.contains("generation: 7"));
}

/// Inserts and removes on one slot until its generation is the largest one
/// the layout holds, and returns the keys handed out along the way.
fn use_up_one_slot<C>(map: &mut GenMap<u32, C>) -> Vec<Key<C::KeyConfig>>
where
    C: MapConfig,
    C::KeyConfig: KeyConfig<Idx = u16, Gen = u8>,
{
    let mut keys = Vec::new();
    for i in 0..8u32 {
        let key = map.insert(i);
        assert_eq!(key.idx(), 0);
        assert_eq!(key.generation().get().get(), (2 * i + 1) as u8);
        keys.push(key);
        if i < 7 {
            assert_eq!(map.remove(key), Some(i));
        }
    }
    keys
}

#[test]
fn a_slot_retires_when_the_generation_field_is_full() {
    let mut map = GenMap::<u32, Tiny>::new_with_config();
    let keys = use_up_one_slot(&mut map);
    let last = keys[7];
    assert_eq!(last.generation().get().get(), 15);

    assert_eq!(map.remove(last), Some(7));
    assert!(map.get(last).is_none());
    assert_eq!(map.len(), 0);
    assert_eq!(map.slots_len(), 1);

    let fresh = map.insert(100);
    assert_eq!((fresh.idx(), fresh.generation().get().get()), (1, 1));
    assert_eq!(map.slots_len(), 2);
    for key in keys {
        assert!(map.get(key).is_none());
    }
}

#[test]
fn a_slot_wraps_when_the_generation_field_is_full_and_the_config_says_so() {
    let mut map = GenMap::<u32, TinyWrap>::new_with_config();
    let keys = use_up_one_slot(&mut map);
    assert_eq!(map.remove(keys[7]), Some(7));

    let again = map.insert(100);
    assert_eq!((again.idx(), again.generation().get().get()), (0, 1));
    assert_eq!(map.slots_len(), 1);
    assert_eq!(again, keys[0]);
    assert_eq!(map.get(keys[0]), Some(&100));
    assert!(map.get(keys[7]).is_none());
}

#[test]
fn detach_refuses_a_slot_at_the_generation_limit() {
    let mut map = GenMap::<u32, Tiny>::new_with_config();
    let keys = use_up_one_slot(&mut map);
    let last = keys[7];
    assert!(map.detach(last).is_none());
    assert_eq!(map.get(last), Some(&7));
    assert_eq!(map.len(), 1);

    assert_eq!(map.remove(last), Some(7));
    let fresh = map.insert(8);
    assert_eq!(fresh.generation().get().get(), 1);
    assert_eq!(map.detach(fresh), Some(8));
    map.reattach(fresh, 9);
    assert_eq!(map[fresh], 9);
}

#[test]
fn the_index_field_limits_the_slot_count() {
    let mut map = GenMap::<u8, Byte>::new_with_config();
    let keys: Vec<_> = (0..16u8).map(|i| map.insert(i)).collect();
    for (i, key) in keys.iter().enumerate() {
        assert_eq!(key.idx() as usize, i);
        assert_eq!(map[*key], i as u8);
    }
    assert_eq!(map.slots_len(), 16);

    assert!(matches!(
        map.try_insert(16),
        Err(InsertError::IndexExhausted(16))
    ));
    assert!(matches!(map.vacant_entry(), Err(FullError::IndexExhausted)));
    assert_eq!(map.len(), 16);
    assert_eq!(map.slots_len(), 16);

    assert_eq!(map.remove(keys[5]), Some(5));
    let again = map.insert(50);
    assert_eq!((again.idx(), again.generation().get().get()), (5, 3));
    assert_eq!(map.len(), 16);
}

#[test]
#[should_panic(expected = "can not address more than 16 slots")]
fn insert_panics_when_the_index_field_is_full() {
    let mut map = GenMap::<u8, Byte>::new_with_config();
    for i in 0..=16u8 {
        map.insert(i);
    }
}

#[test]
fn parts_wider_than_their_fields_work() {
    let mut huge = GenMap::<i32, Huge>::new_with_config();
    let k = huge.insert(1);
    assert_eq!((k.idx(), k.generation().get().get()), (0, 1));
    assert_eq!(huge.remove(k), Some(1));
    let k = huge.insert(2);
    assert_eq!((k.idx(), k.generation().get().get()), (0, 3));
    assert_eq!(huge[k], 2);

    let mut native = GenMap::<i32, Native>::new_with_config();
    let k = native.insert(3);
    assert_eq!((k.idx(), k.generation().get().get()), (0, 1));
    assert_eq!(native[k], 3);
}

#[test]
fn a_packed_key_for_a_missing_slot_matches_nothing() {
    let mut map = GenMap::<i32, Compact>::new_with_config();
    let k = map.insert(1);
    let missing = key::<Compact>(1000, 1);
    let stale = key::<Compact>(k.idx(), 3);
    assert!(map.get(missing).is_none());
    assert!(map.get(stale).is_none());
    assert!(map.remove(missing).is_none());
    assert!(map.remove(stale).is_none());
    assert_eq!(map[k], 1);
}

#[test]
fn split_and_packed_hand_out_the_same_parts() {
    let mut split = GenMap::<u32, Cfg<u32, u32>>::new_with_config();
    let mut packed = GenMap::<u32, Compact>::new_with_config();
    let mut split_keys = Vec::new();
    let mut packed_keys = Vec::new();
    for i in 0..20 {
        split_keys.push(split.insert(i));
        packed_keys.push(packed.insert(i));
        if i % 3 == 0 {
            let at = (i as usize) / 2;
            assert_eq!(split.remove(split_keys[at]), packed.remove(packed_keys[at]));
        }
    }
    for (s, p) in split_keys.iter().zip(&packed_keys) {
        assert_eq!(s.idx(), p.idx());
        assert_eq!(
            s.generation().get().get(),
            u32::from(p.generation().get().get())
        );
        assert_eq!(split.get(*s), packed.get(*p));
    }
}
