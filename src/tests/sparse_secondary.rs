//! Tests for `SparseSecondaryMap`. The model test at the end runs the same
//! random operations on a `SecondaryMap` and checks after every step that
//! both maps give the same results and hold the same keys and values. The
//! other tests cover what that check cannot, such as hashers, drops and
//! memory.

use super::model::Rng;
use super::{key_from_parts, Cfg, DropTracker};
use crate::{
    ExistingWins, GenMap, GetDisjointMutAtError, GetDisjointMutError, Key, MapConfig,
    SecondaryInsertError, SecondaryMap, SecondaryMapConfig, SecondarySlotItem, SparseSecondaryMap,
    SparseSecondaryMapConfig, Split,
};
use core::cell::Cell;
use core::hash::{BuildHasher, BuildHasherDefault, Hasher};
use std::format;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;
use std::vec::Vec;

/// This config gives keys a `u8` index and a `u8` generation, and its
/// secondary maps let a newer key replace a value.
type Newer = Cfg<u8, u8>;

/// This config gives keys the same types as [`Newer`], but its secondary maps
/// keep a value until it is removed.
struct Keep;

impl MapConfig for Keep {
    type KeyConfig = Split<u8, u8>;
}

impl SecondaryMapConfig for Keep {
    type ReplaceStrategy = ExistingWins;
    type Storage<S: SecondarySlotItem> = Vec<S>;
}

impl SparseSecondaryMapConfig for Keep {
    type ReplaceStrategy = ExistingWins;
}

/// This hasher gives every index the same hash, so every lookup in the
/// `HashMap` has to tell colliding indices apart.
#[derive(Default)]
struct Colliding;

impl Hasher for Colliding {
    fn finish(&self) -> u64 {
        0
    }

    fn write(&mut self, _bytes: &[u8]) {}
}

type Collide = BuildHasherDefault<Colliding>;

/// This hasher hashes correctly until it has built `good` hashers. After that,
/// it flips every bit of each hash, so the `HashMap` stops finding the values
/// it holds. The `BuildHasher` docs ask for hashers that do not change, but a
/// safe trait cannot promise it, so the map's safe methods must stay sound
/// with this one.
#[derive(Clone)]
struct Flaky {
    good: usize,
    built: Rc<Cell<usize>>,
}

impl BuildHasher for Flaky {
    type Hasher = FlakyHasher;

    fn build_hasher(&self) -> FlakyHasher {
        let built = self.built.get();
        self.built.set(built + 1);
        FlakyHasher {
            hash: 0,
            broken: built >= self.good,
        }
    }
}

struct FlakyHasher {
    hash: u64,
    broken: bool,
}

impl Hasher for FlakyHasher {
    fn finish(&self) -> u64 {
        if self.broken {
            !self.hash
        } else {
            self.hash
        }
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.hash = (self.hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01B3);
        }
    }
}

/// Hands out `n` keys from a `GenMap`.
fn keys(n: usize) -> Vec<Key> {
    let mut map = GenMap::new();
    (0..n).map(|_| map.insert(())).collect()
}

/// Builds a `Split<u8, u8>` key with index `idx` and generation `generation`.
fn key8(idx: u8, generation: u8) -> Key<Split<u8, u8>> {
    key_from_parts::<Split<u8, u8>>(idx, generation)
}

#[test]
fn a_newer_key_replaces_the_value_and_an_older_key_is_refused() {
    let mut map = SparseSecondaryMap::<&str, Newer>::new_with_config();
    let old = key8(3, 1);
    let new = key8(3, 5);
    assert_eq!(map.insert(old, "old").unwrap(), None);
    assert_eq!(map.insert(new, "new").unwrap(), Some("old"));
    assert_eq!(map.get(old), None);
    assert_eq!(map[new], "new");
    assert!(matches!(
        map.insert(old, "older"),
        Err(SecondaryInsertError::Refused("older"))
    ));
    assert_eq!(map.key_at(3), Some(new));
    assert_eq!(map.len(), 1);
}

