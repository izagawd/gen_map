//! Tests for `SecondaryMap`.

use crate::tests::model::Rng;
use crate::tests::{key_from_parts, Bomb, Cfg, DropItem, DropTracker};
use crate::{
    DefaultKeyConfig, ExistingWins, GenMap, GenMapConfig, GenSlotItem, GetDisjointMutAtError,
    GetDisjointMutError, Key, MapConfig, MapKeyConfig, NewerWins, Odd, Packed, ReplaceStrategy,
    SecondaryInsertError, SecondaryMap, SecondaryMapConfig, SecondarySlotItem, Split,
};
use core::marker::PhantomData;
use std::collections::HashMap;
use std::format;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::vec::Vec;

/// Default keys and slots in a `Vec`, but a value is never replaced by an
/// insert under a different generation.
struct Keep;

impl MapConfig for Keep {
    type KeyConfig = DefaultKeyConfig;
}

impl SecondaryMapConfig for Keep {
    type ReplaceStrategy = ExistingWins;
    type Storage<S: SecondarySlotItem> = Vec<S>;
}

/// Keys with a 4 bit generation, which goes from 0 to 15. The `GenMap` that
/// hands them out wraps the generation.
struct Wrap4;

impl MapConfig for Wrap4 {
    type KeyConfig = Packed<u16, 4>;
}

impl GenMapConfig for Wrap4 {
    const WRAP_ON_OVERFLOW: bool = true;
    type Storage<S: GenSlotItem> = Vec<S>;
}

/// [`Wrap4`] keys with the strategy `R`.
struct With<R>(PhantomData<R>);

impl<R> MapConfig for With<R> {
    type KeyConfig = MapKeyConfig<Wrap4>;
}

