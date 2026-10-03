//! Runs the same random operations on a dense map and on the map it mirrors,
//! a `GenMap` for a `DenseGenMap` and a `SecondaryMap` for a
//! `DenseSecondaryMap`, and checks after every step that both give the same
//! results and hold the same keys and values.

use super::model::Rng;
use super::{key_from_parts, Cfg};
use crate::{
    DenseGenMap, DenseGenMapConfig, DenseSecondaryMap, DenseSecondaryMapConfig, ExistingWins,
    GenMap, GenMapConfig, GenSlotItem, InsertError, Key, KeyPiece, MapConfig, MapIdx, MapKeyConfig,
    NewerWins, Packed, SecondaryInsertError, SecondaryMap, SecondaryMapConfig, SecondarySlotItem,
    SliceStorage, Split,
};
use core::marker::PhantomData;
use std::vec::Vec;

/// The keys of [`Cfg`], with every dense storage in a `Vec`.
struct DenseCfg<Idx, Gen>(PhantomData<(Idx, Gen)>);

impl<Idx: KeyPiece, Gen: KeyPiece> MapConfig for DenseCfg<Idx, Gen> {
    type KeyConfig = Split<Idx, Gen>;
}

impl<Idx: KeyPiece, Gen: KeyPiece> DenseGenMapConfig for DenseCfg<Idx, Gen> {
    type SlotStorage<S: GenSlotItem> = Vec<S>;
    type ValueStorage<V> = Vec<V>;
    type KeyStorage<K> = Vec<K>;
}

impl<Idx: KeyPiece, Gen: KeyPiece> DenseSecondaryMapConfig for DenseCfg<Idx, Gen> {
    type ReplaceStrategy = NewerWins;
    type SlotStorage<S: GenSlotItem> = Vec<S>;
    type ValueStorage<V> = Vec<V>;
    type KeyStorage<K> = Vec<K>;
}

/// Packed keys with a 4 bit generation, in a map that wraps generations.
struct Wrap4;

impl MapConfig for Wrap4 {
    type KeyConfig = Packed<u16, 4>;
}

impl GenMapConfig for Wrap4 {
    const WRAP_ON_OVERFLOW: bool = true;
    type Storage<S: GenSlotItem> = Vec<S>;
}

/// The dense form of [`Wrap4`].
struct DenseWrap4;

impl MapConfig for DenseWrap4 {
    type KeyConfig = Packed<u16, 4>;
}

impl DenseGenMapConfig for DenseWrap4 {
    const WRAP_ON_OVERFLOW: bool = true;
    type SlotStorage<S: GenSlotItem> = Vec<S>;
    type ValueStorage<V> = Vec<V>;
    type KeyStorage<K> = Vec<K>;
}

/// `u8` keys whose secondary maps keep the existing value.
struct Keep;

impl MapConfig for Keep {
    type KeyConfig = Split<u8, u8>;
}

impl SecondaryMapConfig for Keep {
    type ReplaceStrategy = ExistingWins;
    type Storage<S: SecondarySlotItem> = Vec<S>;
}

impl DenseSecondaryMapConfig for Keep {
    type ReplaceStrategy = ExistingWins;
    type SlotStorage<S: GenSlotItem> = Vec<S>;
    type ValueStorage<V> = Vec<V>;
    type KeyStorage<K> = Vec<K>;
}

/// Twelve inline slots, values and keys, so inserts fail once the map is
/// full.
#[cfg(feature = "arrayvec")]
struct Inline;

#[cfg(feature = "arrayvec")]
impl MapConfig for Inline {
    type KeyConfig = Split<u8, u8>;
}

#[cfg(feature = "arrayvec")]
impl GenMapConfig for Inline {
    type Storage<S: GenSlotItem> = arrayvec::ArrayVec<S, 12>;
}

#[cfg(feature = "arrayvec")]
impl SecondaryMapConfig for Inline {
    type ReplaceStrategy = NewerWins;
    type Storage<S: SecondarySlotItem> = arrayvec::ArrayVec<S, 12>;
}

#[cfg(feature = "arrayvec")]
impl DenseGenMapConfig for Inline {
    type SlotStorage<S: GenSlotItem> = arrayvec::ArrayVec<S, 12>;
    type ValueStorage<V> = arrayvec::ArrayVec<V, 12>;
    type KeyStorage<K> = arrayvec::ArrayVec<K, 12>;
}

