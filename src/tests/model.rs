//! Runs long random sequences of operations on a map and checks it against
//! a simple model after every step. The model only tracks which keys are
//! valid and what they hold, so it knows nothing about slots or the free
//! list. Each config gets its own test so they run in parallel.

use crate::{
    Config, DefaultConfig, GenMap, GetDisjointMutAtError, GetDisjointMutError, InsertError,
    InsertWithError, Key, KeyLayout, KeyPiece, Packed,
};
use std::vec::Vec;

/// 4096 slots that each retire after eight uses, so slots retire often but
/// the map never runs out of them.
struct Retiring;

impl Config for Retiring {
    type Idx = u16;
    type Gen = u8;
    type Layout = Packed<u16, 4>;
    type Storage<S> = Vec<S>;
}

/// Sixteen slots that each retire after eight uses, so the index and the
/// generations both run out often.
struct Small;

impl Config for Small {
    type Idx = u8;
    type Gen = u8;
    type Layout = Packed<u8, 4>;
    type Storage<S> = Vec<S>;
}

/// The same as [`Small`], but a slot wraps instead of retiring.
struct SmallWrap;

impl Config for SmallWrap {
    type Idx = u8;
    type Gen = u8;
    type Layout = Packed<u8, 4>;
    type Storage<S> = Vec<S>;
    const WRAP_ON_OVERFLOW: bool = true;
}

/// The same keys as [`Small`], but with the slots in an `ArrayVec` of
/// twelve, so the storage runs out before the index does.
#[cfg(feature = "arrayvec")]
struct InlineRetiring;

#[cfg(feature = "arrayvec")]
impl Config for InlineRetiring {
    type Idx = u8;
    type Gen = u8;
    type Layout = Packed<u8, 4>;
    type Storage<S> = arrayvec::ArrayVec<S, 12>;
}

/// The same keys as [`SmallWrap`], but with the slots in an `ArrayVec` of
/// sixteen, so the storage and the index run out together.
#[cfg(feature = "arrayvec")]
struct InlineWrap;

#[cfg(feature = "arrayvec")]
impl Config for InlineWrap {
    type Idx = u8;
    type Gen = u8;
    type Layout = Packed<u8, 4>;
    type Storage<S> = arrayvec::ArrayVec<S, 16>;
    const WRAP_ON_OVERFLOW: bool = true;
}

/// The same keys as [`Retiring`], but with the slots in a `SmallVec` that
/// holds four of them inline, so the storage moves to the heap once the map
/// grows past four slots.
#[cfg(feature = "smallvec")]
struct Spilling;

#[cfg(feature = "smallvec")]
impl Config for Spilling {
    type Idx = u16;
    type Gen = u8;
    type Layout = Packed<u16, 4>;
    type Storage<S> = smallvec::SmallVec<S, 4>;
}

#[test]
fn the_default_config_agrees_with_the_model() {
    run_seeds::<DefaultConfig>();
}

#[test]
fn a_packed_config_that_retires_agrees_with_the_model() {
    run_seeds::<Retiring>();
}

#[test]
fn a_small_packed_config_agrees_with_the_model() {
    run_seeds::<Small>();
}

#[test]
fn a_small_packed_config_that_wraps_agrees_with_the_model() {
    run_seeds::<SmallWrap>();
}

#[cfg(feature = "arrayvec")]
#[test]
fn an_array_vec_that_fills_up_agrees_with_the_model() {
    run_seeds::<InlineRetiring>();
}

#[cfg(feature = "arrayvec")]
#[test]
fn an_array_vec_as_large_as_the_index_agrees_with_the_model() {
    run_seeds::<InlineWrap>();
}

#[cfg(feature = "smallvec")]
#[test]
fn a_small_vec_agrees_with_the_model() {
    run_seeds::<Spilling>();
}

/// Runs every seed for `C`. Miri is far slower than a normal run, so it gets
/// fewer and shorter runs.
fn run_seeds<C: Config>() {
    let (seeds, steps) = if cfg!(miri) { (2, 300) } else { (48, 2000) };
    for seed in 0..seeds {
        run::<C>(seed, steps);
    }
}

/// A small deterministic random number generator (SplitMix64), so a failing
/// seed replays the same way every time.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Returns a random number below `n`. It panics if `n` is zero.
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    /// Returns `true` with a chance of `percent` in a hundred.
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