impl<R: ReplaceStrategy<MapKeyConfig<Wrap4>>> SecondaryMapConfig for With<R> {
    type ReplaceStrategy = R;
    type Storage<S: SecondarySlotItem> = Vec<S>;
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
fn keys_around_a_wrap() -> (Key<Packed<u16, 4>>, Key<Packed<u16, 4>>) {
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
fn newer_wins_refuses_a_key_from_after_a_wrap() {
    let (before, after) = keys_around_a_wrap();
    let mut map = SecondaryMap::<u32, With<NewerWins>>::new_with_config();
    map.insert(before, 1).unwrap();
    assert_eq!(refused(map.insert(after, 2)), 2);
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
fn clone_debug_and_default() {
    let mut keys = GenMap::new();
    let a = keys.insert(());
    let mut map = SecondaryMap::<u32>::new();
    map.insert(a, 1).unwrap();
    let copy = map.clone();
    assert_eq!(copy[a], 1);
    assert_eq!(format!("{map:?}"), format!("{{{a:?}: 1}}"));
    assert!(SecondaryMap::<u32>::default().is_empty());
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
fn a_secondary_map_keeps_its_slots() {
    let mut map = SecondaryMap::<u32, Cfg<u8, u8>>::new_with_config();
    // 254 is the largest index a slot can have, since `u8::MAX` is never
    // used.
    let last = key_from_parts::<Split<u8, u8>>(254, 1);
    assert_eq!(map.insert(last, 7).unwrap(), None);
    assert_eq!(map.slots_len(), 255);
    assert_eq!(map.iter().collect::<Vec<_>>(), [(last, &7)]);

    // Removing, clearing and draining take the values out and leave the
    // slots, as they do in a `GenMap`.
    assert_eq!(map.remove(last), Some(7));
    assert_eq!(map.slots_len(), 255);
    map.insert(last, 8).unwrap();
    map.clear();
    assert!(map.is_empty());
    assert_eq!(map.slots_len(), 255);
    map.insert(last, 9).unwrap();
    drop(map.drain());
    assert!(map.is_empty());
    assert_eq!(map.slots_len(), 255);
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

    // `u128::MAX` itself is refused before the storage is asked, so the key
    // uses the index just below it.
    let key = key_from_parts::<MapKeyConfig<Wide>>(u128::MAX - 1, 1);
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
    let key = key_from_parts::<MapKeyConfig<Big>>(1 << 62, 1);
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
    let newer = <NewerWins as ReplaceStrategy<MapKeyConfig<Wrap4>>>::replaces;
    let keep = <ExistingWins as ReplaceStrategy<MapKeyConfig<Wrap4>>>::replaces;
    // Each case holds the slot's generation, the key's generation, and
    // whether `NewerWins` replaces the value.
    let cases = [
        (5, 7, true),
        (7, 5, false),
        (15, 1, false),
        (3, 13, true),
        (13, 3, false),
    ];
    for (slot, key, replaces) in cases {
        assert_eq!(newer(odd(slot), odd(key)), replaces, "{slot} to {key}");
        assert!(!keep(odd(slot), odd(key)));
    }
}

/// Inserts `value` under `key` into `model` the way a map with `NewerWins`
/// does, and returns what the map's `insert` would return. The model maps an
/// index to the generation and the value there.
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
        Some((current, old)) if generation >= *current => {
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
impl MapConfig for InSmallVec {
    type KeyConfig = DefaultKeyConfig;
}

#[cfg(feature = "smallvec")]
impl SecondaryMapConfig for InSmallVec {
    type ReplaceStrategy = NewerWins;
    type Storage<S: SecondarySlotItem> = smallvec::SmallVec<S, 4>;
}

/// Default keys and the default strategy, with room for 16 slots in an
/// `ArrayVec`, so that even a short run fills it.
#[cfg(feature = "arrayvec")]
struct InArrayVec;

#[cfg(feature = "arrayvec")]
impl MapConfig for InArrayVec {
    type KeyConfig = DefaultKeyConfig;
}

#[cfg(feature = "arrayvec")]
impl SecondaryMapConfig for InArrayVec {
    type ReplaceStrategy = NewerWins;
    type Storage<S: SecondarySlotItem> = arrayvec::ArrayVec<S, 16>;
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

/// How often each case came up in the model runs, so that the test can show
/// it covered every case.
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
    C: SecondaryMapConfig + MapConfig<KeyConfig = DefaultKeyConfig>,
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
/// `C` and on the model above. After every step, it checks that the map and
/// the model gave the same result and hold the same number of values. Every
/// hundred steps and after the last step, it also checks that they hold the
/// same keys and values. When the storage holds at most `capacity` slots, an
/// insert at that index or past it must fail with `StorageFull` and leave the
/// map as it was.
fn run_model<C>(seed: u64, steps: u32, capacity: Option<usize>, coverage: &mut Coverage)
where
    C: SecondaryMapConfig + MapConfig<KeyConfig = DefaultKeyConfig>,
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
            // Iterating from the back gives the same values in reverse.
            let backward: Vec<(u32, u32, u32)> = map
                .iter()
                .rev()
                .map(|(key, &value)| (key.idx(), key.generation().get().get(), value))
                .collect();
            assert!(
                backward.iter().rev().eq(found.iter()),
                "reversed contents at step {step} of seed {seed}"
            );
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
fn clear_and_drain_stay_consistent_when_a_drop_panics() {
    for drain in [false, true] {
        let tracker = DropTracker::new();
        let mut keys = GenMap::new();
        let mut map = SecondaryMap::<Bomb>::new();
        let all: Vec<Key> = (0..5)
            .map(|i| {
                let key = keys.insert(());
                map.insert(key, Bomb::new(&tracker, i == 2)).unwrap();
                key
            })
            .collect();

        let result = catch_unwind(AssertUnwindSafe(|| {
            if drain {
                // The drain takes the first value, and dropping it takes the
                // rest, until the value at 2 panics.
                let mut iter = map.drain();
                drop(iter.next());
                drop(iter);
            } else {
                map.clear();
            }
        }));
        assert!(result.is_err());

        // The panic stopped right after the value at 2 was taken out, so the
        // last two stay in the map, as they would in a `GenMap`.
        assert_eq!(map.len(), 2);
        assert!(all[..3].iter().all(|&key| !map.contains_key(key)));
        assert!(all[3..].iter().all(|&key| map.contains_key(key)));
        assert_eq!(map.slots_len(), 5);

        drop(map);
        tracker.assert_all_dropped_exactly_once(5);
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
    map.insert(key_from_parts::<MapKeyConfig<Wide>>(0, 1), 1)
        .unwrap();
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
    // `all[1]` has an empty slot, and `all[3]` has no slot at all.
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

#[test]
fn reserve_makes_room_for_slots_ahead_of_time() {
    let mut map = SecondaryMap::<u32>::new();
    assert_eq!(map.capacity(), 0);
    assert_eq!(map.slots_len(), 0);

    map.reserve(100);
    let capacity = map.capacity();
    assert!(capacity >= 100);
    // Filling the reserved slots does not grow the storage again.
    map.insert(key_from_parts::<DefaultKeyConfig>(99, 1), 1)
        .unwrap();
    assert_eq!(map.slots_len(), 100);
    assert_eq!(map.capacity(), capacity);

    assert!(map.try_reserve(50).is_ok());
    assert!(map.capacity() >= 150);
    // No storage has room for that many, and the map stays as it was.
    assert!(map.try_reserve(usize::MAX).is_err());
    assert_eq!(map.slots_len(), 100);
    assert_eq!(map.len(), 1);
}

#[test]
fn with_capacity_makes_room_up_front() {
    let map = SecondaryMap::<u32>::with_capacity(32);
    assert!(map.capacity() >= 32);
    assert!(map.is_empty());
    assert_eq!(map.slots_len(), 0);

    let map = SecondaryMap::<u32, Keep>::with_capacity_and_config(8);
    assert!(map.capacity() >= 8);
    assert!(map.is_empty());
}

#[test]
fn every_iterator_runs_from_both_ends() {
    let mut keys = GenMap::new();
    let all: Vec<Key> = (0..6).map(|_| keys.insert(())).collect();
    let mut map = SecondaryMap::<u32>::new();
    // Gaps at 1 and 4, so both ends have empty slots to skip.
    for i in [0, 2, 3, 5] {
        map.insert(all[i], i as u32).unwrap();
    }
    let forward: Vec<(Key, u32)> = map.iter().map(|(key, &value)| (key, value)).collect();
    let backward: Vec<(Key, u32)> = forward.iter().rev().copied().collect();
    let backward_keys: Vec<Key> = backward.iter().map(|&(key, _)| key).collect();
    let backward_values: Vec<u32> = backward.iter().map(|&(_, value)| value).collect();

    let pairs: Vec<(Key, u32)> = map.iter().rev().map(|(key, &value)| (key, value)).collect();
    assert_eq!(pairs, backward);
    assert_eq!(map.keys().rev().collect::<Vec<_>>(), backward_keys);
    assert_eq!(
        map.values().rev().copied().collect::<Vec<_>>(),
        backward_values
    );
    let pairs: Vec<(Key, u32)> = map
        .iter_mut()
        .rev()
        .map(|(key, &mut value)| (key, value))
        .collect();
    assert_eq!(pairs, backward);
    let values: Vec<u32> = map.values_mut().rev().map(|value| *value).collect();
    assert_eq!(values, backward_values);
    assert_eq!(map.clone().into_iter().rev().collect::<Vec<_>>(), backward);

    // Taking from both ends meets in the middle, without repeating or
    // skipping a value, and the length counts down with every step.
    let mut iter = map.iter();
    assert_eq!(iter.next(), Some((all[0], &0)));
    assert_eq!(iter.next_back(), Some((all[5], &5)));
    assert_eq!(iter.len(), 2);
    assert_eq!(iter.next_back(), Some((all[3], &3)));
    assert_eq!(iter.next(), Some((all[2], &2)));
    assert_eq!(iter.len(), 0);
    assert_eq!(iter.next(), None);
    assert_eq!(iter.next_back(), None);

    let mut owned = map.into_iter();
    assert_eq!(owned.next_back(), Some((all[5], 5)));
    assert_eq!(owned.next(), Some((all[0], 0)));
    assert_eq!(owned.len(), 2);
    assert_eq!(owned.next_back(), Some((all[3], 3)));
    assert_eq!(owned.next_back(), Some((all[2], 2)));
    assert_eq!(owned.next(), None);
    assert_eq!(owned.next_back(), None);
}

#[test]
fn insert_rejects_a_key_at_the_largest_index() {
    // No `GenMap` gives a slot the largest index, so only a hand-built key
    // can have it.
    let key = key_from_parts::<Split<u8, u8>>(u8::MAX, 1);
    let mut map = SecondaryMap::<&str, Cfg<u8, u8>>::new_with_config();
    let error = map.insert(key, "x").unwrap_err();
    assert!(matches!(error, SecondaryInsertError::IndexReserved("x")));
    assert_eq!(format!("{error:?}"), "IndexReserved(..)");
    assert!(format!("{error}").contains("largest value"));
    assert_eq!(error.into_inner(), "x");
    assert_eq!(map.len(), 0);
    assert_eq!(map.slots_len(), 0);
    assert!(map.get(key).is_none());
}

#[test]
fn a_value_under_every_key_a_byte_map_hands_out_fits_the_count() {
    let mut keys = GenMap::<(), Cfg<u8, u8>>::new_with_config();
    let mut map = SecondaryMap::<u32, Cfg<u8, u8>>::new_with_config();
    for i in 0..255 {
        assert_eq!(map.insert(keys.insert(()), i).unwrap(), None);
    }
    assert!(keys.try_insert(()).is_err());
    assert_eq!(map.len(), 255);
    assert_eq!(map.iter().len(), 255);
    assert_eq!(map.drain().count(), 255);
    assert!(map.is_empty());
}
