//! Tests for `SecondaryMap`.

use super::{key_from_parts, Bomb, Cfg, DropItem, DropTracker};
use crate::{
    DefaultKeyConfig, ExistingWins, GenMap, GenMapConfig, GenSlotItem, GetDisjointMutAtError,
    GetDisjointMutError, Key, KeyConfig, MapConfig, NewerWins, NewerWinsWrapping, Odd, Packed,
    ReplaceStrategy, SecondaryInsertError, SecondaryMap, SecondaryMapConfig, SecondaryMapConfigFor,
    SecondarySlot, SecondarySlotItem,
};
use core::marker::PhantomData;
use std::collections::HashMap;
use std::format;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::vec::Vec;

/// Default keys and slots in a `Vec`, but a value is never replaced by an
/// insert under a different generation.
struct Keep;

impl<T> MapConfig<T> for Keep {
    type KeyConfig = DefaultKeyConfig;
}

impl<S: SecondarySlotItem> SecondaryMapConfig<S> for Keep {
    type ReplaceStrategy = ExistingWins;
    type Storage = Vec<S>;
}

/// A `SecondaryMap` with a `Cfg` config uses `NewerWins` and keeps its
/// slots in a `Vec`.
impl<Idx: crate::KeyPiece, Gen: crate::KeyPiece, S: SecondarySlotItem> SecondaryMapConfig<S>
    for Cfg<Idx, Gen>
{
    type ReplaceStrategy = NewerWins;
    type Storage = Vec<S>;
}

/// Keys whose generation is 4 bits, so it goes from 0 to 15, from a
/// `GenMap` that wraps it.
struct Wrap4;

impl KeyConfig for Wrap4 {
    type Idx = u16;
    type Gen = u8;
    type Layout = Packed<u16, 4>;
}

impl<T> MapConfig<T> for Wrap4 {
    type KeyConfig = Self;
}

impl<S: GenSlotItem> GenMapConfig<S> for Wrap4 {
    const WRAP_ON_OVERFLOW: bool = true;
    type Storage = Vec<S>;
}

/// [`Wrap4`] keys with the strategy `R`.
struct With<R>(PhantomData<R>);

impl<R, T> MapConfig<T> for With<R> {
    type KeyConfig = Wrap4;
}

impl<R: ReplaceStrategy<Wrap4>, S: SecondarySlotItem> SecondaryMapConfig<S> for With<R> {
    type ReplaceStrategy = R;
    type Storage = Vec<S>;
}

/// Two keys for the same index. The first is from before the `GenMap`
/// reused the slot, and the second is the newer key it handed out after.
fn reused_keys() -> (Key, Key) {
    let mut keys = GenMap::new();
    let old = keys.insert(());
    keys.remove(old);
    let new = keys.insert(());
    assert_eq!(new.idx(), old.idx());
    (old, new)
}

/// The key a slot had just before its generation wrapped, which is 15, and
/// the key it had just after, which is 1.
fn keys_around_a_wrap() -> (Key<Wrap4>, Key<Wrap4>) {
    let mut keys = GenMap::<(), Wrap4>::new_with_config();
    let mut before = keys.insert(());
    while before.generation().get().get() != 15 {
        keys.remove(before);
        before = keys.insert(());
    }
    keys.remove(before);
    let after = keys.insert(());
    assert_eq!(after.idx(), before.idx());
    assert_eq!(after.generation().get().get(), 1);
    (before, after)
}

/// The value a refused insert handed back. Panics if the insert was not
/// refused.
fn refused<T, E>(result: Result<Option<T>, SecondaryInsertError<T, E>>) -> T {
    match result {
        Err(SecondaryInsertError::Refused(value)) => value,
        _ => panic!("the insert was not refused"),
    }
}

fn odd<G: crate::KeyPiece>(generation: G) -> Odd<G> {
    Odd::new(generation).unwrap()
}

#[test]
fn insert_get_remove() {
    let mut keys = GenMap::new();
    let a = keys.insert("a");
    let b = keys.insert("b");
    let mut map = SecondaryMap::<u32>::new();
    assert!(map.is_empty());

    assert_eq!(map.insert(a, 1).unwrap(), None);
    assert_eq!(map.insert(b, 2).unwrap(), None);
    assert_eq!(map.len(), 2);
    assert_eq!(map.get(a), Some(&1));
    assert!(map.contains_key(b));

    *map.get_mut(b).unwrap() += 10;
    assert_eq!(map[b], 12);
    map[a] = 5;
    assert_eq!(map.insert(a, 6).unwrap(), Some(5));
    assert_eq!(map.len(), 2);

    assert_eq!(map.remove(a), Some(6));
    assert_eq!(map.remove(a), None);
    assert_eq!(map.get(a), None);
    assert!(!map.contains_key(a));
    assert_eq!(map.len(), 1);

    map.clear();
    assert!(map.is_empty());
    assert_eq!(map.get(b), None);
}

#[test]
fn a_newer_key_replaces_an_older_value() {
    let (old, new) = reused_keys();
    let mut map = SecondaryMap::<u32>::new();
    map.insert(old, 1).unwrap();

    // The newer key does not match the older value, but its insert
    // replaces it and hands it back.
    assert_eq!(map.get(new), None);
    assert_eq!(map.insert(new, 2).unwrap(), Some(1));
    assert_eq!(map.get(old), None);
    assert_eq!(map[new], 2);
    assert_eq!(map.len(), 1);

    // The older key cannot take the slot back.
    assert_eq!(refused(map.insert(old, 3)), 3);
    assert_eq!(map.remove(old), None);
    assert_eq!(map[new], 2);
    assert_eq!(map.len(), 1);
}