/// What the map should hold after the operations so far.
struct Model<C: Config> {
    /// Every valid key and the value under it.
    live: Vec<(Key<C>, u32)>,
    /// Keys whose value was detached and not reattached yet.
    detached: Vec<Key<C>>,
    /// Keys whose value was removed. None of them may match anything, and a
    /// config that retires slots never hands one out again. A config that
    /// wraps can hand one out again, and that key then moves back to
    /// `live`.
    dead: Vec<Key<C>>,
    /// The value the next insert uses. Every value is different, so a value
    /// that shows up under the wrong key is caught.
    next_value: u32,
}

impl<C: Config> Model<C> {
    fn fresh_value(&mut self) -> u32 {
        self.next_value += 1;
        self.next_value
    }

    /// Moves every valid key to `dead`, which is what `clear` and `drain` do.
    fn kill_all(&mut self) {
        self.dead.extend(self.live.drain(..).map(|(key, _)| key));
    }
}

/// Prints the seed and the step when a check fails, so the failure can be
/// replayed.
struct Context {
    seed: u64,
    step: usize,
}

impl Drop for Context {
    fn drop(&mut self) {
        if std::thread::panicking() {
            std::eprintln!(
                "the model check failed at seed {} step {}",
                self.seed,
                self.step
            );
        }
    }
}

fn run<C: Config>(seed: u64, steps: usize) {
    let mut rng = Rng(seed);
    let mut map = GenMap::<u32, C>::new_with_config();
    let mut model = Model::<C> {
        live: Vec::new(),
        detached: Vec::new(),
        dead: Vec::new(),
        next_value: 0,
    };
    let mut context = Context { seed, step: 0 };

    for step in 0..steps {
        context.step = step;
        // The weights are out of a thousand.
        match rng.below(1000) {
            0..=299 => insert(&mut map, &mut model, &mut rng),
            300..=449 => remove(&mut map, &mut model, &mut rng),
            450..=499 => look_up_invalid(&mut map, &mut model, &mut rng),
            500..=599 => overwrite(&mut map, &mut model, &mut rng),
            600..=669 => detach(&mut map, &mut model, &mut rng),
            670..=739 => reattach(&mut map, &mut model, &mut rng),
            740..=769 => retain(&mut map, &mut model, &mut rng),
            770..=819 => get_disjoint(&mut map, &mut model, &mut rng),
            820..=879 => look_up_by_index(&map, &model, &mut rng),
            880..=909 => compare_clones(&map),
            910..=949 => drain(&mut map, &mut model, &mut rng),
            950..=989 => {
                map.clear();
                model.kill_all();
            }
            _ => {
                // Old keys may match new values after a reset, so the model
                // stops tracking all of them.
                map.reset();
                model.live.clear();
                model.detached.clear();
                model.dead.clear();
            }
        }
        check(&map, &model, &mut rng);
    }

    for key in &model.dead {
        assert!(map.get(*key).is_none());
    }
}

fn largest_generation<C: Config>() -> C::Gen {
    <C::Layout as KeyLayout<C::Idx, C::Gen>>::max_generation()
}

fn slot_count_limit<C: Config>() -> Option<usize> {
    <C::Layout as KeyLayout<C::Idx, C::Gen>>::max_idx()
        .into_usize()?
        .checked_add(1)
}

fn insert<C: Config>(map: &mut GenMap<u32, C>, model: &mut Model<C>, rng: &mut Rng) {
    // Dropping the entry leaves the map as it was, so the insert below must
    // hand out the key it promised.
    let promised = map.vacant_entry().ok().map(|entry| entry.key());

    if rng.chance(10) {
        match map.try_insert_with_key(Err::<u32, _>) {
            Err(InsertWithError::Rejected(key)) => assert_eq!(Some(key), promised),
            Err(InsertWithError::Full(_)) => assert!(promised.is_none()),
            Ok(_) => unreachable!("the closure always fails"),
        }
    }

    let value = model.fresh_value();
    match map.try_insert(value) {
        Ok(key) => {
            assert_eq!(Some(key), promised);
            assert!(model.live.iter().all(|(live, _)| *live != key));
            assert!(!model.detached.contains(&key));
            if C::WRAP_ON_OVERFLOW {
                model.dead.retain(|dead| *dead != key);
            } else {
                assert!(!model.dead.contains(&key));
            }
            model.live.push((key, value));
        }
        Err(InsertError::IndexExhausted(back)) => {
            assert_eq!(back, value);
            assert!(promised.is_none());
            // Every slot the keys can address exists.
            assert_eq!(Some(map.slots_len()), slot_count_limit::<C>());
            assert_no_slot_is_free(map, model);
        }
        Err(InsertError::StorageFull(back, _)) => {
            assert_eq!(back, value);
            assert!(promised.is_none());
            // The storage holds as many slots as it can. The index has not
            // run out as well, because when both run out, the map reports
            // the index.
            assert_eq!(map.slots_len(), map.capacity());
            assert!(slot_count_limit::<C>().map_or(true, |limit| map.slots_len() < limit));
            assert_no_slot_is_free(map, model);
        }
    }
}