#[test]
fn existing_wins_keeps_the_value_until_it_is_removed() {
    let mut map = SparseSecondaryMap::<&str, Keep>::new_with_config();
    let first = key8(0, 1);
    let second = key8(0, 3);
    map.insert(first, "first").unwrap();
    assert!(matches!(
        map.insert(second, "second"),
        Err(SecondaryInsertError::Refused("second"))
    ));
    assert_eq!(map.remove(second), None);
    assert_eq!(map.remove(first), Some("first"));
    assert_eq!(map.insert(second, "second").unwrap(), None);
    assert_eq!(map[second], "second");
}

#[test]
fn the_largest_index_is_reserved() {
    let mut map = SparseSecondaryMap::<u32, Newer>::new_with_config();
    assert!(matches!(
        map.insert(key8(u8::MAX, 1), 1),
        Err(SecondaryInsertError::IndexReserved(1))
    ));
    assert!(map.is_empty());
}

#[test]
fn a_key_with_another_generation_neither_reads_nor_removes() {
    let mut map = SparseSecondaryMap::<u32, Newer>::new_with_config();
    map.insert(key8(2, 3), 7).unwrap();
    let stale = key8(2, 1);
    assert!(!map.contains_key(stale));
    assert_eq!(map.get(stale), None);
    assert_eq!(map.get_mut(stale), None);
    assert_eq!(map.remove(stale), None);
    assert_eq!(map[key8(2, 3)], 7);
}

#[test]
fn a_high_index_takes_no_room_for_the_indices_below_it() {
    let mut people = GenMap::new();
    let mut last = people.insert(());
    for _ in 0..10_000 {
        last = people.insert(());
    }
    let mut map = SparseSecondaryMap::new();
    map.insert(last, "last").unwrap();
    assert_eq!(map.len(), 1);
    assert!(map.capacity() < 100);
    assert_eq!(map[last], "last");
}

#[test]
fn every_value_is_dropped_once() {
    let tracker = DropTracker::new();
    let k = keys(10);
    let mut map = SparseSecondaryMap::new();
    for key in &k {
        map.insert(*key, tracker.make_item()).unwrap();
    }
    drop(map.insert(k[0], tracker.make_item()));
    drop(map.remove(k[1]));
    map.retain(|key, _| key != k[2]);
    let mut drain = map.drain();
    drop(drain.next());
    drop(drain);
    assert!(map.is_empty());

    for key in &k {
        map.insert(*key, tracker.make_item()).unwrap();
    }
    let mut into_iter = map.clone().into_iter();
    drop(into_iter.next());
    drop(into_iter);
    map.clear();
    for key in &k[..3] {
        map.insert(*key, tracker.make_item()).unwrap();
    }
    drop(map);
    tracker.assert_all_dropped_exactly_once(tracker.total_made());
}

#[test]
fn lookups_work_when_every_index_collides() {
    let mut map = SparseSecondaryMap::<u32, Newer, Collide>::new_with_config();
    for idx in 0..20 {
        map.insert(key8(idx, 1), u32::from(idx)).unwrap();
    }
    for idx in 0..20 {
        assert_eq!(map[key8(idx, 1)], u32::from(idx));
        assert_eq!(map.key_at(idx), Some(key8(idx, 1)));
    }
    let [a, b, c] = map
        .get_disjoint_mut([key8(4, 1), key8(9, 1), key8(13, 1)])
        .unwrap();
    core::mem::swap(a, c);
    *b += 100;
    assert_eq!(
        (map[key8(4, 1)], map[key8(9, 1)], map[key8(13, 1)]),
        (13, 109, 4)
    );
    // The swap above moved 13 to index 4, and 4 to index 13.
    assert_eq!(map.remove(key8(4, 1)), Some(13));
    assert_eq!(map.remove(key8(6, 1)), Some(6));
    assert_eq!(map.len(), 18);
    assert_eq!(map.get(key8(4, 1)), None);
    assert_eq!(map[key8(13, 1)], 4);
}