#[test]
fn an_empty_slot_takes_any_key() {
    // An empty slot has no generation to compare with, so even an older
    // key's insert goes in.
    let (old, new) = reused_keys();
    let mut map = SecondaryMap::<u32>::new();
    map.insert(new, 1).unwrap();
    map.remove(new);
    assert_eq!(map.insert(old, 2).unwrap(), None);
    assert_eq!(map[old], 2);
    assert_eq!(map.len(), 1);
}

#[test]
fn existing_wins_keeps_the_older_value_until_it_is_removed() {
    let (old, new) = reused_keys();
    let mut map = SecondaryMap::<u32, Keep>::new_with_config();
    map.insert(old, 1).unwrap();
    assert_eq!(refused(map.insert(new, 2)), 2);
    assert_eq!(map[old], 1);

    assert_eq!(map.remove(old), Some(1));
    assert_eq!(map.insert(new, 2).unwrap(), None);
    assert_eq!(map[new], 2);
}

#[test]
fn only_the_wrapping_strategy_takes_a_key_from_after_a_wrap() {
    let (before, after) = keys_around_a_wrap();

    let mut plain = SecondaryMap::<u32, With<NewerWins>>::new_with_config();
    plain.insert(before, 1).unwrap();
    assert_eq!(refused(plain.insert(after, 2)), 2);

    let mut wrapping = SecondaryMap::<u32, With<NewerWinsWrapping>>::new_with_config();
    wrapping.insert(before, 1).unwrap();
    assert_eq!(wrapping.insert(after, 2).unwrap(), Some(1));
}

#[test]
fn iterators() {
    let mut keys = GenMap::new();
    let all: Vec<Key> = (0..5).map(|_| keys.insert(())).collect();
    let mut map = SecondaryMap::<u32>::new();
    // Skipping the second key leaves an empty slot in a `Vec`.
    for (i, &key) in all.iter().enumerate() {
        if i != 1 {
            map.insert(key, i as u32).unwrap();
        }
    }

    let mut pairs: Vec<(Key, u32)> = map.iter().map(|(key, &value)| (key, value)).collect();
    pairs.sort();
    assert_eq!(pairs, [(all[0], 0), (all[2], 2), (all[3], 3), (all[4], 4)]);

    {
        let mut iter = map.iter();
        assert_eq!(iter.size_hint(), (4, Some(4)));
        iter.next();
        assert_eq!(iter.len(), 3);
        assert_eq!(iter.clone().count(), 3);
        assert_eq!(iter.by_ref().count(), 3);
        assert_eq!(iter.next(), None);
    }

    let mut found: Vec<Key> = map.keys().collect();
    found.sort();
    assert_eq!(found, [all[0], all[2], all[3], all[4]]);
    assert_eq!(map.keys().len(), 4);
    assert_eq!(map.values().sum::<u32>(), 9);

    for value in map.values_mut() {
        *value *= 10;
    }
    for (_, value) in map.iter_mut() {
        *value += 1;
    }
    for (_, value) in &mut map {
        *value += 1;
    }
    for (key, value) in &map {
        assert_eq!(map[key], *value);
    }

    let mut owned: Vec<(Key, u32)> = map.into_iter().collect();
    owned.sort();
    assert_eq!(
        owned,
        [(all[0], 2), (all[2], 22), (all[3], 32), (all[4], 42)]
    );
}

#[test]
fn retain() {
    let mut keys = GenMap::new();
    let mut map = SecondaryMap::<u32>::new();
    let all: Vec<Key> = (0..6)
        .map(|i| {
            let key = keys.insert(());
            map.insert(key, i).unwrap();
            key
        })
        .collect();

    map.retain(|key, value| {
        assert_eq!(key, all[*value as usize]);
        *value % 2 == 0
    });
    assert_eq!(map.len(), 3);
    for (i, &key) in all.iter().enumerate() {
        assert_eq!(map.contains_key(key), i % 2 == 0);
    }
}

#[test]
fn drain() {
    let tracker = DropTracker::new();
    let mut keys = GenMap::new();
    let mut map = SecondaryMap::<DropItem>::new();
    for _ in 0..5 {
        map.insert(keys.insert(()), tracker.make_item()).unwrap();
    }

    // Take two values, then drop the iterator, which drops the other three.
    let mut drain = map.drain();
    assert_eq!(drain.len(), 5);
    let taken: Vec<_> = drain.by_ref().take(2).collect();
    assert_eq!(drain.len(), 3);
    drop(drain);
    assert_eq!(tracker.total_dropped(), 3);
    assert!(map.is_empty());
    assert_eq!(map.iter().count(), 0);

    drop(taken);
    tracker.assert_all_dropped_exactly_once(5);
}

#[test]
fn a_drained_map_takes_values_again() {
    let mut keys = GenMap::new();
    let a = keys.insert(());
    let b = keys.insert(());
    let mut map = SecondaryMap::<u32>::new();
    map.insert(a, 1).unwrap();
    map.insert(b, 2).unwrap();

    let mut drained: Vec<(Key, u32)> = map.drain().collect();
    drained.sort();
    assert_eq!(drained, [(a, 1), (b, 2)]);
    assert_eq!(map.get(a), None);

    assert_eq!(map.insert(b, 3).unwrap(), None);
    assert_eq!(map.len(), 1);
    assert_eq!(map[b], 3);
}