#[cfg(feature = "arrayvec")]
impl DenseSecondaryMapConfig for Inline {
    type ReplaceStrategy = NewerWins;
    type SlotStorage<S: GenSlotItem> = arrayvec::ArrayVec<S, 12>;
    type ValueStorage<V> = arrayvec::ArrayVec<V, 12>;
    type KeyStorage<K> = arrayvec::ArrayVec<K, 12>;
}

/// How many seeds to run and how many steps each one takes. Miri gets fewer
/// and shorter runs.
fn seeds() -> (u64, usize) {
    if cfg!(miri) {
        (2, 200)
    } else {
        (24, 1500)
    }
}

#[test]
fn a_dense_gen_map_agrees_with_a_gen_map() {
    let (seeds, steps) = seeds();
    for seed in 0..seeds {
        run_gen::<Cfg<u8, u8>, DenseCfg<u8, u8>>(seed, steps);
    }
}

#[test]
fn a_dense_gen_map_that_wraps_agrees_with_a_gen_map() {
    let (seeds, steps) = seeds();
    for seed in 0..seeds {
        run_gen::<Wrap4, DenseWrap4>(seed, steps);
    }
}

#[cfg(feature = "arrayvec")]
#[test]
fn a_dense_gen_map_that_fills_up_agrees_with_a_gen_map() {
    let (seeds, steps) = seeds();
    for seed in 0..seeds {
        run_gen::<Inline, Inline>(seed, steps);
    }
}

#[test]
fn a_dense_secondary_map_agrees_with_a_secondary_map() {
    let (seeds, steps) = seeds();
    for seed in 0..seeds {
        run_secondary::<Cfg<u8, u8>, DenseCfg<u8, u8>>(seed, steps);
    }
}

#[test]
fn a_dense_secondary_map_that_keeps_values_agrees_with_a_secondary_map() {
    let (seeds, steps) = seeds();
    for seed in 0..seeds {
        run_secondary::<Keep, Keep>(seed, steps);
    }
}

#[cfg(feature = "arrayvec")]
#[test]
fn a_dense_secondary_map_that_fills_up_agrees_with_a_secondary_map() {
    let (seeds, steps) = seeds();
    for seed in 0..seeds {
        run_secondary::<Inline, Inline>(seed, steps);
    }
}

/// Picks one of the keys seen so far, or `None` if there are none.
fn pick<K: Copy>(known: &[K], rng: &mut Rng) -> Option<K> {
    (!known.is_empty()).then(|| known[rng.below(known.len())])
}

/// Whether `retain` keeps a value. It only looks at the value, so both maps
/// keep the same values whatever order they visit them in.
fn keeps(value: u32) -> bool {
    value % 3 != 0
}

