use super::Cfg;
use crate::{
    FullError, GenMap, GenMapConfig, GenSlotItem, InsertError, Key, KeyConfig, KeyPiece, MapConfig,
    MapConfigFor, MapKeyConfig, Odd, Packed, Split,
};
use core::any::TypeId;
use core::mem::size_of;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::vec::Vec;

/// Four byte keys with 24 bits of index and 8 bits of generation. `Gen` is
/// exactly as wide as its field, and `Idx` is wider than its field.
struct Compact;

impl MapConfig for Compact {
    type KeyConfig = Packed<u32, 8>;
}

impl<S: GenSlotItem> GenMapConfig<S> for Compact {
    type Storage = Vec<S>;
}

/// Two byte keys with 12 bits of index and 4 bits of generation, so a slot
/// retires after eight uses.
struct Tiny;

impl MapConfig for Tiny {
    type KeyConfig = Packed<u16, 4>;
}

impl<S: GenSlotItem> GenMapConfig<S> for Tiny {
    type Storage = Vec<S>;
}

/// The same as [`Tiny`], but a slot wraps instead of retiring.
struct TinyWrap;

impl MapConfig for TinyWrap {
    type KeyConfig = Packed<u16, 4>;
}

impl<S: GenSlotItem> GenMapConfig<S> for TinyWrap {
    const WRAP_ON_OVERFLOW: bool = true;
    type Storage = Vec<S>;
}

/// One byte keys with 4 bits of index, so the map holds sixteen slots.
struct Byte;

impl MapConfig for Byte {
    type KeyConfig = Packed<u8, 4>;
}

impl<S: GenSlotItem> GenMapConfig<S> for Byte {
    type Storage = Vec<S>;
}

/// Maps with this config hand out sixteen byte keys with a `u128` field and
/// 64 bits for each part, so the index and the generation are both a `u64`.
struct Huge;

impl MapConfig for Huge {
    type KeyConfig = Packed<u128, 64>;
}

impl<S: GenSlotItem> GenMapConfig<S> for Huge {
    type Storage = Vec<S>;
}

/// Keys the size of a pointer, with 8 bits of generation.
struct Native;

impl MapConfig for Native {
    type KeyConfig = Packed<usize, 8>;
}

impl<S: GenSlotItem> GenMapConfig<S> for Native {
    type Storage = Vec<S>;
}

fn key<K: KeyConfig>(idx: K::Idx, generation: K::Gen) -> Key<K> {
    super::key_from_parts(idx, generation)
}

#[test]
fn pack_refuses_parts_that_do_not_fit() {
    let odd = |n: u8| Odd::new(n).unwrap();
    assert!(Packed::<u16, 4>::pack(4095, odd(15)).is_some());
    assert!(Packed::<u16, 4>::pack(4096, odd(15)).is_none());
    assert!(Packed::<u16, 4>::pack(0, odd(17)).is_none());
    assert!(Split::<u8, u8>::pack(u8::MAX, odd(u8::MAX)).is_some());
}

#[test]
fn packed_keys_are_the_size_of_their_field() {
    assert_eq!(size_of::<Key<Packed<u32, 8>>>(), 4);
    assert_eq!(size_of::<Key<Packed<u16, 4>>>(), 2);
    assert_eq!(size_of::<Key<Packed<u8, 4>>>(), 1);
    assert_eq!(size_of::<Key<Packed<u128, 64>>>(), 16);
    assert_eq!(size_of::<Key<Packed<usize, 8>>>(), size_of::<usize>());
}

#[test]
fn option_of_a_packed_key_costs_nothing_extra() {
    assert_eq!(size_of::<Option<Key<Packed<u32, 8>>>>(), 4);
    assert_eq!(size_of::<Option<Key<Packed<u16, 4>>>>(), 2);
    assert_eq!(size_of::<Option<Key<Packed<u8, 4>>>>(), 1);
    assert_eq!(size_of::<Option<Key<Packed<u128, 64>>>>(), 16);
}

