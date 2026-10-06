//! These tests cover `into_raw_parts` and `from_raw_parts`. They check the
//! parts that come out of a map, parts that change between the two calls, and
//! parts that are built by hand.

use super::{key_from_parts, DropTracker};
use crate::{
    DenseGenMap, DenseGenMapConfig, DenseGenMapRawParts, DenseSecondaryMap,
    DenseSecondaryMapConfig, DenseSecondaryMapRawParts, Even, GenMap, GenMapConfig, GenMapRawParts,
    GenSlotItem, Key, MapConfig, NewerWins, Odd, Packed, Parity, SecondaryMap, SecondaryMapConfig,
    SecondaryMapRawParts, SecondarySlotItem, Slot, Split,
};
use std::vec;
use std::vec::Vec;

/// This config gives keys a `u8` index and a `u8` generation, so the parts of
/// a map are small enough to write out by hand. It keeps every storage in a
/// `Vec`.
struct Byte;

impl MapConfig for Byte {
    type KeyConfig = Split<u8, u8>;
}

impl GenMapConfig for Byte {
    type Storage<S: GenSlotItem> = Vec<S>;
}

impl DenseGenMapConfig for Byte {
    type SlotStorage<S: GenSlotItem> = Vec<S>;
    type ValueStorage<V> = Vec<V>;
    type KeyStorage<K> = Vec<K>;
}

impl SecondaryMapConfig for Byte {
    type ReplaceStrategy = NewerWins;
    type Storage<S: SecondarySlotItem> = Vec<S>;
}

impl DenseSecondaryMapConfig for Byte {
    type ReplaceStrategy = NewerWins;
    type SlotStorage<S: GenSlotItem> = Vec<S>;
    type ValueStorage<V> = Vec<V>;
    type KeyStorage<K> = Vec<K>;
}

#[cfg(feature = "std")]
impl crate::SparseSecondaryMapConfig for Byte {
    type ReplaceStrategy = NewerWins;
}

/// This config gives keys a four bit generation, so the largest generation a
/// key can hold is 15, although the generation type is a `u8`.
struct Gen4;

impl MapConfig for Gen4 {
    type KeyConfig = Packed<u16, 4>;
}

impl GenMapConfig for Gen4 {
    type Storage<S: GenSlotItem> = Vec<S>;
}

/// Returns the key of [`Byte`] with index `idx` and generation `generation`.
fn key(idx: u8, generation: u8) -> Key<Split<u8, u8>> {
    key_from_parts::<Split<u8, u8>>(idx, generation)
}

/// Returns `generation` as an `Odd`, and panics if `generation` is even.
fn odd(generation: u8) -> Odd<u8> {
    Odd::new(generation).unwrap()
}

/// Returns `generation` as an `Even`, and panics if `generation` is odd.
fn even(generation: u8) -> Even<u8> {
    Even::new(generation).unwrap()
}

/// Returns a copy of the generation and the value of `slot`.
fn parity<T: Copy, U: Copy>(slot: &Slot<u8, T, U>) -> Parity<u8, T, U> {
    match slot.as_parity() {
        Parity::Odd(generation, value) => Parity::Odd(generation, *value),
        Parity::Even(generation, value) => Parity::Even(generation, *value),
    }
}

#[test]
fn the_parts_of_a_gen_map_hold_its_slots_free_list_and_count() {
    let mut map = GenMap::<&str, Byte>::new_with_config();
    let a = map.insert("a");
    let b = map.insert("b");
    let c = map.insert("c");
    let d = map.insert("d");
    let e = map.insert("e");
    map.remove(b);
    map.remove(d);
    map.detach(a);
    map.retire(e);

    let parts = map.into_raw_parts();
    assert_eq!(parts.len, 1);
    // The slot of `d` was freed last, so the free list starts there and goes
    // on to the slot of `b`.
    assert_eq!(parts.next_free, d.idx());
    let slots: Vec<_> = parts.slots.iter().map(parity).collect();
    assert_eq!(
        slots,
        [
            Parity::Even(even(2), a.idx()),
            Parity::Even(even(2), u8::MAX),
            Parity::Odd(odd(1), "c"),
            Parity::Even(even(2), b.idx()),
            Parity::Even(even(0), u8::MAX),
        ]
    );

    let mut map = unsafe { GenMap::from_raw_parts(parts) };
    assert_eq!(map.len(), 1);
    assert_eq!(map[c], "c");
    assert!(map.get(b).is_none());
    // Inserts take the free slots in the same order as before, and the
    // retired slot stays out of use.
    assert_eq!(map.insert("f").idx(), d.idx());
    assert_eq!(map.insert("g").idx(), b.idx());
    assert_eq!(map.insert("h").idx(), 5);
    // The slot of `a` is still detached under `a`.
    assert_eq!(map.reattach(a, "a"), Ok(()));
    assert_eq!(map[a], "a");
}

