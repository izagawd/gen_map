//! Runs random operations on all four maps with values whose drops and clones
//! sometimes panic, and with `retain` closures that sometimes panic. After
//! every step, each map must still find every value under its key. At the end,
//! every value must have been dropped exactly once.

use super::model::Rng;
use super::{DropItem, DropTracker};
use crate::{
    DefaultMapConfig, DenseGenMap, DenseGenMapConfig, DenseSecondaryMap, DenseSecondaryMapConfig,
    GenMap, GenMapConfig, GenSlotItem, Key, KeyPiece, MapConfig, MapIdx, MapKeyConfig, NewerWins,
    Packed, SecondaryMap, SecondaryMapConfig, SecondarySlotItem,
};
use std::cell::{Cell, RefCell};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;
use std::vec::Vec;

/// Decides when a [`Fragile`] panics. It only does so while `percent` is above
/// zero, which the test only sets while an operation runs.
struct Chaos {
    rng: RefCell<Rng>,
    percent: Cell<u64>,
}

impl Chaos {
    fn strikes(&self) -> bool {
        let percent = self.percent.get();
        percent > 0 && self.rng.borrow_mut().chance(percent)
    }
}

/// A value whose drop or clone panics when its [`Chaos`] strikes. It holds a
/// [`DropItem`], which is still dropped when the drop panics, so the tracker
/// sees every value dropped exactly once either way.
struct Fragile {
    item: DropItem,
    chaos: Rc<Chaos>,
}

impl Clone for Fragile {
    fn clone(&self) -> Self {
        if self.chaos.strikes() {
            panic!("clone panic");
        }
        Fragile {
            item: self.item.clone(),
            chaos: self.chaos.clone(),
        }
    }
}

impl Drop for Fragile {
    fn drop(&mut self) {
        // A second panic while the thread unwinds would abort the tests.
        if !std::thread::panicking() && self.chaos.strikes() {
            panic!("drop panic");
        }
    }
}