fn run_gen<G, D>(seed: u64, steps: usize)
where
    G: GenMapConfig,
    D: DenseGenMapConfig<KeyConfig = G::KeyConfig>,
    D::KeyStorage<Key<MapKeyConfig<D>>>: SliceStorage<Item = Key<MapKeyConfig<D>>>,
{
    assert_eq!(G::WRAP_ON_OVERFLOW, D::WRAP_ON_OVERFLOW);
    let mut rng = Rng(seed);
    let mut sparse = GenMap::<u32, G>::new_with_config();
    let mut dense = DenseGenMap::<u32, D>::new_with_config();
    // Every key either map has handed out, valid or not.
    let mut known: Vec<Key<MapKeyConfig<G>>> = Vec::new();
    for (step, value) in (0..steps).zip(1u32..) {
        let context = (seed, step);
        match rng.below(100) {
            0..=34 => match (sparse.try_insert(value), dense.try_insert(value)) {
                (Ok(a), Ok(b)) => {
                    assert_eq!(a, b, "{context:?}");
                    known.push(a);
                }
                (Err(InsertError::IndexExhausted(a)), Err(InsertError::IndexExhausted(b)))
                | (Err(InsertError::StorageFull(a, _)), Err(InsertError::StorageFull(b, _))) => {
                    assert_eq!((a, b), (value, value), "{context:?}");
                }
                _ => panic!("the maps disagree on an insert at {context:?}"),
            },
            35..=49 => {
                if let Some(key) = pick(&known, &mut rng) {
                    assert_eq!(sparse.remove(key), dense.remove(key), "{context:?}");
                }
            }
            50..=53 => {
                if let Some(key) = pick(&known, &mut rng) {
                    assert_eq!(sparse.retire(key), dense.retire(key), "{context:?}");
                }
            }
            54..=62 => {
                if let Some(key) = pick(&known, &mut rng) {
                    assert_eq!(sparse.detach(key), dense.detach(key), "{context:?}");
                }
            }
            63..=70 => {
                if let Some(key) = pick(&known, &mut rng) {
                    assert_eq!(
                        sparse.reattach(key, value),
                        dense.reattach(key, value),
                        "{context:?}"
                    );
                }
            }
            71..=74 => {
                if let Some(key) = pick(&known, &mut rng) {
                    assert_eq!(sparse.release(key), dense.release(key), "{context:?}");
                }
            }
            75..=82 => {
                if let Some(key) = pick(&known, &mut rng) {
                    if let Some(old) = sparse.get_mut(key) {
                        *old = value;
                    }
                    if let Some(old) = dense.get_mut(key) {
                        *old = value;
                    }
                }
            }
            83..=87 => {
                if let (Some(a), Some(b)) = (pick(&known, &mut rng), pick(&known, &mut rng)) {
                    match (
                        sparse.get_disjoint_mut([a, b]),
                        dense.get_disjoint_mut([a, b]),
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
            88..=91 => {
                // The dense map visits its values in another order, so it frees
                // their slots in another order too. The `GenMap` removes the
                // same keys in the same order, which keeps the free lists of
                // both maps alike.
                let mut rejected = Vec::new();
                dense.retain(|key, value| {
                    *value = value.wrapping_mul(7);
                    let keep = keeps(*value);
                    if !keep {
                        rejected.push(key);
                    }
                    keep
                });
                for (_, value) in sparse.iter_mut() {
                    *value = value.wrapping_mul(7);
                }
                for key in rejected {
                    assert!(sparse.remove(key).is_some(), "{context:?}");
                }
            }
            92..=94 => {
                // For the same reason, the `GenMap` removes the drained keys in
                // the order the dense map yields them.
                for (key, value) in dense.drain() {
                    assert_eq!(sparse.remove(key), Some(value), "{context:?}");
                }
                assert!(sparse.is_empty(), "{context:?}");
            }
            95..=97 => {
                sparse.clear();
                dense.clear();
            }
            _ => {
                sparse.reset();
                dense.reset();
                // Old keys can match new values after a reset, in both maps
                // alike, so they stay in `known`.
            }
        }
        check_gen(&sparse, &dense, &known, context);
    }
}

fn check_gen<G, D>(
    sparse: &GenMap<u32, G>,
    dense: &DenseGenMap<u32, D>,
    known: &[Key<MapKeyConfig<G>>],
    context: (u64, usize),
) where
    G: GenMapConfig,
    D: DenseGenMapConfig<KeyConfig = G::KeyConfig>,
    D::KeyStorage<Key<MapKeyConfig<D>>>: SliceStorage<Item = Key<MapKeyConfig<D>>>,
{
    assert_eq!(sparse.len(), dense.len(), "{context:?}");
    assert_eq!(sparse.slots_len(), dense.slots_len(), "{context:?}");
    for position in 0..sparse.slots_len() {
        let idx = MapIdx::<G>::from_usize(position).unwrap();
        assert_eq!(
            sparse.generation_at(idx),
            dense.generation_at(idx),
            "{context:?}"
        );
        assert_eq!(sparse.get_at(idx), dense.get_at(idx), "{context:?}");
    }
    for key in known {
        assert_eq!(sparse.get(*key), dense.get(*key), "{context:?}");
    }
    let mut expected: Vec<_> = sparse.iter().map(|(key, value)| (key, *value)).collect();
    let pairs: Vec<_> = dense.iter().map(|(key, value)| (key, *value)).collect();
    let values: Vec<_> = dense.values().copied().collect();
    let keys: Vec<_> = dense.keys().collect();
    assert_eq!(values, pairs.iter().map(|pair| pair.1).collect::<Vec<_>>());
    assert_eq!(keys, pairs.iter().map(|pair| pair.0).collect::<Vec<_>>());
    let mut actual = pairs;
    expected.sort_unstable();
    actual.sort_unstable();
    assert_eq!(actual, expected, "{context:?}");
}

/// What an insert into a secondary map returned, without the storage error,
/// so that the results of both maps can be compared.
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

fn run_secondary<S, D>(seed: u64, steps: usize)
where
    S: SecondaryMapConfig<KeyConfig = Split<u8, u8>>,
    D: DenseSecondaryMapConfig<KeyConfig = Split<u8, u8>>,
    D::KeyStorage<Key<MapKeyConfig<D>>>: SliceStorage<Item = Key<MapKeyConfig<D>>>,
{
    let mut rng = Rng(seed);
    let mut sparse = SecondaryMap::<u32, S>::new_with_config();
    let mut dense = DenseSecondaryMap::<u32, D>::new_with_config();
    let mut known: Vec<Key<Split<u8, u8>>> = Vec::new();
    for (step, value) in (0..steps).zip(1u32..) {
        let context = (seed, step);
        match rng.below(100) {
            0..=44 => {
                // The keys mostly have small indices and generations, so they
                // often meet a value under another generation. Now and then the
                // index is the reserved largest one.
                let idx = if rng.chance(2) {
                    u8::MAX
                } else {
                    rng.below(20) as u8
                };
                let generation = 1 + 2 * rng.below(5) as u8;
                let key = key_from_parts::<Split<u8, u8>>(idx, generation);
                known.push(key);
                assert_eq!(
                    Outcome::from(sparse.insert(key, value)),
                    Outcome::from(dense.insert(key, value)),
                    "{context:?}"
                );
            }
            45..=64 => {
                if let Some(key) = pick(&known, &mut rng) {
                    assert_eq!(sparse.remove(key), dense.remove(key), "{context:?}");
                }
            }
            65..=79 => {
                if let Some(key) = pick(&known, &mut rng) {
                    if let Some(old) = sparse.get_mut(key) {
                        *old = value;
                    }
                    if let Some(old) = dense.get_mut(key) {
                        *old = value;
                    }
                }
            }
            80..=86 => {
                if let (Some(a), Some(b)) = (pick(&known, &mut rng), pick(&known, &mut rng)) {
                    match (
                        sparse.get_disjoint_mut([a, b]),
                        dense.get_disjoint_mut([a, b]),
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
            87..=92 => {
                sparse.retain(|_, value| {
                    *value = value.wrapping_mul(7);
                    keeps(*value)
                });
                dense.retain(|_, value| {
                    *value = value.wrapping_mul(7);
                    keeps(*value)
                });
            }
            93..=95 => {
                let mut a: Vec<_> = sparse.drain().collect();
                let mut b: Vec<_> = dense.drain().collect();
                a.sort_unstable();
                b.sort_unstable();
                assert_eq!(a, b, "{context:?}");
            }
            _ => {
                sparse.clear();
                dense.clear();
            }
        }
        check_secondary(&sparse, &dense, &known, context);
    }
}

fn check_secondary<S, D>(
    sparse: &SecondaryMap<u32, S>,
    dense: &DenseSecondaryMap<u32, D>,
    known: &[Key<Split<u8, u8>>],
    context: (u64, usize),
) where
    S: SecondaryMapConfig<KeyConfig = Split<u8, u8>>,
    D: DenseSecondaryMapConfig<KeyConfig = Split<u8, u8>>,
    D::KeyStorage<Key<MapKeyConfig<D>>>: SliceStorage<Item = Key<MapKeyConfig<D>>>,
{
    assert_eq!(sparse.len(), dense.len(), "{context:?}");
    assert_eq!(sparse.slots_len(), dense.slots_len(), "{context:?}");
    for position in 0..sparse.slots_len() {
        let idx = position as u8;
        assert_eq!(
            sparse.generation_at(idx),
            dense.generation_at(idx),
            "{context:?}"
        );
        assert_eq!(sparse.get_at(idx), dense.get_at(idx), "{context:?}");
    }
    for key in known {
        assert_eq!(sparse.get(*key), dense.get(*key), "{context:?}");
    }
    let mut expected: Vec<_> = sparse.iter().map(|(key, value)| (key, *value)).collect();
    let mut actual: Vec<_> = dense.iter().map(|(key, value)| (key, *value)).collect();
    expected.sort_unstable();
    actual.sort_unstable();
    assert_eq!(actual, expected, "{context:?}");
}