/// Checks that every slot holds a value, is detached or is retired, so none
/// of them can take a new value.
fn assert_no_slot_is_free<C: Config>(map: &GenMap<u32, C>, model: &Model<C>) {
    let retired = if C::WRAP_ON_OVERFLOW {
        0
    } else {
        (0..map.slots_len())
            .filter(|&position| {
                let idx = C::Idx::from_usize(position).unwrap();
                map.generation_at(idx) == Some(C::Gen::ZERO)
            })
            .count()
    };
    assert_eq!(
        model.live.len() + model.detached.len() + retired,
        map.slots_len()
    );
}

fn remove<C: Config>(map: &mut GenMap<u32, C>, model: &mut Model<C>, rng: &mut Rng) {
    if model.live.is_empty() {
        return;
    }
    let (key, value) = model.live.swap_remove(rng.below(model.live.len()));
    assert_eq!(map.remove(key), Some(value));
    model.dead.push(key);
}

/// Tries every kind of lookup with a removed or detached key, none of which
/// may find anything.
fn look_up_invalid<C: Config>(map: &mut GenMap<u32, C>, model: &mut Model<C>, rng: &mut Rng) {
    let count = model.dead.len() + model.detached.len();
    if count == 0 {
        return;
    }
    let i = rng.below(count);
    let key = match model.dead.get(i) {
        Some(key) => *key,
        None => model.detached[i - model.dead.len()],
    };
    assert!(map.get(key).is_none());
    assert!(map.get_mut(key).is_none());
    assert!(!map.contains_key(key));
    assert!(map.remove(key).is_none());
    assert!(map.detach(key).is_none());
}

fn overwrite<C: Config>(map: &mut GenMap<u32, C>, model: &mut Model<C>, rng: &mut Rng) {
    if model.live.is_empty() {
        return;
    }
    let i = rng.below(model.live.len());
    let value = model.fresh_value();
    let (key, old) = model.live[i];
    assert_eq!(map[key], old);
    map[key] = value;
    model.live[i].1 = value;
}

fn detach<C: Config>(map: &mut GenMap<u32, C>, model: &mut Model<C>, rng: &mut Rng) {
    if model.live.is_empty() {
        return;
    }
    let (key, value) = model.live.swap_remove(rng.below(model.live.len()));
    match map.detach(key) {
        Some(taken) => {
            assert_eq!(taken, value);
            model.detached.push(key);
        }
        None => {
            // `detach` only refuses a slot whose generation is already the
            // largest one, and the value then stays in the map.
            assert_eq!(key.generation(), largest_generation::<C>());
            assert_eq!(map.get(key), Some(&value));
            model.live.push((key, value));
        }
    }
}

fn reattach<C: Config>(map: &mut GenMap<u32, C>, model: &mut Model<C>, rng: &mut Rng) {
    if model.detached.is_empty() {
        return;
    }
    let key = model.detached.swap_remove(rng.below(model.detached.len()));
    let value = model.fresh_value();
    map.reattach(key, value);
    model.live.push((key, value));
}

/// Keeps a random part of the values and changes the ones it keeps. It also
/// checks that `retain` visits every value once, in slot order.
fn retain<C: Config>(map: &mut GenMap<u32, C>, model: &mut Model<C>, rng: &mut Rng) {
    let mut visited = Vec::new();
    map.retain(|key, value| {
        let keep = rng.chance(70);
        visited.push((key, *value, keep));
        *value += 1_000_000;
        keep
    });

    let mut expected = model.live.clone();
    expected.sort();
    let seen: Vec<_> = visited
        .iter()
        .map(|&(key, value, _)| (key, value))
        .collect();
    assert_eq!(seen, expected);

    model.live.clear();
    for (key, value, keep) in visited {
        if keep {
            model.live.push((key, value + 1_000_000));
        } else {
            model.dead.push(key);
        }
    }
}