#[test]
fn every_value_is_dropped_once() {
    let tracker = DropTracker::new();
    let mut keys = GenMap::new();
    let old = keys.insert(());
    let other = keys.insert(());
    keys.remove(old);
    let new = keys.insert(());
    assert_eq!(new.idx(), old.idx());

    let mut map = SecondaryMap::<DropItem>::new();
    map.insert(old, tracker.make_item()).unwrap();
    map.insert(other, tracker.make_item()).unwrap();
    let replaced = map.insert(other, tracker.make_item()).unwrap();
    let overridden = map.insert(new, tracker.make_item()).unwrap();
    let handed_back = refused(map.insert(old, tracker.make_item()));
    tracker.assert_none_dropped();

    drop((replaced, overridden, handed_back));
    assert_eq!(tracker.total_dropped(), 3);
    drop(map.remove(other));
    assert_eq!(tracker.total_dropped(), 4);
    drop(map);
    tracker.assert_all_dropped_exactly_once(5);
}

#[test]
fn clone_debug_default_and_collect() {
    let mut keys = GenMap::new();
    let a = keys.insert(());
    let map: SecondaryMap<u32> = [(a, 1)].into_iter().collect();
    let copy = map.clone();
    assert_eq!(copy[a], 1);
    assert_eq!(format!("{map:?}"), format!("{{{a:?}: 1}}"));
    assert!(SecondaryMap::<u32>::default().is_empty());
}

#[test]
fn extend_drops_the_values_whose_insert_is_refused() {
    let (old, new) = reused_keys();
    let mut map = SecondaryMap::<u32>::new();
    map.extend([(new, 1), (old, 2)]);
    assert_eq!(map.len(), 1);
    assert_eq!(map[new], 1);
}

#[should_panic(expected = "SecondaryMap key")]
#[test]
fn indexing_with_a_key_that_has_no_value_panics() {
    let mut keys = GenMap::new();
    let map = SecondaryMap::<u32>::new();
    let _ = map[keys.insert(())];
}

#[test]
fn a_secondary_map_iterates_in_index_order() {
    let mut keys = GenMap::new();
    let all: Vec<Key> = (0..4).map(|_| keys.insert(())).collect();
    let mut map = SecondaryMap::<u32>::new();
    for &i in &[3, 0, 2] {
        map.insert(all[i], i as u32).unwrap();
    }
    let pairs: Vec<(Key, u32)> = map.iter().map(|(key, &value)| (key, value)).collect();
    assert_eq!(pairs, [(all[0], 0), (all[2], 2), (all[3], 3)]);
    let drained: Vec<u32> = map.drain().map(|(_, value)| value).collect();
    assert_eq!(drained, [0, 2, 3]);
}

#[test]
fn a_secondary_map_keeps_a_slot_up_to_the_largest_index_until_it_is_cleared() {
    let mut map = SecondaryMap::<u32, Cfg<u8, u8>>::new_with_config();
    let last = key_from_parts::<Cfg<u8, u8>>(255, 1);
    assert_eq!(map.insert(last, 7).unwrap(), None);
    assert_eq!(map.slots_len(), 256);
    assert_eq!(map.iter().collect::<Vec<_>>(), [(last, &7)]);

    // Removing leaves the slot, and clearing removes every slot.
    assert_eq!(map.remove(last), Some(7));
    assert_eq!(map.slots_len(), 256);
    map.clear();
    assert_eq!(map.slots_len(), 0);

    // A drain removes every slot too, even when it is dropped early.
    map.insert(last, 8).unwrap();
    drop(map.drain());
    assert_eq!(map.slots_len(), 0);
    assert!(map.is_empty());
}

#[test]
fn unchecked_lookups_find_the_same_values() {
    let mut keys = GenMap::new();
    let a = keys.insert(());
    let b = keys.insert(());
    let mut map = SecondaryMap::<u32>::new();
    map.insert(a, 1).unwrap();
    map.insert(b, 2).unwrap();

    // SAFETY: the map has a value for `a` and for `b`.
    unsafe {
        assert_eq!(*map.get_unchecked(a), 1);
        *map.get_unchecked_mut(b) += 10;
        assert_eq!(*map.get_unchecked(b), 12);
    }
    assert_eq!(map[b], 12);
}

#[test]
fn a_clone_has_the_same_values_and_keys() {
    let (old, new) = reused_keys();
    let mut map = SecondaryMap::<u32, Keep>::new_with_config();
    map.insert(old, 1).unwrap();
    let mut copy = map.clone();
    assert_eq!(copy.iter().collect::<Vec<_>>(), [(old, &1)]);
    // The clone keeps the generation, so the newer key is still refused.
    assert_eq!(refused(copy.insert(new, 2)), 2);
    assert_eq!(copy.remove(old), Some(1));
    assert_eq!(map[old], 1);
}

#[test]
fn an_index_past_usize_fails_in_a_secondary_map_before_adding_a_slot() {
    type Wide = Cfg<u128, u32>;

    let key = key_from_parts::<Wide>(u128::MAX, 1);
    let mut map = SecondaryMap::<u8, Wide>::new_with_config();
    assert!(matches!(
        map.insert(key, 5),
        Err(SecondaryInsertError::StorageFull(5, _))
    ));
    assert!(map.is_empty());
    assert_eq!(map.slots_len(), 0);
    assert_eq!(map.get(key), None);
    assert_eq!(map.remove(key), None);
}