/// Implements all four config traits for a config with the given key config,
/// overflow rule, and slot, value and key storages.
macro_rules! config {
    ($(#[$doc:meta])* $name:ident, $key:ty, $wrap:expr, $slots:ident, $values:ident, $keys:ident) => {
        $(#[$doc])*
        struct $name;

        impl MapConfig for $name {
            type KeyConfig = $key;
        }

        impl GenMapConfig for $name {
            const WRAP_ON_OVERFLOW: bool = $wrap;
            type Storage<S: GenSlotItem> = $slots<S>;
        }

        impl SecondaryMapConfig for $name {
            type ReplaceStrategy = NewerWins;
            type Storage<S: SecondarySlotItem> = $slots<S>;
        }

        impl DenseGenMapConfig for $name {
            const WRAP_ON_OVERFLOW: bool = $wrap;
            type SlotStorage<S: GenSlotItem> = $slots<S>;
            type ValueStorage<V> = $values<V>;
            type KeyStorage<K> = $keys<K>;
        }

        impl DenseSecondaryMapConfig for $name {
            type ReplaceStrategy = NewerWins;
            type SlotStorage<S: GenSlotItem> = $slots<S>;
            type ValueStorage<V> = $values<V>;
            type KeyStorage<K> = $keys<K>;
        }
    };
}

config!(
    /// Packed keys with a 3 bit generation, in maps that wrap generations.
    Wrapping,
    Packed<u16, 3>,
    true,
    Vec,
    Vec,
    Vec
);

config!(
    /// Packed keys with 5 index bits and a 3 bit generation. The keys run out
    /// of indices once a map has 32 slots, and a slot retires once its fourth
    /// value is removed.
    Retiring,
    Packed<u8, 3>,
    false,
    Vec,
    Vec,
    Vec
);

#[cfg(feature = "arrayvec")]
type Eight<T> = arrayvec::ArrayVec<T, 8>;

#[cfg(feature = "arrayvec")]
type Six<T> = arrayvec::ArrayVec<T, 6>;

#[cfg(feature = "arrayvec")]
config!(
    /// Inline storages with room for eight slots and keys but only six values,
    /// so the dense maps run out of room for values first.
    Inline,
    crate::Split<u8, u8>,
    false,
    Eight,
    Six,
    Eight
);

/// How many seeds to run and how many steps each one takes. Miri gets fewer
/// and shorter runs.
fn seeds() -> (u64, usize) {
    if cfg!(miri) {
        (1, 60)
    } else {
        (16, 1000)
    }
}

#[test]
fn the_default_config_survives_panics() {
    let (seeds, steps) = seeds();
    for seed in 0..seeds {
        run::<DefaultMapConfig>(seed, steps);
    }
}

#[test]
fn maps_that_wrap_survive_panics() {
    let (seeds, steps) = seeds();
    for seed in 0..seeds {
        run::<Wrapping>(seed, steps);
    }
}

#[test]
fn maps_whose_keys_run_out_survive_panics() {
    let (seeds, steps) = seeds();
    for seed in 0..seeds {
        run::<Retiring>(seed, steps);
    }
}

#[cfg(feature = "arrayvec")]
#[test]
fn maps_that_fill_up_survive_panics() {
    let (seeds, steps) = seeds();
    for seed in 0..seeds {
        run::<Inline>(seed, steps);
    }
}

/// Picks one of the keys seen so far, or `None` if there are none.
fn pick<K: Copy>(known: &[K], rng: &mut Rng) -> Option<K> {
    (!known.is_empty()).then(|| known[rng.below(known.len())])
}

/// Returns a `retain` closure that keeps every other value. With `panics`, it
/// panics on its third call.
fn keeper<K>(panics: bool) -> impl FnMut(K, &mut Fragile) -> bool {
    let mut calls = 0;
    move |_, _| {
        calls += 1;
        if panics && calls == 3 {
            panic!("retain panic");
        }
        calls % 2 == 0
    }
}

/// Takes `taken` items out of a drain, then forgets the drain if `taken` is 3
/// and drops it otherwise.
fn finish<I: Iterator>(mut drain: I, taken: usize) {
    for _ in 0..taken {
        drop(drain.next());
    }
    if taken == 3 {
        core::mem::forget(drain);
    } else {
        drop(drain);
    }
}

fn run<C>(seed: u64, steps: usize)
where
    C: GenMapConfig + SecondaryMapConfig + DenseGenMapConfig + DenseSecondaryMapConfig,
{
    let tracker = DropTracker::new();
    let chaos = Rc::new(Chaos {
        rng: RefCell::new(Rng(seed ^ 0x5EED)),
        percent: Cell::new(0),
    });
    let fragile = || Fragile {
        item: tracker.make_item(),
        chaos: chaos.clone(),
    };
    let mut rng = Rng(seed);
    let mut sparse = GenMap::<Fragile, C>::new_with_config();
    let mut dense = DenseGenMap::<Fragile, C>::new_with_config();
    let mut secondary = SecondaryMap::<Fragile, C>::new_with_config();
    let mut dense_secondary = DenseSecondaryMap::<Fragile, C>::new_with_config();
    // Every key either gen map has handed out, valid or not.
    let mut known: Vec<Key<MapKeyConfig<C>>> = Vec::new();
    for _ in 0..steps {
        let operation = rng.below(16);
        let (a, b) = (pick(&known, &mut rng), pick(&known, &mut rng));
        let retain_panics = rng.chance(20);
        let taken = rng.below(4);
        chaos.percent.set(10);
        // The clones are returned from the closure and checked outside it,
        // because `catch_unwind` would also catch a failed check.
        let clones = catch_unwind(AssertUnwindSafe(|| {
            match operation {
                0..=3 => {
                    if let Ok(key) = sparse.try_insert(fragile()) {
                        known.push(key);
                    }
                    if let Ok(key) = dense.try_insert(fragile()) {
                        known.push(key);
                    }
                    if let Some(key) = a {
                        drop(secondary.insert(key, fragile()));
                    }
                    if let Some(key) = b {
                        drop(dense_secondary.insert(key, fragile()));
                    }
                }
                4 => {
                    if let Some(key) = a {
                        drop(sparse.remove(key));
                        drop(dense.remove(key));
                        drop(secondary.remove(key));
                        drop(dense_secondary.remove(key));
                    }
                }
                5 => {
                    if let Some(key) = a {
                        drop(sparse.retire(key));
                        drop(dense.retire(key));
                    }
                }
                6 => {
                    if let Some(key) = a {
                        drop(sparse.detach(key));
                        drop(dense.detach(key));
                    }
                }
                7 => {
                    if let Some(key) = a {
                        drop(sparse.reattach(key, fragile()));
                        drop(dense.reattach(key, fragile()));
                    }
                }
                8 => {
                    if let Some(key) = a {
                        sparse.release(key);
                        dense.release(key);
                    }
                }
                9 => {
                    sparse.clear();
                    dense.clear();
                    secondary.clear();
                    dense_secondary.clear();
                }
                10 => {
                    sparse.reset();
                    dense.reset();
                }
                11 => {
                    sparse.retain(keeper(retain_panics));
                    dense.retain(keeper(retain_panics));
                    secondary.retain(keeper(retain_panics));
                    dense_secondary.retain(keeper(retain_panics));
                }
                12 => {
                    finish(sparse.drain(), taken);
                    finish(dense.drain(), taken);
                    finish(secondary.drain(), taken);
                    finish(dense_secondary.drain(), taken);
                }
                13 => {
                    return Some((
                        sparse.clone(),
                        dense.clone(),
                        secondary.clone(),
                        dense_secondary.clone(),
                    ));
                }
                14 => {
                    if let (Some(x), Some(y)) = (a, b) {
                        if let Ok([p, q]) = sparse.get_disjoint_mut([x, y]) {
                            core::mem::swap(p, q);
                        }
                        if let Ok([p, q]) = dense.get_disjoint_mut([x, y]) {
                            core::mem::swap(p, q);
                        }
                        if let Ok([p, q]) = secondary.get_disjoint_mut([x, y]) {
                            core::mem::swap(p, q);
                        }
                        if let Ok([p, q]) = dense_secondary.get_disjoint_mut([x, y]) {
                            core::mem::swap(p, q);
                        }
                    }
                }
                _ => {
                    // Each assignment drops the old value in place.
                    if let Some(key) = a {
                        if let Some(value) = sparse.get_mut(key) {
                            *value = fragile();
                        }
                        if let Some(value) = dense.get_mut(key) {
                            *value = fragile();
                        }
                        if let Some(value) = secondary.get_mut(key) {
                            *value = fragile();
                        }
                        if let Some(value) = dense_secondary.get_mut(key) {
                            *value = fragile();
                        }
                    }
                }
            }
            None
        }));
        chaos.percent.set(0);
        if let Ok(Some((sparse, dense, secondary, dense_secondary))) = clones {
            check(&sparse, &dense, &secondary, &dense_secondary);
        }
        check(&sparse, &dense, &secondary, &dense_secondary);
    }
    drop((sparse, dense, secondary, dense_secondary));
    tracker.assert_all_dropped_exactly_once(tracker.total_made());
}

/// Checks that every map finds each of its values under that value's key. A
/// dense map must also keep each key next to its value, and exactly one of its
/// slots must hold each value.
fn check<C>(
    sparse: &GenMap<Fragile, C>,
    dense: &DenseGenMap<Fragile, C>,
    secondary: &SecondaryMap<Fragile, C>,
    dense_secondary: &DenseSecondaryMap<Fragile, C>,
) where
    C: GenMapConfig + SecondaryMapConfig + DenseGenMapConfig + DenseSecondaryMapConfig,
{
    assert_eq!(sparse.iter().count(), sparse.len());
    for (key, value) in sparse {
        assert!(core::ptr::eq(sparse.get(key).unwrap(), value));
    }
    assert_eq!(secondary.iter().count(), secondary.len());
    for (key, value) in secondary {
        assert!(core::ptr::eq(secondary.get(key).unwrap(), value));
    }
    macro_rules! check_dense {
        ($map:expr) => {{
            let map = $map;
            assert_eq!(map.keys().len(), map.len());
            assert_eq!(map.values().len(), map.len());
            for (key, value) in map.keys().zip(map.values()) {
                assert!(core::ptr::eq(map.get(key).unwrap(), value));
                assert_eq!(map.key_at(key.idx()), Some(key));
            }
            let holding = (0..map.slots_len())
                .filter(|&position| {
                    map.key_at(MapIdx::<C>::from_usize(position).unwrap())
                        .is_some()
                })
                .count();
            assert_eq!(holding, map.len());
        }};
    }
    check_dense!(dense);
    check_dense!(dense_secondary);
}
