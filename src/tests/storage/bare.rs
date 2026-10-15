//! Storages that implement `SliceStorage` and little else, to check that a
//! map only asks for more where a method needs it.

use crate::{
    DenseGenMap, DenseGenMapConfig, DenseSecondaryMap, DenseSecondaryMapConfig, GenMap,
    GenMapConfig, GenSlotItem, KeepLen, MapConfig, NewerWins, SliceStorage, Split, SplitPair,
};
use std::collections::TryReserveError;
use std::vec::Vec;

/// A `Vec` behind `SliceStorage` alone. With `ITER` it also has an owning
/// iterator, which implements `Iterator` but not `DoubleEndedIterator` or
/// `ExactSizeIterator`.
struct Bare<S, const ITER: bool>(Vec<S>);

// SAFETY: every method forwards to the `Vec`, which behaves as the trait
// describes. `ensure_room` makes room with `try_reserve`, so `push_unchecked`
// never grows the `Vec` or panics while the rules promise room.
unsafe impl<S, const ITER: bool> SliceStorage for Bare<S, ITER> {
    type Item = S;
    type Error = TryReserveError;

    fn capacity(&self) -> usize {
        self.0.capacity()
    }

    fn as_slice(&self) -> &[S] {
        &self.0
    }

    fn as_mut_slice(&mut self) -> &mut [S] {
        &mut self.0
    }

    fn ensure_room(&mut self, additional: usize) -> Result<(), TryReserveError> {
        self.0.try_reserve(additional)
    }

    unsafe fn push_unchecked(&mut self, item: S) {
        debug_assert!(self.0.len() < self.0.capacity());
        self.0.push(item);
    }

    fn pop(&mut self) -> Option<S> {
        self.0.pop()
    }

    fn clear(&mut self) {
        self.0.clear();
    }
}

/// A `Vec` iterator that only implements `Iterator`, so it has no
/// `next_back` and no `len`. Its size hint is exact, as `SliceStorage`
/// requires.
struct Forwards<S>(std::vec::IntoIter<S>);

impl<S> Iterator for Forwards<S> {
    type Item = S;