#[test]
fn the_values_of_a_gen_map_can_change_between_the_two_calls() {
    let mut map = GenMap::<u32, Byte>::new_with_config();
    let keys: Vec<_> = (0..4).map(|value| map.insert(value)).collect();
    map.remove(keys[1]);

    let mut parts = map.into_raw_parts();
    for slot in &mut parts.slots {
        if let Parity::Odd(_, value) = slot.as_parity_mut() {
            *value += 10;
        }
    }
    let map = unsafe { GenMap::from_raw_parts(parts) };
    let values: Vec<_> = map.values().copied().collect();
    assert_eq!(values, [10, 12, 13]);
    assert_eq!(map[keys[3]], 13);
}

#[test]
fn the_capacity_of_a_gen_map_can_change_between_the_two_calls() {
    let mut map = GenMap::<u32, Byte>::new_with_config();
    let key = map.insert(1);

    let mut parts = map.into_raw_parts();
    parts.slots.reserve(100);
    let map = unsafe { GenMap::from_raw_parts(parts) };
    assert!(map.capacity() >= 101);

    let mut parts = map.into_raw_parts();
    parts.slots.shrink_to_fit();
    let map = unsafe { GenMap::from_raw_parts(parts) };
    assert_eq!(map.capacity(), 1);
    assert_eq!(map[key], 1);
}

#[test]
fn a_gen_map_built_by_hand_follows_its_free_list() {
    let parts = GenMapRawParts::<&str, Byte> {
        slots: vec![
            Slot::new_odd(odd(1), "a"),
            // This slot is the first one on the free list, and it links to
            // the slot at index 3.
            Slot::new_even(even(4), 3),
            // This slot is retired, so nothing ever uses it again.
            Slot::new_even(even(0), u8::MAX),
            // This slot is the last one on the free list.
            Slot::new_even(even(2), u8::MAX),
            // This slot is detached under the key with index 4 and generation
            // 5, so it holds its own index.
            Slot::new_even(even(6), 4),
        ],
        next_free: 1,
        len: 1,
    };
    let mut map = unsafe { GenMap::from_raw_parts(parts) };
    assert_eq!(map.len(), 1);
    assert_eq!(map.key_at(0), Some(key(0, 1)));
    assert_eq!(map[key(0, 1)], "a");

    assert_eq!(map.insert("b"), key(1, 5));
    assert_eq!(map.insert("c"), key(3, 3));
    assert_eq!(map.insert("d"), key(5, 1));
    assert_eq!(map.generation_at(2), Some(0));
    assert_eq!(map.reattach(key(4, 5), "e"), Ok(()));
    assert_eq!(map[key(4, 5)], "e");
    assert_eq!(map.len(), 5);
}

#[test]
fn a_free_slot_one_below_the_largest_generation_hands_out_the_largest_key() {
    let parts = GenMapRawParts::<u32, Gen4> {
        slots: vec![Slot::new_even(even(14), u16::MAX)],
        next_free: 0,
        len: 0,
    };
    let mut map = unsafe { GenMap::from_raw_parts(parts) };
    let key = map.insert(1);
    assert_eq!(key.idx(), 0);
    assert!(key.is_max_generation());
    // The slot has no generations left, so removing the value retires it.
    assert_eq!(map.remove(key), Some(1));
    assert_eq!(map.insert(2).idx(), 1);
}

#[test]
fn every_value_of_a_gen_map_is_dropped_once_with_or_without_the_round_trip() {
    let tracker = DropTracker::new();
    let mut map = GenMap::<_, Byte>::new_with_config();
    for _ in 0..4 {
        map.insert(tracker.make_item());
    }
    let parts = map.into_raw_parts();
    tracker.assert_none_dropped();
    drop(unsafe { GenMap::from_raw_parts(parts) });
    tracker.assert_all_dropped_exactly_once(4);

    let tracker = DropTracker::new();
    let mut map = GenMap::<_, Byte>::new_with_config();
    for _ in 0..4 {
        map.insert(tracker.make_item());
    }
    drop(map.into_raw_parts());
    tracker.assert_all_dropped_exactly_once(4);
}