#[cfg(target_pointer_width = "64")]
#[test]
fn a_far_index_fails_in_a_secondary_map_before_adding_a_slot() {
    type Big = Cfg<u64, u32>;

    // A `Vec` needs a slot at every index up to 2^62, which is more bytes
    // than it can ask for, so it fails without allocating or pushing.
    let key = key_from_parts::<Big>(1 << 62, 1);
    let mut map = SecondaryMap::<u8, Big>::new_with_config();
    assert!(matches!(
        map.insert(key, 5),
        Err(SecondaryInsertError::StorageFull(5, _))
    ));
    assert!(map.is_empty());
    assert_eq!(map.slots_len(), 0);
}

#[test]
fn the_strategies_compare_generations_as_documented() {
    let plain = <NewerWins as ReplaceStrategy<Wrap4>>::replaces;
    let wrapping = <NewerWinsWrapping as ReplaceStrategy<Wrap4>>::replaces;
    let keep = <ExistingWins as ReplaceStrategy<Wrap4>>::replaces;
    // The slot's generation, the key's, and whether each of the first two
    // strategies replaces the value. These are the examples in the docs.
    let cases = [
        (5, 7, true, true),
        (7, 5, false, false),
        (15, 1, false, true),
        (3, 13, true, false),
        (13, 3, false, true),
    ];
    for (slot, key, by_plain, by_wrapping) in cases {
        assert_eq!(
            plain(odd(slot), odd(key)),
            by_plain,
            "plain, {slot} to {key}"
        );
        assert_eq!(
            wrapping(odd(slot), odd(key)),
            by_wrapping,
            "wrapping, {slot} to {key}"
        );
        assert!(!keep(odd(slot), odd(key)));
    }
}

#[test]
fn the_wrapping_strategy_uses_the_whole_range_of_wide_generations() {
    // A `u32` generation, as in the default key config.
    let wrapping = <NewerWinsWrapping as ReplaceStrategy<DefaultKeyConfig>>::replaces;
    assert!(wrapping(odd(u32::MAX), odd(1)));
    assert!(!wrapping(odd(1), odd(u32::MAX)));

    // A `u128` generation, whose number of generations does not fit in a
    // `u128`. Half the range is `1 << 127` steps.
    let wrapping = <NewerWinsWrapping as ReplaceStrategy<Cfg<u32, u128>>>::replaces;
    assert!(wrapping(odd(u128::MAX), odd(1)));
    assert!(!wrapping(odd(1), odd(u128::MAX)));
    assert!(wrapping(odd(1), odd((1 << 127) - 1)));
    assert!(!wrapping(odd(1), odd((1 << 127) + 1)));
}

#[test]
fn a_secondary_slot_holds_a_value_only_while_its_generation_is_odd() {
    let mut slot = SecondarySlot::<Cfg<u8, u8>, u32>::new(odd(3), 7);
    assert_eq!(
        format!("{slot:?}"),
        "SecondarySlot { generation: 3, value: Some(7) }"
    );
    *slot.get_mut().unwrap().1 += 1;
    assert_eq!(slot.get_odd(odd(3)), Some(&8));
    assert_eq!(slot.get_odd(odd(5)), None);
    *slot.get_odd_mut(odd(3)).unwrap() -= 1;
    assert_eq!(slot.get_odd_mut(odd(5)), None);
    *slot.get_odd_mut(odd(3)).unwrap() += 1;
    assert_eq!(slot.replace(odd(5), 9), Some(8));
    assert_eq!(slot.clone().into_inner(), Some((odd(5), 9)));
    assert_eq!(slot.take(), Some(9));
    assert_eq!(slot.take(), None);
    assert_eq!(
        format!("{slot:?}"),
        "SecondarySlot { generation: 0, value: None }"
    );
    assert_eq!(slot.into_inner(), None);
    assert!(SecondarySlot::<Cfg<u8, u8>, u32>::empty().get().is_none());
}

/// A small random number generator, so the test below is the same on every
/// run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// What an insert into `model` does under `NewerWinsWrapping` with `u32`
/// generations. The model maps an index to the generation and the value
/// there.
fn model_insert(
    model: &mut HashMap<u32, (u32, u32)>,
    key: Key,
    value: u32,
) -> Result<Option<u32>, u32> {
    let generation = key.generation().get().get();
    match model.get_mut(&key.idx()) {
        None => {
            model.insert(key.idx(), (generation, value));
            Ok(None)
        }
        Some((current, old))
            if *current == generation || generation.wrapping_sub(*current) < 1 << 31 =>
        {
            *current = generation;
            Ok(Some(core::mem::replace(old, value)))
        }
        Some(_) => Err(value),
    }
}

/// Default keys and the default strategy, with slots in a `SmallVec` that
/// keeps four of them inline.
#[cfg(feature = "smallvec")]
struct InSmallVec;

#[cfg(feature = "smallvec")]
impl<T> MapConfig<T> for InSmallVec {
    type KeyConfig = DefaultKeyConfig;
}

#[cfg(feature = "smallvec")]
impl<S: SecondarySlotItem> SecondaryMapConfig<S> for InSmallVec {
    type ReplaceStrategy = NewerWinsWrapping;
    type Storage = smallvec::SmallVec<S, 4>;
}