#[test]
fn limits_follow_the_bit_counts() {
    assert_eq!(Packed::<u32, 8>::max_idx(), (1 << 24) - 1);
    assert_eq!(Packed::<u32, 8>::max_generation().get().get(), u8::MAX);
    assert_eq!(Packed::<u16, 4>::max_idx(), 4095);
    assert_eq!(Packed::<u16, 4>::max_generation().get().get(), 15);
    assert_eq!(Packed::<u8, 4>::max_idx(), 15);
    assert_eq!(Packed::<u128, 64>::max_idx(), u64::MAX);
    assert_eq!(Packed::<u128, 64>::max_generation().get().get(), u64::MAX);
    assert_eq!(
        Packed::<usize, 8>::max_idx(),
        (1usize << (usize::BITS - 8)) - 1
    );
}

#[test]
fn split_limits_are_the_integer_limits() {
    assert_eq!(Split::<u8, u16>::max_idx(), u8::MAX);
    assert_eq!(Split::<u8, u16>::max_generation().get().get(), u16::MAX);
    assert_eq!(Split::<u128, usize>::max_idx(), u128::MAX);
    assert_eq!(
        Split::<u128, usize>::max_generation().get().get(),
        usize::MAX
    );
}

#[test]
fn every_key_config_has_an_odd_largest_generation() {
    assert_eq!(Packed::<u32, 8>::max_generation().get().get() % 2, 1);
    assert_eq!(Packed::<u16, 4>::max_generation().get().get() % 2, 1);
    assert_eq!(Packed::<u16, 1>::max_generation().get().get(), 1);
    assert_eq!(Split::<u8, u8>::max_generation().get().get() % 2, 1);
}

#[test]
fn parts_round_trip_through_a_packed_key() {
    let max_idx = Packed::<u32, 8>::max_idx();
    for (idx, generation) in [
        (0, 1),
        (1, 1),
        (7, 255),
        (max_idx, 1),
        (max_idx, 255),
        (12345, 99),
    ] {
        let k = key::<Packed<u32, 8>>(idx, generation);
        assert_eq!(k.idx(), idx);
        assert_eq!(k.generation().get().get(), generation);
    }

    let k = key::<Packed<u128, 64>>(u64::MAX, u64::MAX);
    assert_eq!(k.idx(), u64::MAX);
    assert_eq!(k.generation().get().get(), u64::MAX);

    let k = key::<Packed<u8, 4>>(15, 15);
    assert_eq!((k.idx(), k.generation().get().get()), (15, 15));
}

#[test]
fn the_parts_do_not_bleed_into_each_other() {
    let a = key::<Packed<u32, 8>>(1, 1);
    let b = key::<Packed<u32, 8>>(0, 255);
    let c = key::<Packed<u32, 8>>(1, 255);
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

    let rebuilt = key::<Packed<u32, 8>>(c.idx(), c.generation().get().get());
    assert_eq!(rebuilt, c);
    assert_eq!(map.get(rebuilt), Some(&"c"));
}

#[test]
fn packed_keys_are_ordered_by_index_then_generation() {
    let a = key::<Packed<u32, 8>>(1, 3);
    let b = key::<Packed<u32, 8>>(1, 5);
    let c = key::<Packed<u32, 8>>(2, 1);
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

    let a = key::<Packed<u32, 8>>(4, 7);
    let same = key::<Packed<u32, 8>>(4, 7);
    let other = key::<Packed<u32, 8>>(4, 9);
    assert_eq!(a, same);
    assert_eq!(hash_of(a), hash_of(same));
    assert_ne!(a, other);
}

#[test]
fn debug_prints_both_parts_of_a_packed_key() {
    let k = key::<Packed<u32, 8>>(4, 7);
    let text = std::format!("{k:?}");
    assert!(text.contains("idx: 4"));
    assert!(text.contains("generation: 7"));
}