#[test]
fn the_constructors_use_the_given_hasher_and_config() {
    let k = keys(1);
    let mut map = SparseSecondaryMap::with_hasher(Collide::default());
    map.insert(k[0], 1).unwrap();
    assert_eq!(map[k[0]], 1);
    let _: &Collide = map.hasher();

    let map = SparseSecondaryMap::<u32, _, _>::with_capacity_and_hasher(10, Collide::default());
    assert!(map.capacity() >= 10);
    let map = SparseSecondaryMap::<u32>::with_capacity(10);
    assert!(map.capacity() >= 10);

    let mut map = SparseSecondaryMap::<u32, Keep, Collide>::with_capacity_and_config(10);
    assert!(map.capacity() >= 10);
    map.insert(key8(1, 1), 1).unwrap();
    let mut map = SparseSecondaryMap::<u32, Keep, Collide>::with_capacity_and_hasher_and_config(
        10,
        Collide::default(),
    );
    assert!(map.capacity() >= 10);
    map.insert(key8(1, 1), 1).unwrap();
    let map = SparseSecondaryMap::<u32, Keep, Collide>::with_hasher_and_config(Collide::default());
    assert_eq!(map.capacity(), 0);
    let map: SparseSecondaryMap<u32, Keep> = SparseSecondaryMap::default();
    assert!(map.is_empty());
}

#[test]
fn a_clone_holds_the_same_values_under_the_same_keys() {
    let k = keys(4);
    let mut map = SparseSecondaryMap::new();
    for (i, key) in k.iter().enumerate() {
        map.insert(*key, i).unwrap();
    }
    let mut clone = map.clone();
    clone[k[0]] = 100;
    clone.remove(k[1]);
    assert_eq!(map[k[0]], 0);
    assert_eq!(map[k[1]], 1);
    assert_eq!(clone[k[0]], 100);
    assert_eq!(clone.get(k[1]), None);
    assert_eq!(clone[k[3]], 3);
}

#[test]
fn the_iterators_agree_with_each_other() {
    let k = keys(6);
    let mut map = SparseSecondaryMap::new();
    for (i, key) in k.iter().enumerate() {
        map.insert(*key, i).unwrap();
    }
    let mut pairs: Vec<_> = map.iter().map(|(key, value)| (key, *value)).collect();
    pairs.sort_unstable();
    assert_eq!(pairs, k.iter().copied().zip(0..).collect::<Vec<_>>());

    let mut keys: Vec<_> = map.keys().collect();
    keys.sort_unstable();
    assert_eq!(keys, k);
    let mut values: Vec<_> = map.values().copied().collect();
    values.sort_unstable();
    assert_eq!(values, [0, 1, 2, 3, 4, 5]);

    for value in map.values_mut() {
        *value *= 10;
    }
    for (key, value) in &mut map {
        *value += k.iter().position(|known| *known == key).unwrap();
    }
    for (key, value) in &map {
        assert_eq!(
            *value,
            k.iter().position(|known| *known == key).unwrap() * 11
        );
    }

    let mut iter = map.iter();
    assert_eq!(iter.len(), 6);
    iter.next();
    assert_eq!(iter.clone().count(), 5);
    assert_eq!(iter.len(), 5);
    assert_eq!(map.keys().len(), 6);
    assert_eq!(map.values().len(), 6);
    assert_eq!(map.iter_mut().len(), 6);
    assert_eq!(map.values_mut().len(), 6);

    let mut drained: Vec<_> = map.clone().drain().collect();
    drained.sort_unstable();
    let mut owned: Vec<_> = map.into_iter().collect();
    owned.sort_unstable();
    assert_eq!(drained, owned);
    assert_eq!(owned.len(), 6);
}

#[test]
fn get_disjoint_mut_checks_every_key() {
    let k = keys(3);
    let mut map = SparseSecondaryMap::new();
    map.insert(k[0], 0).unwrap();
    map.insert(k[1], 1).unwrap();
    assert_eq!(
        map.get_disjoint_mut([k[0], k[2]]),
        Err(GetDisjointMutError::InvalidKey)
    );
    assert_eq!(
        map.get_disjoint_mut([k[1], k[1]]),
        Err(GetDisjointMutError::OverlappingKeys)
    );
    let [] = map.get_disjoint_mut([]).unwrap();
    let [(a, x), (b, y)] = map.get_disjoint_mut_at([k[1].idx(), k[0].idx()]).unwrap();
    assert_eq!((a, b), (k[1], k[0]));
    core::mem::swap(x, y);
    assert_eq!((map[k[0]], map[k[1]]), (1, 0));
    assert_eq!(
        map.get_disjoint_mut_at([k[0].idx(), k[2].idx()]),
        Err(GetDisjointMutAtError::NoValue)
    );
    assert_eq!(
        map.get_disjoint_mut_at([k[0].idx(), k[0].idx()]),
        Err(GetDisjointMutAtError::OverlappingIndices)
    );
}