/// Default keys and the default strategy, with room for 16 slots in an
/// `ArrayVec`, so that even a short run fills it.
#[cfg(feature = "arrayvec")]
struct InArrayVec;

#[cfg(feature = "arrayvec")]
impl<T> MapConfig<T> for InArrayVec {
    type KeyConfig = DefaultKeyConfig;
}

#[cfg(feature = "arrayvec")]
impl<S: SecondarySlotItem> SecondaryMapConfig<S> for InArrayVec {
    type ReplaceStrategy = NewerWinsWrapping;
    type Storage = arrayvec::ArrayVec<S, 16>;
}

/// Picks one of the keys in `live` and `dead` at random.
fn pick(rng: &mut Rng, live: &[Key], dead: &[Key]) -> Key {
    let i = rng.below(live.len() + dead.len());
    if i < live.len() {
        live[i]
    } else {
        dead[i - live.len()]
    }
}

/// How often the model runs met each case, so the test shows it covered
/// them.
#[derive(Default)]
struct Coverage {
    /// Inserts that met a value from a different generation.
    met_older_or_newer: usize,
    /// Inserts that met such a value and replaced it.
    replaced_it: usize,
    /// Inserts past the capacity of the storage.
    full: usize,
}

/// Runs the model test on a map with config `C` for every seed. Miri is far
/// slower than a normal run, so it gets fewer and shorter runs, and those
/// are too short to be sure of meeting every case.
fn follow_the_model<C>(capacity: Option<usize>)
where
    C: SecondaryMapConfigFor<u32> + MapConfig<u32, KeyConfig = DefaultKeyConfig>,
{
    let (seeds, steps) = if cfg!(miri) { (2, 300) } else { (8, 2000) };
    let mut coverage = Coverage::default();
    for seed in 0..seeds {
        run_model::<C>(seed, steps, capacity, &mut coverage);
    }
    if !cfg!(miri) {
        assert!(coverage.replaced_it > 0 && coverage.replaced_it < coverage.met_older_or_newer);
        assert_eq!(coverage.full > 0, capacity.is_some());
    }
}

/// Runs the same random inserts, removes and retains on a map with config
/// `C` and on the model above, and checks that the two agree after every
/// step. When the storage holds at most `capacity` slots, an insert at that
/// index or past it must fail with `StorageFull` and leave the map as it was.
fn run_model<C>(seed: u64, steps: u32, capacity: Option<usize>, coverage: &mut Coverage)
where
    C: SecondaryMapConfigFor<u32> + MapConfig<u32, KeyConfig = DefaultKeyConfig>,
{
    let mut rng = Rng(seed);
    let mut keys = GenMap::new();
    let mut live: Vec<Key> = Vec::new();
    let mut dead: Vec<Key> = Vec::new();
    let mut map = SecondaryMap::<u32, C>::new_with_config();
    let mut model: HashMap<u32, (u32, u32)> = HashMap::new();

    for step in 0..steps {
        match rng.below(100) {
            0..=24 => live.push(keys.insert(())),
            25..=39 if !live.is_empty() => {
                let key = live.swap_remove(rng.below(live.len()));
                keys.remove(key);
                dead.push(key);
            }
            40..=74 if !live.is_empty() || !dead.is_empty() => {
                let key = pick(&mut rng, &live, &dead);
                if capacity.is_some_and(|capacity| key.idx() as usize >= capacity) {
                    coverage.full += 1;
                    assert!(
                        matches!(
                            map.insert(key, step),
                            Err(SecondaryInsertError::StorageFull(value, _)) if value == step
                        ),
                        "insert past the capacity at step {step} of seed {seed}"
                    );
                } else {
                    let before = model.get(&key.idx()).map(|&(generation, _)| generation);
                    let expected = model_insert(&mut model, key, step);
                    if before.is_some_and(|current| current != key.generation().get().get()) {
                        coverage.met_older_or_newer += 1;
                        coverage.replaced_it += usize::from(expected.is_ok());
                    }
                    let found = map.insert(key, step).map_err(|error| error.into_inner());
                    assert_eq!(found, expected, "insert at step {step} of seed {seed}");
                }
            }
            75..=94 if !live.is_empty() || !dead.is_empty() => {
                let key = pick(&mut rng, &live, &dead);
                let generation = key.generation().get().get();
                let expected = match model.get(&key.idx()) {
                    Some(&(current, _)) if current == generation => {
                        model.remove(&key.idx()).map(|(_, value)| value)
                    }
                    _ => None,
                };
                assert_eq!(
                    map.remove(key),
                    expected,
                    "remove at step {step} of seed {seed}"
                );
            }
            95..=99 => {
                model.retain(|_, (_, value)| *value % 3 != 0);
                map.retain(|_, value| *value % 3 != 0);
            }
            _ => {}
        }

        assert_eq!(map.len(), model.len());
        if step % 100 == 0 || step + 1 == steps {
            let mut expected: Vec<(u32, u32, u32)> = model
                .iter()
                .map(|(&idx, &(generation, value))| (idx, generation, value))
                .collect();
            expected.sort();
            // The map iterates in index order, which the sorted model matches.
            let found: Vec<(u32, u32, u32)> = map
                .iter()
                .map(|(key, &value)| (key.idx(), key.generation().get().get(), value))
                .collect();
            assert_eq!(found, expected, "contents at step {step} of seed {seed}");
            for (key, value) in map.iter() {
                assert_eq!(map.get_at(key.idx()), Some((key, value)));
                let generation = key.generation().get().get();
                assert_eq!(map.generation_at(key.idx()), Some(generation));
            }
        }
    }
}