    fn next(&mut self) -> Option<S> {
        self.0.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<S> IntoIterator for Bare<S, true> {
    type Item = S;
    type IntoIter = Forwards<S>;

    fn into_iter(self) -> Forwards<S> {
        Forwards(self.0.into_iter())
    }
}

/// Returns a `Bare` with no items. `Bare` does not implement `Empty`, so the
/// tests build their maps by passing empty `Bare`s to `with_storage` and
/// `with_storages`.
fn bare<S, const ITER: bool>() -> Bare<S, ITER> {
    Bare(Vec::new())
}

/// Returns a `SplitPair` of two `Bare`s with no items.
fn bare_pairs<A, B, const ITER: bool>() -> SplitPair<Bare<A, ITER>, Bare<B, ITER>> {
    SplitPair::from_parts(bare(), bare()).unwrap()
}

/// Its storage has no owning iterator.
struct NoIter;

impl MapConfig for NoIter {
    type KeyConfig = Split<u32, u32>;
}

impl GenMapConfig for NoIter {
    type Len = KeepLen;
    type Storage<S: GenSlotItem> = Bare<S, false>;
}

/// Its storage's owning iterator is a `Forwards`.
struct ForwardIter;

impl MapConfig for ForwardIter {
    type KeyConfig = Split<u32, u32>;
}

impl GenMapConfig for ForwardIter {
    type Len = KeepLen;
    type Storage<S: GenSlotItem> = Bare<S, true>;
}

/// Its dense storages have no owning iterator.
struct DenseNoIter;

impl MapConfig for DenseNoIter {
    type KeyConfig = Split<u32, u32>;
}

impl DenseGenMapConfig for DenseNoIter {
    type SlotStorage<S: GenSlotItem> = Bare<S, false>;
    type PairStorage<K, V> = SplitPair<Bare<K, false>, Bare<V, false>>;
}

impl DenseSecondaryMapConfig for DenseNoIter {
    type ReplaceStrategy = NewerWins;
    type SlotStorage<S: GenSlotItem> = Bare<S, false>;
    type PairStorage<K, V> = SplitPair<Bare<K, false>, Bare<V, false>>;
}

/// The owning iterators of its dense storages are `Forwards`.
struct DenseForwardIter;

impl MapConfig for DenseForwardIter {
    type KeyConfig = Split<u32, u32>;
}

impl DenseGenMapConfig for DenseForwardIter {
    type SlotStorage<S: GenSlotItem> = Bare<S, true>;
    type PairStorage<K, V> = SplitPair<Bare<K, true>, Bare<V, true>>;
}

impl DenseSecondaryMapConfig for DenseForwardIter {
    type ReplaceStrategy = NewerWins;
    type SlotStorage<S: GenSlotItem> = Bare<S, true>;
    type PairStorage<K, V> = SplitPair<Bare<K, true>, Bare<V, true>>;
}

#[test]
fn a_map_works_without_an_owning_iterator() {
    let mut map = GenMap::<u32, NoIter>::with_storage(bare()).unwrap();
    let a = map.insert(1);
    let b = map.insert(2);
    assert_eq!(map.remove(a), Some(1));
    let c = map.insert(3);
    assert_eq!(c.idx(), a.idx());

    // The borrowing iterators do not need anything from the storage.
    let backwards: Vec<_> = map.iter().rev().map(|(_, value)| *value).collect();
    assert_eq!(backwards, [2, 3]);
    let drained: Vec<_> = map.drain().collect();
    assert_eq!(drained, [(c, 3), (b, 2)]);
}

#[test]
fn a_dense_map_works_without_an_owning_iterator() {
    let mut map = DenseGenMap::<u32, DenseNoIter>::with_storages(bare(), bare_pairs()).unwrap();
    let a = map.insert(1);
    let b = map.insert(2);
    assert_eq!(map.remove(a), Some(1));
    let c = map.insert(3);
    assert_eq!(c.idx(), a.idx());

    // The borrowing iterators do not need anything from the storages.
    let backwards: Vec<_> = map.iter().rev().map(|(_, value)| *value).collect();
    assert_eq!(backwards, [3, 2]);
    let drained: Vec<_> = map.drain().collect();
    assert_eq!(drained, [(c, 3), (b, 2)]);

    let mut secondary =
        DenseSecondaryMap::<u32, DenseNoIter>::with_storages(bare(), bare_pairs()).unwrap();
    secondary.insert(c, 30).unwrap();
    secondary.insert(b, 20).unwrap();
    let drained: Vec<_> = secondary.drain().collect();
    assert_eq!(drained, [(b, 20), (c, 30)]);
}

#[test]
fn dense_into_iter_works_when_the_storage_iterators_have_no_next_back() {
    let mut map =
        DenseGenMap::<u32, DenseForwardIter>::with_storages(bare(), bare_pairs()).unwrap();
    let keys: Vec<_> = (0..4).map(|i| map.insert(i)).collect();
    map.remove(keys[1]);

    let mut iter = map.into_iter();
    // The storage's iterator does not implement `ExactSizeIterator`, but its
    // size hint is exact, so `iter` still knows its exact length.
    assert_eq!(iter.len(), 3);
    assert_eq!(iter.next(), Some((keys[0], 0)));
    assert_eq!(iter.len(), 2);
    let rest: Vec<_> = iter.collect();
    assert_eq!(rest, [(keys[3], 3), (keys[2], 2)]);

    let mut secondary =
        DenseSecondaryMap::<u32, DenseForwardIter>::with_storages(bare(), bare_pairs()).unwrap();
    secondary.insert(keys[2], 2).unwrap();
    secondary.insert(keys[0], 0).unwrap();
    let pairs: Vec<_> = secondary.into_iter().collect();
    assert_eq!(pairs, [(keys[2], 2), (keys[0], 0)]);
}

#[test]
fn into_iter_works_when_the_storage_iterator_has_no_next_back() {
    let mut map = GenMap::<u32, ForwardIter>::with_storage(bare()).unwrap();
    let keys: Vec<_> = (0..4).map(|i| map.insert(i)).collect();
    map.remove(keys[1]);

    let mut iter = map.into_iter();
    // The map counts its own values, so the length is still exact.
    assert_eq!(iter.len(), 3);
    assert_eq!(iter.next(), Some((keys[0], 0)));
    assert_eq!(iter.len(), 2);
    let rest: Vec<_> = iter.collect();
    assert_eq!(rest, [(keys[2], 2), (keys[3], 3)]);
}