#[test]
fn reserving_more_than_any_map_can_hold_is_an_error() {
    let mut map = SparseSecondaryMap::<u8>::new();
    assert!(map.try_reserve(usize::MAX).is_err());
    map.try_reserve(10).unwrap();
    assert!(map.capacity() >= 10);
    map.reserve(20);
    assert!(map.capacity() >= 20);
}

#[test]
fn shrinking_frees_room_and_keeps_every_value() {
    let k = keys(1000);
    let mut map = SparseSecondaryMap::new();
    for (i, key) in k.iter().enumerate() {
        map.insert(*key, i).unwrap();
    }
    map.retain(|_, value| *value < 5);
    let before = map.capacity();
    map.shrink_to(100);
    assert!(map.capacity() >= 100);
    assert!(map.capacity() < before);
    map.shrink_to_fit();
    assert!(map.capacity() >= 5);
    assert!(map.capacity() < 100);
    assert_eq!(map.len(), 5);
    for (i, key) in k[..5].iter().enumerate() {
        assert_eq!(map[*key], i);
    }

    // Shrinking to more than the current capacity changes nothing.
    let capacity = map.capacity();
    map.shrink_to(10_000);
    assert_eq!(map.capacity(), capacity);

    map.clear();
    map.shrink_to_fit();
    assert_eq!(map.capacity(), 0);
    map.insert(k[0], 0).unwrap();
    assert_eq!(map[k[0]], 0);
}

#[test]
fn insert_hashes_the_index_once_and_remove_at_most_twice() {
    // A `Flaky` hasher that never breaks counts how many hashes the map makes.
    let built = Rc::new(Cell::new(0));
    let hasher = Flaky {
        good: usize::MAX,
        built: built.clone(),
    };
    let mut map = SparseSecondaryMap::<u32, Newer, Flaky>::with_hasher_and_config(hasher);
    // With room to spare, the map does not grow and rehash its values.
    map.reserve(10);
    for (key, value, expected) in [
        (key8(0, 1), 0, None),
        (key8(1, 1), 1, None),
        (key8(0, 1), 2, Some(0)),
        (key8(0, 3), 3, Some(2)),
    ] {
        built.set(0);
        assert_eq!(map.insert(key, value).unwrap(), expected);
        assert_eq!(built.get(), 1);
    }
    built.set(0);
    assert!(matches!(
        map.insert(key8(0, 1), 4),
        Err(SecondaryInsertError::Refused(4))
    ));
    assert_eq!(built.get(), 1);
    // A remove that finds a value under the key's generation hashes the index
    // twice, once to find the value and once to remove it. Any other remove
    // hashes it once.
    for (key, expected, hashes) in [
        (key8(0, 3), Some(3), 2),
        (key8(5, 1), None, 1),
        (key8(1, 3), None, 1),
    ] {
        built.set(0);
        assert_eq!(map.remove(key), expected);
        assert_eq!(built.get(), hashes);
    }
    assert_eq!(map[key8(1, 1)], 1);
}

#[test]
fn a_hasher_that_changes_its_hashes_cannot_cause_undefined_behavior() {
    let a = key8(1, 1);
    let b = key8(2, 1);
    // Each run lets the hasher give one more correct hash than the run before,
    // so some run breaks it between a method's check and the lookup after that
    // check. The safe methods may panic or give a wrong answer then, but they
    // must stay sound.
    for good in 0..64 {
        let hasher = Flaky {
            good,
            built: Rc::new(Cell::new(0)),
        };
        let mut map = SparseSecondaryMap::<u32, Newer, Flaky>::with_hasher_and_config(hasher);
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let _ = map.insert(a, 1);
            let _ = map.insert(b, 2);
            let _ = map.get_disjoint_mut([a, b]);
            let _ = map.get_disjoint_mut_at([1, 2]);
            let _ = (
                map.contains_key(a),
                map.get(a),
                map.key_at(1),
                map.get_at(2),
            );
            let _ = (map.get_mut(b).is_some(), map.get_at_mut(1).is_some());
            let _ = map.insert(key8(1, 3), 3);
            let _ = format!("{:?}", map.clone());
            let _ = map.iter().count() + map.keys().count() + map.values().count();
            map.retain(|_, value| *value != 2);
            map.shrink_to_fit();
            let _ = map.remove(a);
            let _ = map.drain().count();
        }));
    }
}