#[test]
fn a_secondary_map_follows_the_model() {
    follow_the_model::<crate::DefaultMapConfig>(None);
}

#[cfg(feature = "smallvec")]
#[test]
fn a_secondary_map_in_a_small_vec_follows_the_model() {
    follow_the_model::<InSmallVec>(None);
}

#[cfg(feature = "arrayvec")]
#[test]
fn a_secondary_map_in_an_array_vec_follows_the_model() {
    follow_the_model::<InArrayVec>(Some(16));
}

#[test]
fn retain_stays_consistent_when_the_closure_panics() {
    let tracker = DropTracker::new();
    let mut keys = GenMap::new();
    let mut map = SecondaryMap::<DropItem>::new();
    let all: Vec<Key> = (0..6)
        .map(|_| {
            let key = keys.insert(());
            map.insert(key, tracker.make_item()).unwrap();
            key
        })
        .collect();

    // The closure removes the first two values and panics on the third.
    let mut calls = 0;
    let result = catch_unwind(AssertUnwindSafe(|| {
        map.retain(|_, _| {
            calls += 1;
            assert!(calls < 3, "the closure panics");
            false
        });
    }));
    assert!(result.is_err());
    assert_eq!(map.len(), 4);
    assert_eq!(map.iter().count(), 4);
    assert!(!map.contains_key(all[0]) && !map.contains_key(all[1]));
    assert!(all[2..].iter().all(|&key| map.contains_key(key)));
    assert_eq!(tracker.total_dropped(), 2);
    drop(map);
    tracker.assert_all_dropped_exactly_once(6);
}

#[test]
fn retain_stays_consistent_when_a_drop_panics() {
    let tracker = DropTracker::new();
    let mut keys = GenMap::new();
    let mut map = SecondaryMap::<Bomb>::new();
    for i in 0..4 {
        map.insert(keys.insert(()), Bomb::new(&tracker, i == 1))
            .unwrap();
    }

    let result = catch_unwind(AssertUnwindSafe(|| map.retain(|_, _| false)));
    assert!(result.is_err());
    // The second value was already out of its slot when its drop panicked,
    // so it counts as removed, and the last two are still there.
    assert_eq!(map.len(), 2);
    assert_eq!(map.iter().count(), 2);
    drop(map);
    tracker.assert_all_dropped_exactly_once(4);
}

#[test]
fn clear_and_drain_empty_the_map_when_a_drop_panics() {
    for drain in [false, true] {
        let tracker = DropTracker::new();
        let mut keys = GenMap::new();
        let mut map = SecondaryMap::<Bomb>::new();
        for i in 0..4 {
            map.insert(keys.insert(()), Bomb::new(&tracker, i == 2))
                .unwrap();
        }

        let result = catch_unwind(AssertUnwindSafe(|| {
            if drain {
                // The drain takes the first value, and dropping it drops the
                // other three, one of which panics.
                let mut iter = map.drain();
                drop(iter.next());
                drop(iter);
            } else {
                map.clear();
            }
        }));
        assert!(result.is_err());
        assert!(map.is_empty());
        assert_eq!(map.slots_len(), 0);
        assert_eq!(map.iter().count(), 0);
        tracker.assert_all_dropped_exactly_once(4);
    }
}

#[test]
fn into_iter_drops_every_value_once() {
    let tracker = DropTracker::new();
    let mut keys = GenMap::new();

    // Dropping the iterator early drops the values it has not reached.
    let mut map = SecondaryMap::<DropItem>::new();
    for _ in 0..4 {
        map.insert(keys.insert(()), tracker.make_item()).unwrap();
    }
    let mut iter = map.into_iter();
    drop(iter.next());
    drop(iter);
    tracker.assert_all_dropped_exactly_once(4);

    // So does a drop that panics on the way.
    let tracker = DropTracker::new();
    let mut map = SecondaryMap::<Bomb>::new();
    for i in 0..4 {
        map.insert(keys.insert(()), Bomb::new(&tracker, i == 2))
            .unwrap();
    }
    let iter = map.into_iter();
    assert!(catch_unwind(AssertUnwindSafe(move || drop(iter))).is_err());
    tracker.assert_all_dropped_exactly_once(4);
}

#[test]
fn clone_leaves_no_trace_when_a_value_panics_while_cloning() {
    /// A value whose clone panics when `panics` is set.
    struct PanicsOnClone {
        item: DropItem,
        panics: bool,
    }

    impl Clone for PanicsOnClone {
        fn clone(&self) -> Self {
            assert!(!self.panics, "this value cannot be cloned");
            PanicsOnClone {
                item: self.item.clone(),
                panics: false,
            }
        }
    }

    let tracker = DropTracker::new();
    let mut keys = GenMap::new();
    let mut map = SecondaryMap::<PanicsOnClone>::new();
    for i in 0..4 {
        let value = PanicsOnClone {
            item: tracker.make_item(),
            panics: i == 2,
        };
        map.insert(keys.insert(()), value).unwrap();
    }

    let result = catch_unwind(AssertUnwindSafe(|| map.clone()));
    assert!(result.is_err());
    // The two clones made before the panic were dropped with the half built
    // storage, and the original is untouched.
    assert_eq!(tracker.total_made(), 6);
    assert_eq!(tracker.total_dropped(), 2);
    assert_eq!(map.len(), 4);
    assert_eq!(map.values().filter(|value| value.panics).count(), 1);
    drop(map);
    tracker.assert_all_dropped_exactly_once(6);
}