fn get_disjoint<C: Config>(map: &mut GenMap<u32, C>, model: &mut Model<C>, rng: &mut Rng) {
    if let Some(key) = model.dead.first() {
        let first = model.live.first().map_or(*key, |(live, _)| *live);
        assert_eq!(
            map.get_disjoint_mut([first, *key]),
            Err(GetDisjointMutError::InvalidKey)
        );
    }
    if model.live.len() < 2 {
        return;
    }
    let i = rng.below(model.live.len());
    let j = (i + 1 + rng.below(model.live.len() - 1)) % model.live.len();
    let (a, value_a) = model.live[i];
    let (b, value_b) = model.live[j];

    assert_eq!(
        map.get_disjoint_mut([a, a]),
        Err(GetDisjointMutError::OverlappingKeys)
    );
    assert_eq!(
        map.get_disjoint_mut_at([a.idx(), a.idx()]),
        Err(GetDisjointMutAtError::OverlappingIndices)
    );

    let [x, y] = map.get_disjoint_mut([a, b]).unwrap();
    assert_eq!((*x, *y), (value_a, value_b));
    core::mem::swap(x, y);

    let [(key_b, y), (key_a, x)] = map.get_disjoint_mut_at([b.idx(), a.idx()]).unwrap();
    assert_eq!((key_a, key_b), (a, b));
    assert_eq!((*x, *y), (value_b, value_a));
    model.live[i].1 = value_b;
    model.live[j].1 = value_a;
}

/// Checks the lookups by index against the model at a random index, which
/// may be past the last slot.
fn look_up_by_index<C: Config>(map: &GenMap<u32, C>, model: &Model<C>, rng: &mut Rng) {
    let position = rng.below(map.slots_len() + 1);
    let Some(idx) = C::Idx::from_usize(position) else {
        return;
    };
    let expected = model.live.iter().find(|(key, _)| key.idx() == idx);
    assert_eq!(map.key_at(idx), expected.map(|(key, _)| *key));
    assert_eq!(map.get_at(idx), expected.map(|(key, value)| (*key, value)));
    assert_eq!(map.generation_at(idx).is_some(), position < map.slots_len());
    if let Some((key, _)) = expected {
        assert_eq!(map.generation_at(idx), Some(key.generation()));
    }
}

/// A clone, and a map that took the contents with `clone_from`, must hold
/// the same values and hand out the same key next.
fn compare_clones<C: Config>(map: &GenMap<u32, C>) {
    let mut copy = map.clone();
    let mut target = GenMap::<u32, C>::new_with_config();
    target.insert(0);
    target.clone_from(map);

    let next = map.clone().vacant_entry().ok().map(|entry| entry.key());
    for other in [&mut copy, &mut target] {
        assert!(other.iter().eq(map.iter()));
        assert_eq!(other.slots_len(), map.slots_len());
        assert_eq!(other.vacant_entry().ok().map(|entry| entry.key()), next);
    }
}

/// Takes a random number of values out of a drain, then drops the drain,
/// which removes the rest.
fn drain<C: Config>(map: &mut GenMap<u32, C>, model: &mut Model<C>, rng: &mut Rng) {
    let mut expected = model.live.clone();
    expected.sort();
    let take = rng.below(expected.len() + 1);

    let mut drain = map.drain();
    assert_eq!(drain.len(), expected.len());
    let taken: Vec<_> = drain.by_ref().take(take).collect();
    drop(drain);
    assert_eq!(taken, expected[..take]);
    assert!(map.is_empty());
    model.kill_all();
}

/// Checks the whole map against the model.
fn check<C: Config>(map: &GenMap<u32, C>, model: &Model<C>, rng: &mut Rng) {
    assert_eq!(map.len(), model.live.len());
    assert_eq!(map.is_empty(), model.live.is_empty());

    // Iteration is in slot order, and a slot holds at most one valid key, so
    // slot order is also key order.
    let mut expected = model.live.clone();
    expected.sort();
    let actual: Vec<_> = map.iter().map(|(key, value)| (key, *value)).collect();
    assert_eq!(actual, expected);
    assert_eq!(map.iter().len(), expected.len());

    for (key, value) in &model.live {
        assert_eq!(map.get(*key), Some(value));
        assert_eq!(map.key_at(key.idx()), Some(*key));
    }
    for key in &model.detached {
        assert!(map.get(*key).is_none());
        assert!(map.key_at(key.idx()).is_none());
    }
    // Checking every removed key on every step would be slow, so a few are
    // picked at random, and all of them are checked at the end of the run.
    for _ in 0..model.dead.len().min(8) {
        let key = model.dead[rng.below(model.dead.len())];
        assert!(map.get(key).is_none());
    }
}