#[test]
#[should_panic(expected = "SparseSecondaryMap cannot make room")]
fn reserve_panics_when_the_map_cannot_make_room() {
    SparseSecondaryMap::<u8>::new().reserve(usize::MAX);
}

#[test]
#[should_panic(expected = "invalid SparseSecondaryMap key")]
fn indexing_with_a_missing_key_panics() {
    let k = keys(1);
    let map = SparseSecondaryMap::<u8>::new();
    let _ = map[k[0]];
}

#[test]
fn debug_lists_each_key_with_its_value() {
    let mut map = SparseSecondaryMap::<u32, Newer>::new_with_config();
    map.insert(key8(1, 3), 5).unwrap();
    assert_eq!(format!("{map:?}"), "{Key { idx: 1, generation: 3 }: 5}");
}

#[test]
fn the_map_can_move_to_and_be_shared_between_threads() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<SparseSecondaryMap<u32>>();
    assert_send_sync::<SparseSecondaryMap<u32, Keep, Collide>>();
}

/// This enum holds what an insert into a secondary map returned, without the
/// storage error, so that the results of both maps can be compared.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Stored(Option<u32>),
    Refused(u32),
    IndexReserved(u32),
    StorageFull(u32),
}

impl<S> From<Result<Option<u32>, SecondaryInsertError<u32, S>>> for Outcome {
    fn from(result: Result<Option<u32>, SecondaryInsertError<u32, S>>) -> Self {
        match result {
            Ok(old) => Self::Stored(old),
            Err(SecondaryInsertError::Refused(value)) => Self::Refused(value),
            Err(SecondaryInsertError::IndexReserved(value)) => Self::IndexReserved(value),
            Err(SecondaryInsertError::StorageFull(value, _)) => Self::StorageFull(value),
        }
    }
}

/// Returns how many seeds to run and how many steps each one takes. Miri gets
/// fewer and shorter runs.
fn seeds() -> (u64, usize) {
    if cfg!(miri) {
        (2, 200)
    } else {
        (24, 1500)
    }
}

/// Picks one of the keys seen so far, or `None` if there are none.
fn pick(known: &[Key<Split<u8, u8>>], rng: &mut Rng) -> Option<Key<Split<u8, u8>>> {
    (!known.is_empty()).then(|| known[rng.below(known.len())])
}

/// Picks an index that may hold a value, or now and then the reserved
/// largest index.
fn pick_idx(rng: &mut Rng) -> u8 {
    if rng.chance(2) {
        u8::MAX
    } else {
        rng.below(21) as u8
    }
}

/// Returns whether `retain` keeps a value. It only looks at the value, so both
/// maps keep the same values whatever order they visit them in.
fn keeps(value: u32) -> bool {
    value % 3 != 0
}