#[test]
fn a_leaked_drain_leaves_the_rest_in_the_map() {
    let mut keys = GenMap::new();
    let all: Vec<Key> = (0..4).map(|_| keys.insert(())).collect();
    let mut map = SecondaryMap::<u32>::new();
    for (i, &key) in all.iter().enumerate() {
        map.insert(key, i as u32).unwrap();
    }

    let mut drain = map.drain();
    assert_eq!(drain.next(), Some((all[0], 0)));
    core::mem::forget(drain);
    assert_eq!(map.len(), 3);
    assert_eq!(map.get(all[0]), None);
    let rest: Vec<(Key, u32)> = map.iter().map(|(key, &value)| (key, value)).collect();
    assert_eq!(rest, [(all[1], 1), (all[2], 2), (all[3], 3)]);
}

#[test]
fn zero_sized_values_work() {
    let mut keys = GenMap::new();
    let all: Vec<Key> = (0..3).map(|_| keys.insert(())).collect();
    let mut map = SecondaryMap::<()>::new();
    map.insert(all[0], ()).unwrap();
    map.insert(all[2], ()).unwrap();

    assert_eq!(map.len(), 2);
    assert_eq!(map.get(all[1]), None);
    assert_eq!(map.keys().collect::<Vec<_>>(), [all[0], all[2]]);
    assert_eq!(map.remove(all[0]), Some(()));
    assert_eq!(map.drain().collect::<Vec<_>>(), [(all[2], ())]);
    assert!(map.is_empty());
}

#[test]
fn every_iterator_reports_how_many_values_it_has_left() {
    /// Checks that `iter` yields `total` items, that its length counts down
    /// with them, and that it stays empty afterwards.
    fn check<I: ExactSizeIterator>(mut iter: I, total: usize) {
        assert_eq!(iter.len(), total);
        for left in (0..total).rev() {
            assert!(iter.next().is_some());
            assert_eq!(iter.len(), left);
        }
        assert!(iter.next().is_none());
        assert!(iter.next().is_none());
        assert_eq!(iter.len(), 0);
    }

    let mut keys = GenMap::new();
    let all: Vec<Key> = (0..5).map(|_| keys.insert(())).collect();
    let mut map = SecondaryMap::<u32>::new();
    // Every other index, so there are empty slots between the values.
    for i in [0, 2, 4] {
        map.insert(all[i], i as u32).unwrap();
    }

    check(map.iter(), 3);
    check(map.keys(), 3);
    check(map.values(), 3);
    check(map.iter_mut(), 3);
    check(map.values_mut(), 3);
    check(map.clone().into_iter(), 3);
    check(map.drain(), 3);
    assert!(map.is_empty());
}

/// A map with values at indices 0 and 2, so the slot at index 1 is empty and
/// there is no slot at index 3. The keys are the first four of a `GenMap`.
fn map_with_a_gap() -> (SecondaryMap<u32>, Vec<Key>) {
    let mut keys = GenMap::new();
    let all: Vec<Key> = (0..4).map(|_| keys.insert(())).collect();
    let mut map = SecondaryMap::new();
    map.insert(all[0], 10).unwrap();
    map.insert(all[2], 12).unwrap();
    (map, all)
}

#[test]
fn key_at_and_get_at_find_only_the_slots_that_hold_a_value() {
    let (mut map, all) = map_with_a_gap();
    assert_eq!(map.key_at(0), Some(all[0]));
    assert_eq!(map.key_at(1), None);
    assert_eq!(map.key_at(3), None);
    assert_eq!(map.get_at(2), Some((all[2], &12)));
    assert_eq!(map.get_at(1), None);
    assert_eq!(map.get_at(3), None);

    let (key, value) = map.get_at_mut(2).unwrap();
    assert_eq!(key, all[2]);
    *value += 1;
    assert_eq!(map[all[2]], 13);
    assert!(map.get_at_mut(1).is_none());
    assert!(map.get_at_mut(3).is_none());
}

#[test]
fn generation_at_is_the_keys_generation_or_zero() {
    let (map, all) = map_with_a_gap();
    assert_eq!(map.generation_at(0), Some(all[0].generation().get().get()));
    assert_eq!(map.generation_at(1), Some(0));
    assert_eq!(map.generation_at(3), None);
}

#[test]
fn the_index_lookups_follow_a_replaced_and_removed_value() {
    let (old, new) = reused_keys();
    let mut map = SecondaryMap::<u32>::new();
    map.insert(old, 1).unwrap();
    assert_eq!(map.key_at(old.idx()), Some(old));
    map.insert(new, 2).unwrap();
    assert_eq!(map.key_at(new.idx()), Some(new));
    assert_eq!(map.get_at(new.idx()), Some((new, &2)));

    // Removing the value leaves an empty slot with generation zero.
    map.remove(new);
    assert_eq!(map.key_at(new.idx()), None);
    assert_eq!(map.generation_at(new.idx()), Some(0));
}