#[test]
fn the_parts_of_a_dense_gen_map_keep_its_values_keys_and_slots_in_step() {
    let mut map = DenseGenMap::<&str, Byte>::new_with_config();
    let a = map.insert("a");
    let b = map.insert("b");
    let c = map.insert("c");
    map.remove(a);

    let parts = map.into_raw_parts();
    // Removing `a` moved the last value into its place.
    assert_eq!(parts.values, ["c", "b"]);
    assert_eq!(parts.keys, [c, b]);
    assert_eq!(parts.next_free, a.idx());
    let slots: Vec<_> = parts.slots.iter().map(parity).collect();
    assert_eq!(
        slots,
        [
            Parity::Even(even(2), u8::MAX),
            Parity::Odd(odd(1), 1),
            Parity::Odd(odd(1), 0),
        ]
    );

    let mut map = unsafe { DenseGenMap::from_raw_parts(parts) };
    assert_eq!((map[b], map[c]), ("b", "c"));
    assert_eq!(map.insert("d"), key(0, 3));
    let values: Vec<_> = map.values().copied().collect();
    assert_eq!(values, ["c", "b", "d"]);
}

#[test]
fn a_dense_gen_map_built_by_hand_moves_its_last_value_on_remove() {
    let parts = DenseGenMapRawParts::<&str, Byte> {
        slots: vec![
            Slot::new_odd(odd(1), 1),
            Slot::new_even(even(2), u8::MAX),
            Slot::new_odd(odd(3), 0),
        ],
        next_free: 1,
        values: vec!["x", "y"],
        keys: vec![key(2, 3), key(0, 1)],
    };
    let mut map = unsafe { DenseGenMap::from_raw_parts(parts) };
    assert_eq!((map[key(0, 1)], map[key(2, 3)]), ("y", "x"));

    assert_eq!(map.remove(key(2, 3)), Some("x"));
    assert_eq!(map[key(0, 1)], "y");
    // The slot freed last is the first one on the free list.
    assert_eq!(map.insert("z"), key(2, 5));
    assert_eq!(map.insert("w"), key(1, 3));
    let pairs: Vec<_> = map.iter().map(|(key, value)| (key, *value)).collect();
    assert_eq!(
        pairs,
        [(key(0, 1), "y"), (key(2, 5), "z"), (key(1, 3), "w")]
    );
}

#[test]
fn the_parts_of_a_secondary_map_hold_its_slots_and_count() {
    let mut map = SecondaryMap::<&str, Byte>::new_with_config();
    map.insert(key(1, 3), "a").unwrap();
    map.insert(key(3, 1), "b").unwrap();
    map.remove(key(3, 1));

    let parts = map.into_raw_parts();
    assert_eq!(parts.len, 1);
    let slots: Vec<_> = parts.slots.iter().map(parity).collect();
    assert_eq!(
        slots,
        [
            Parity::Even(even(0), ()),
            Parity::Odd(odd(3), "a"),
            Parity::Even(even(0), ()),
            Parity::Even(even(0), ()),
        ]
    );

    let map = unsafe { SecondaryMap::from_raw_parts(parts) };
    assert_eq!(map.len(), 1);
    assert_eq!(map[key(1, 3)], "a");
    assert!(map.get(key(3, 1)).is_none());
}

#[test]
fn a_secondary_map_built_by_hand_lets_a_newer_key_replace_its_value() {
    let parts = SecondaryMapRawParts::<&str, Byte> {
        slots: vec![Slot::new_odd(odd(5), "old"), Slot::new_even(Even::ZERO, ())],
        len: 1,
    };
    let mut map = unsafe { SecondaryMap::from_raw_parts(parts) };
    assert_eq!(map[key(0, 5)], "old");
    assert_eq!(map.insert(key(0, 7), "new").unwrap(), Some("old"));
    assert!(map.get(key(0, 5)).is_none());
    let pairs: Vec<_> = map.iter().map(|(key, value)| (key, *value)).collect();
    assert_eq!(pairs, [(key(0, 7), "new")]);
}