fn run<C, S>(seed: u64, steps: usize)
where
    C: SecondaryMapConfig<KeyConfig = Split<u8, u8>>
        + SparseSecondaryMapConfig<KeyConfig = Split<u8, u8>>,
    S: core::hash::BuildHasher + Default,
{
    let mut rng = Rng(seed);
    let mut secondary = SecondaryMap::<u32, C>::new_with_config();
    let mut sparse = SparseSecondaryMap::<u32, C, S>::new_with_config();
    let mut known: Vec<Key<Split<u8, u8>>> = Vec::new();
    for (step, value) in (0..steps).zip(1u32..) {
        let context = (seed, step);
        match rng.below(100) {
            0..=44 => {
                // The keys mostly have small indices and generations, so they
                // often meet a value under another generation.
                let generation = 1 + 2 * rng.below(5) as u8;
                let key = key8(pick_idx(&mut rng), generation);
                known.push(key);
                assert_eq!(
                    Outcome::from(secondary.insert(key, value)),
                    Outcome::from(sparse.insert(key, value)),
                    "{context:?}"
                );
            }
            45..=64 => {
                if let Some(key) = pick(&known, &mut rng) {
                    assert_eq!(secondary.remove(key), sparse.remove(key), "{context:?}");
                }
            }
            65..=79 => match rng.below(6) {
                0 => {
                    if let Some(key) = pick(&known, &mut rng) {
                        if let Some(old) = secondary.get_mut(key) {
                            *old = value;
                        }
                        if let Some(old) = sparse.get_mut(key) {
                            *old = value;
                        }
                    }
                }
                1 => {
                    if let Some(key) =
                        pick(&known, &mut rng).filter(|key| secondary.contains_key(*key))
                    {
                        secondary[key] = value;
                        sparse[key] = value;
                    }
                }
                2 => {
                    if let Some(key) = pick(&known, &mut rng) {
                        match (secondary.get_mut(key), sparse.get_mut(key)) {
                            (Some(old), Some(sparse_old)) => {
                                *old = value;
                                *sparse_old = value;
                            }
                            (None, None) => {}
                            _ => panic!("the maps disagree on get_mut at {context:?}"),
                        }
                    }
                }
                3 => {
                    let idx = pick_idx(&mut rng);
                    let set = |(key, old): (_, &mut u32)| {
                        *old = value;
                        key
                    };
                    assert_eq!(
                        secondary.get_at_mut(idx).map(set),
                        sparse.get_at_mut(idx).map(set),
                        "{context:?}"
                    );
                }
                4 => {
                    for (key, old) in secondary.iter_mut() {
                        *old = old.wrapping_add(u32::from(key.idx()));
                    }
                    for (key, old) in sparse.iter_mut() {
                        *old = old.wrapping_add(u32::from(key.idx()));
                    }
                }
                _ => {
                    for old in secondary.values_mut() {
                        *old = old.wrapping_add(value);
                    }
                    for old in sparse.values_mut() {
                        *old = old.wrapping_add(value);
                    }
                }
            },
            80..=86 => match rng.below(4) {
                0 => {
                    if let (Some(a), Some(b)) = (pick(&known, &mut rng), pick(&known, &mut rng)) {
                        match (
                            secondary.get_disjoint_mut([a, b]),
                            sparse.get_disjoint_mut([a, b]),
                        ) {
                            (Ok([x, y]), Ok([p, q])) => {
                                core::mem::swap(x, y);
                                core::mem::swap(p, q);
                            }
                            (Err(e), Err(f)) => assert_eq!(e, f, "{context:?}"),
                            _ => panic!("the maps disagree on get_disjoint_mut at {context:?}"),
                        }
                    }
                }
                1 => {
                    if let (Some(a), Some(b)) = (pick(&known, &mut rng), pick(&known, &mut rng)) {
                        if let Ok([x, y]) = secondary.get_disjoint_mut([a, b]) {
                            core::mem::swap(x, y);
                            // SAFETY: the `SecondaryMap` has a value for both
                            // keys at different indices, and `check` confirmed
                            // after the last step that both maps hold the same
                            // keys.
                            let [p, q] = unsafe { sparse.get_disjoint_mut_unchecked([a, b]) };
                            core::mem::swap(p, q);
                        }
                    }
                }
                2 => {
                    let indices = [pick_idx(&mut rng), pick_idx(&mut rng)];
                    match (
                        secondary.get_disjoint_mut_at(indices),
                        sparse.get_disjoint_mut_at(indices),
                    ) {
                        (Ok([(a, x), (b, y)]), Ok([(c, p), (d, q)])) => {
                            assert_eq!((a, b), (c, d), "{context:?}");
                            core::mem::swap(x, y);
                            core::mem::swap(p, q);
                        }
                        (Err(e), Err(f)) => assert_eq!(e, f, "{context:?}"),
                        _ => panic!("the maps disagree on get_disjoint_mut_at at {context:?}"),
                    }
                }
                _ => {
                    let indices = [pick_idx(&mut rng), pick_idx(&mut rng)];
                    if let Ok([(a, x), (b, y)]) = secondary.get_disjoint_mut_at(indices) {
                        core::mem::swap(x, y);
                        // SAFETY: the `SecondaryMap` has a value at both
                        // indices, which differ, and `check` confirmed after
                        // the last step that both maps hold the same keys.
                        let [(c, p), (d, q)] =
                            unsafe { sparse.get_disjoint_mut_at_unchecked(indices) };
                        assert_eq!((a, b), (c, d), "{context:?}");
                        core::mem::swap(p, q);
                    }
                }
            },
            87..=92 => {
                secondary.retain(|_, value| {
                    *value = value.wrapping_mul(7);
                    keeps(*value)
                });
                sparse.retain(|_, value| {
                    *value = value.wrapping_mul(7);
                    keeps(*value)
                });
            }
            93..=95 => {
                let mut a: Vec<_> = secondary.drain().collect();
                let mut b: Vec<_> = sparse.drain().collect();
                a.sort_unstable();
                b.sort_unstable();
                assert_eq!(a, b, "{context:?}");
            }
            96..=97 => {
                secondary.clear();
                sparse.clear();
            }
            // Only the sparse map can shrink, and shrinking must not change
            // what it holds.
            _ => {
                if rng.chance(50) {
                    sparse.shrink_to_fit();
                } else {
                    sparse.shrink_to(rng.below(40));
                }
            }
        }
        check(&secondary, &sparse, &known, context);
    }
}