#[test]
fn the_index_lookups_find_nothing_past_usize() {
    type Wide = Cfg<u128, u32>;

    let mut map = SecondaryMap::<u8, Wide>::new_with_config();
    map.insert(key_from_parts::<Wide>(0, 1), 1).unwrap();
    assert_eq!(map.key_at(u128::MAX), None);
    assert_eq!(map.get_at(u128::MAX), None);
    assert!(map.get_at_mut(u128::MAX).is_none());
    assert_eq!(map.generation_at(u128::MAX), None);
    assert_eq!(
        map.get_disjoint_mut_at([0, u128::MAX]),
        Err(GetDisjointMutAtError::NoValue)
    );
}

#[test]
fn the_unchecked_index_lookups_agree_with_the_checked_ones() {
    let (mut map, all) = map_with_a_gap();
    for idx in [0, 2] {
        // SAFETY: the slots at 0 and 2 hold values.
        unsafe {
            assert_eq!(Some(map.key_at_unchecked(idx)), map.key_at(idx));
            assert_eq!(Some(map.get_at_unchecked(idx)), map.get_at(idx));
            *map.get_at_unchecked_mut(idx).1 += 100;
        }
    }
    assert_eq!((map[all[0]], map[all[2]]), (110, 112));

    for idx in 0..3 {
        // SAFETY: there are slots at 0, 1 and 2. The one at 1 holds no value,
        // which `generation_at_unchecked` allows.
        let generation = unsafe { map.generation_at_unchecked(idx) };
        assert_eq!(Some(generation), map.generation_at(idx));
    }
}

#[test]
fn get_disjoint_mut_hands_out_every_value() {
    let (mut map, all) = map_with_a_gap();
    let [x, y] = map.get_disjoint_mut([all[2], all[0]]).unwrap();
    assert_eq!((*x, *y), (12, 10));
    core::mem::swap(x, y);
    assert_eq!((map[all[0]], map[all[2]]), (12, 10));

    assert!(map.get_disjoint_mut::<0>([]).is_ok());
    let [only] = map.get_disjoint_mut([all[0]]).unwrap();
    *only = 1;
    assert_eq!(map[all[0]], 1);

    // SAFETY: both keys have a value, and they are in different slots.
    let [x, y] = unsafe { map.get_disjoint_mut_unchecked([all[0], all[2]]) };
    *x += 1;
    *y += 1;
    assert_eq!((map[all[0]], map[all[2]]), (2, 11));
}

#[test]
fn get_disjoint_mut_rejects_missing_stale_and_repeated_keys() {
    let (mut map, all) = map_with_a_gap();
    // The first key has an empty slot, and the second has no slot at all.
    assert_eq!(
        map.get_disjoint_mut([all[0], all[1]]),
        Err(GetDisjointMutError::InvalidKey)
    );
    assert_eq!(
        map.get_disjoint_mut([all[3]]),
        Err(GetDisjointMutError::InvalidKey)
    );
    assert_eq!(
        map.get_disjoint_mut([all[0], all[0]]),
        Err(GetDisjointMutError::OverlappingKeys)
    );

    // The older key lost its value to the newer one, so it has no value.
    let (old, new) = reused_keys();
    let mut map = SecondaryMap::<u32>::new();
    map.insert(old, 1).unwrap();
    map.insert(new, 2).unwrap();
    assert_eq!(
        map.get_disjoint_mut([new, old]),
        Err(GetDisjointMutError::InvalidKey)
    );
    assert!(map.get_disjoint_mut([new]).is_ok());
}

#[test]
fn get_disjoint_mut_at_hands_out_every_key_and_value() {
    let (mut map, all) = map_with_a_gap();
    let [(key_a, x), (key_b, y)] = map.get_disjoint_mut_at([2, 0]).unwrap();
    assert_eq!((key_a, key_b), (all[2], all[0]));
    core::mem::swap(x, y);
    assert_eq!((map[all[0]], map[all[2]]), (12, 10));

    // SAFETY: the slots at 0 and 2 hold values, and the indices differ.
    let [(key_a, x), (key_b, y)] = unsafe { map.get_disjoint_mut_at_unchecked([0, 2]) };
    assert_eq!((key_a, key_b), (all[0], all[2]));
    *x += 1;
    *y += 1;
    assert_eq!((map[all[0]], map[all[2]]), (13, 11));

    // The slot at 1 is empty, and there is no slot at 3.
    assert_eq!(
        map.get_disjoint_mut_at([0, 1]),
        Err(GetDisjointMutAtError::NoValue)
    );
    assert_eq!(
        map.get_disjoint_mut_at([3]),
        Err(GetDisjointMutAtError::NoValue)
    );
    assert_eq!(
        map.get_disjoint_mut_at([2, 2]),
        Err(GetDisjointMutAtError::OverlappingIndices)
    );
}

#[test]
fn values_swapped_through_the_disjoint_methods_drop_exactly_once() {
    let tracker = DropTracker::new();
    let mut keys = GenMap::new();
    let a = keys.insert(());
    let b = keys.insert(());
    let mut map = SecondaryMap::<DropItem>::new();
    map.insert(a, tracker.make_item()).unwrap();
    map.insert(b, tracker.make_item()).unwrap();

    let [x, y] = map.get_disjoint_mut([a, b]).unwrap();
    core::mem::swap(x, y);
    let [(_, x), (_, y)] = map.get_disjoint_mut_at([a.idx(), b.idx()]).unwrap();
    core::mem::swap(x, y);
    tracker.assert_none_dropped();
    drop(map);
    tracker.assert_all_dropped_exactly_once(2);
}