/// Inserts and removes on one slot until its generation is the largest one
/// the key config holds, and returns the keys handed out along the way.
fn use_up_one_slot<C>(map: &mut GenMap<u32, C>) -> Vec<Key<MapKeyConfig<C>>>
where
    C: MapConfigFor<u32>,
    MapKeyConfig<C>: KeyConfig<Idx = u16, Gen = u8>,
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
fn detach_works_at_the_generation_limit() {
    let mut map = GenMap::<u32, Tiny>::new_with_config();
    let keys = use_up_one_slot(&mut map);
    let last = keys[7];
    // The generation goes to 16, one more than the largest generation that
    // four bits can hold, so no key matches the slot.
    assert_eq!(map.detach(last), Some(7));
    assert_eq!(map.generation_at(0), Some(16));
    assert_eq!(map.len(), 0);
    map.reattach(last, 7).unwrap();
    assert_eq!(map.generation_at(0), Some(15));
    assert_eq!(map.get(last), Some(&7));
    assert_eq!(map.len(), 1);

    assert_eq!(map.remove(last), Some(7));
    let fresh = map.insert(8);
    assert_eq!(fresh.generation().get().get(), 1);
    assert_eq!(map.detach(fresh), Some(8));
    map.reattach(fresh, 9).unwrap();
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
#[should_panic(expected = "cannot address more than 16 slots")]
fn insert_panics_when_the_index_field_is_full() {
    let mut map = GenMap::<u8, Byte>::new_with_config();
    for i in 0..=16u8 {
        map.insert(i);
    }
}

#[test]
fn packed_keys_on_u128_and_usize_work_with_the_map() {
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
    let missing = key::<Packed<u32, 8>>(1000, 1);
    let stale = key::<Packed<u32, 8>>(k.idx(), 3);
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

/// Maps with this config hand out keys with eight bits of index and eight
/// bits of generation in a `u16`, so the index type is `u8`.
struct Wide8;

impl MapConfig for Wide8 {
    type KeyConfig = Packed<u16, 8>;
}

impl<S: GenSlotItem> GenMapConfig<S> for Wide8 {
    type Storage = Vec<S>;
}

#[test]
fn an_index_field_that_fills_its_type_leaves_out_the_largest_index() {
    // The index of `Packed<u16, 8>` has eight bits, so its type is `u8`, and
    // no slot gets `u8::MAX`.
    let mut map = GenMap::<u32, Wide8>::new_with_config();
    for i in 0..255 {
        map.insert(i);
    }
    assert!(matches!(
        map.try_insert(255),
        Err(InsertError::IndexExhausted(255))
    ));
}

#[test]
fn packed_picks_the_types_its_docs_list() {
    fn types<K: KeyConfig>() -> (TypeId, TypeId) {
        (TypeId::of::<K::Idx>(), TypeId::of::<K::Gen>())
    }

    fn pair<Idx: 'static, Gen: 'static>() -> (TypeId, TypeId) {
        (TypeId::of::<Idx>(), TypeId::of::<Gen>())
    }

    assert_eq!(types::<Packed<u16, 4>>(), pair::<u16, u8>());
    assert_eq!(types::<Packed<u32, 8>>(), pair::<u32, u8>());
    assert_eq!(types::<Packed<u32, 16>>(), pair::<u16, u16>());
    assert_eq!(types::<Packed<u64, 40>>(), pair::<u32, u64>());
    assert_eq!(types::<Packed<usize, 8>>(), pair::<usize, u8>());
}

/// Returns the number of bits in the smallest unsigned integer with at least
/// `bits` bits.
fn smallest_bits(bits: u32) -> u32 {
    [8, 16, 32, 64, 128]
        .into_iter()
        .find(|&width| width >= bits)
        .unwrap()
}

/// Checks that the limits of `Packed<R, GEN_BITS>` follow its bit counts,
/// that a key is as large as `R`, and that the largest parts go into a key
/// and come back out unchanged.
fn check_packed_limits<R: KeyPiece, const GEN_BITS: u32>()
where
    Packed<R, GEN_BITS>: KeyConfig,
{
    let idx_bits = R::BITS - GEN_BITS;
    let max_idx = Packed::<R, GEN_BITS>::max_idx();
    let max_generation = Packed::<R, GEN_BITS>::max_generation();
    let max_generation_value =
        <Packed<R, GEN_BITS> as KeyConfig>::Gen::from_non_zero(max_generation.get());
    assert_eq!(max_idx.into_u128(), (1u128 << idx_bits) - 1);
    assert_eq!(max_generation_value.into_u128(), (1u128 << GEN_BITS) - 1);
    assert_eq!(size_of::<Key<Packed<R, GEN_BITS>>>(), size_of::<R>());
    assert_eq!(
        size_of::<Option<Key<Packed<R, GEN_BITS>>>>(),
        size_of::<R>()
    );

    let largest = key::<Packed<R, GEN_BITS>>(max_idx, max_generation_value);
    assert_eq!(largest.idx(), max_idx);
    assert_eq!(largest.generation(), max_generation);
    let smallest = key::<Packed<R, GEN_BITS>>(KeyPiece::ZERO, KeyPiece::ONE);
    assert_eq!(smallest.idx().into_u128(), 0);
    let smallest_generation =
        <Packed<R, GEN_BITS> as KeyConfig>::Gen::from_non_zero(smallest.generation().get());
    assert_eq!(smallest_generation.into_u128(), 1);

    // A key has no room for an index one past the largest. The index type
    // can only hold that index when the index field is narrower than the type.
    if let Some(too_large) = max_idx.checked_add(KeyPiece::ONE) {
        assert!(Packed::<R, GEN_BITS>::pack(too_large, max_generation).is_none());
    }
}

/// Checks one `Packed` key config from the table. Its index and generation
/// types must be the smallest ones that fit, and its limits must follow its
/// bit counts.
fn check_packed<R: KeyPiece, const GEN_BITS: u32>()
where
    Packed<R, GEN_BITS>: KeyConfig,
{
    assert_eq!(
        <<Packed<R, GEN_BITS> as KeyConfig>::Idx as KeyPiece>::BITS,
        smallest_bits(R::BITS - GEN_BITS)
    );
    assert_eq!(
        <<Packed<R, GEN_BITS> as KeyConfig>::Gen as KeyPiece>::BITS,
        smallest_bits(GEN_BITS)
    );
    check_packed_limits::<R, GEN_BITS>();
}

/// Makes an array with a check for every `Packed` key config in the table
/// that `packed_table` hands it.
macro_rules! packed_checks {
    ($($r:ty { $($idx:ty, $gen:ty => $($gen_bits:literal)*;)* })*) => {
        [$($($(check_packed::<$r, $gen_bits> as fn(),)*)*)*]
    };
}

#[test]
fn every_packed_key_config_on_a_fixed_width_integer_picks_the_smallest_types() {
    let checks = crate::key_layout::packed_table!(packed_checks);
    // Each `R` has a key config for every `GEN_BITS` from 1 up to one less
    // than its bits.
    assert_eq!(checks.len(), 7 + 15 + 31 + 63 + 127);
    for check in checks {
        check();
    }
}

/// Checks one `Packed` key config on `usize`. Its index type must be `usize`,
/// its generation type must be the smallest one that fits, and its limits
/// must follow its bit counts.
fn check_packed_usize<const GEN_BITS: u32>()
where
    Packed<usize, GEN_BITS>: KeyConfig,
{
    assert_eq!(
        TypeId::of::<<Packed<usize, GEN_BITS> as KeyConfig>::Idx>(),
        TypeId::of::<usize>()
    );
    assert_eq!(
        <<Packed<usize, GEN_BITS> as KeyConfig>::Gen as KeyPiece>::BITS,
        smallest_bits(GEN_BITS)
    );
    check_packed_limits::<usize, GEN_BITS>();
}

#[cfg(target_pointer_width = "64")]
#[test]
fn every_packed_key_config_on_usize_has_a_usize_index() {
    macro_rules! check {
        ($($gen_bits:literal)*) => {
            $(check_packed_usize::<$gen_bits>();)*
        };
    }

    check!(
        1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31 32
        33 34 35 36 37 38 39 40 41 42 43 44 45 46 47 48 49 50 51 52 53 54 55 56 57 58 59 60 61
        62 63
    );
}