#[test]
fn the_parts_of_a_dense_secondary_map_keep_its_values_keys_and_slots_in_step() {
    let mut map = DenseSecondaryMap::<&str, Byte>::new_with_config();
    map.insert(key(2, 1), "a").unwrap();
    map.insert(key(0, 3), "b").unwrap();
    map.insert(key(1, 1), "c").unwrap();
    map.remove(key(2, 1));

    let parts = map.into_raw_parts();
    assert_eq!(parts.values, ["c", "b"]);
    assert_eq!(parts.keys, [key(1, 1), key(0, 3)]);
    let slots: Vec<_> = parts.slots.iter().map(parity).collect();
    assert_eq!(
        slots,
        [
            Parity::Odd(odd(3), 1),
            Parity::Odd(odd(1), 0),
            Parity::Even(even(0), ()),
        ]
    );

    let map = unsafe { DenseSecondaryMap::from_raw_parts(parts) };
    assert_eq!((map[key(0, 3)], map[key(1, 1)]), ("b", "c"));
    assert!(map.get(key(2, 1)).is_none());
}

#[test]
fn a_dense_secondary_map_built_by_hand_moves_its_last_value_on_remove() {
    let parts = DenseSecondaryMapRawParts::<&str, Byte> {
        slots: vec![
            Slot::new_odd(odd(1), 1),
            Slot::new_even(Even::ZERO, ()),
            Slot::new_odd(odd(3), 0),
        ],
        values: vec!["x", "y"],
        keys: vec![key(2, 3), key(0, 1)],
    };
    let mut map = unsafe { DenseSecondaryMap::from_raw_parts(parts) };
    assert_eq!((map[key(0, 1)], map[key(2, 3)]), ("y", "x"));

    assert_eq!(map.remove(key(2, 3)), Some("x"));
    assert_eq!(map[key(0, 1)], "y");
    map.insert(key(1, 1), "z").unwrap();
    let pairs: Vec<_> = map.iter().map(|(key, value)| (key, *value)).collect();
    assert_eq!(pairs, [(key(0, 1), "y"), (key(1, 1), "z")]);
}

#[cfg(feature = "std")]
mod sparse {
    use super::{key, odd, Byte};
    use crate::{SparseSecondaryMap, SparseSecondaryMapRawParts, SparseSlot};
    use core::hash::BuildHasher;
    use std::collections::HashMap;
    use std::format;
    use std::hash::DefaultHasher;
    use std::vec::Vec;

    /// This hasher remembers the seed it was made with, so a test can check
    /// that a map still has the same hasher.
    struct Seeded(u64);

    impl BuildHasher for Seeded {
        type Hasher = DefaultHasher;

        fn build_hasher(&self) -> DefaultHasher {
            let mut hasher = DefaultHasher::new();
            core::hash::Hasher::write_u64(&mut hasher, self.0);
            hasher
        }
    }

    #[test]
    fn the_parts_of_a_sparse_secondary_map_keep_its_values_and_hasher() {
        let mut map = SparseSecondaryMap::<&str, Byte, Seeded>::with_hasher_and_config(Seeded(7));
        map.insert(key(4, 1), "a").unwrap();
        map.insert(key(9, 3), "b").unwrap();

        let parts = map.into_raw_parts();
        assert_eq!(parts.slots.hasher().0, 7);
        let slot = &parts.slots[&9];
        assert_eq!((slot.generation(), slot.value()), (odd(3), &"b"));

        let map = unsafe { SparseSecondaryMap::from_raw_parts(parts) };
        assert_eq!(map.hasher().0, 7);
        assert_eq!((map[key(4, 1)], map[key(9, 3)]), ("a", "b"));
    }

    #[test]
    fn a_sparse_secondary_map_built_by_hand_holds_what_it_was_given() {
        let mut slots = HashMap::new();
        slots.insert(2, SparseSlot::new(odd(5), "a"));
        slots.insert(200, SparseSlot::new(odd(1), "b"));
        let mut map: SparseSecondaryMap<&str, Byte> =
            unsafe { SparseSecondaryMap::from_raw_parts(SparseSecondaryMapRawParts { slots }) };
        assert_eq!(map.len(), 2);
        assert_eq!(map.key_at(2), Some(key(2, 5)));
        assert!(map.get(key(2, 3)).is_none());

        assert_eq!(map.remove(key(200, 1)), Some("b"));
        let pairs: Vec<_> = map.iter().map(|(key, value)| (key, *value)).collect();
        assert_eq!(pairs, [(key(2, 5), "a")]);
    }

    #[test]
    fn a_sparse_slot_hands_out_its_parts() {
        let mut slot = SparseSlot::<u8, u32>::new(odd(3), 10);
        *slot.value_mut() += 1;
        assert_eq!(slot.generation(), odd(3));
        assert_eq!(slot.value(), &11);
        assert_eq!(
            format!("{slot:?}"),
            "SparseSlot { generation: 3, value: 11 }"
        );
        assert_eq!(slot.clone().into_inner(), (odd(3), 11));
    }
}