fn check<C, S>(
    secondary: &SecondaryMap<u32, C>,
    sparse: &SparseSecondaryMap<u32, C, S>,
    known: &[Key<Split<u8, u8>>],
    context: (u64, usize),
) where
    C: SecondaryMapConfig<KeyConfig = Split<u8, u8>>
        + SparseSecondaryMapConfig<KeyConfig = Split<u8, u8>>,
    S: core::hash::BuildHasher,
{
    assert_eq!(secondary.len(), sparse.len(), "{context:?}");
    for idx in (0..=21).chain([u8::MAX]) {
        let key = secondary.key_at(idx);
        assert_eq!(key, sparse.key_at(idx), "{context:?}");
        assert_eq!(secondary.get_at(idx), sparse.get_at(idx), "{context:?}");
    }
    for key in known {
        assert_eq!(secondary.get(*key), sparse.get(*key), "{context:?}");
        assert_eq!(
            secondary.contains_key(*key),
            sparse.contains_key(*key),
            "{context:?}"
        );
    }
    let mut expected: Vec<_> = secondary.iter().map(|(key, value)| (key, *value)).collect();
    let mut actual: Vec<_> = sparse.iter().map(|(key, value)| (key, *value)).collect();
    expected.sort_unstable();
    actual.sort_unstable();
    assert_eq!(actual, expected, "{context:?}");
}

#[test]
fn a_sparse_secondary_map_agrees_with_a_secondary_map() {
    let (seeds, steps) = seeds();
    for seed in 0..seeds {
        run::<Newer, std::hash::RandomState>(seed, steps);
    }
}

#[test]
fn a_sparse_secondary_map_that_keeps_values_agrees_with_a_secondary_map() {
    let (seeds, steps) = seeds();
    for seed in 0..seeds {
        run::<Keep, std::hash::RandomState>(seed, steps);
    }
}

#[test]
fn a_sparse_secondary_map_whose_indices_collide_agrees_with_a_secondary_map() {
    let (seeds, steps) = seeds();
    for seed in 0..seeds {
        run::<Newer, Collide>(seed, steps);
    }
}

#[test]
fn removing_a_key_without_a_value_makes_no_room() {
    let all = keys(64);
    let missing = all[63];
    let mut map = SparseSecondaryMap::<u32>::new();
    assert_eq!(map.remove(missing), None);
    assert_eq!(map.capacity(), 0);

    // Once the map has no room left, removing a key without a value still
    // makes no room.
    let mut next = 0;
    while map.is_empty() || map.len() < map.capacity() {
        map.insert(all[next], 0).unwrap();
        next += 1;
    }
    assert!(next < 63);
    let capacity = map.capacity();
    assert_eq!(map.remove(missing), None);
    assert_eq!(map.capacity(), capacity);
}
