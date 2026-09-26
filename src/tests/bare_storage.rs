//! Storages that implement `SlotStorage` and little else, to check that a
//! map only asks for more where a method needs it.

use crate::{GenMap, KeyConfig, MapConfig, SlotItem, SlotStorage, Split};
use std::vec::Vec;

/// A `Vec` behind `SlotStorage` alone. With `ITER` it also has an owning
/// iterator, which implements `Iterator` but not `DoubleEndedIterator` or
/// `ExactSizeIterator`.
struct Bare<S, const ITER: bool>(Vec<S>);

// SAFETY: every method forwards to the `Vec`, which is the behaviour the
// trait describes.
unsafe impl<S, const ITER: bool> SlotStorage for Bare<S, ITER> {
    type Item = S;
    type Error = ();

    const EMPTY: Self = Bare(Vec::new());

    fn with_capacity(capacity: usize) -> Self {
        Bare(Vec::with_capacity(capacity))
    }

    fn capacity(&self) -> usize {
        self.0.capacity()
    }

    fn as_slice(&self) -> &[S] {
        &self.0
    }

    fn as_mut_slice(&mut self) -> &mut [S] {
        &mut self.0
    }

    fn ensure_room(&mut self) -> Result<(), ()> {
        Ok(())
    }

    fn try_push(&mut self, item: S) -> Result<(), S> {
        self.0.push(item);
        Ok(())
    }

    fn clear(&mut self) {
        self.0.clear();
    }
}

/// A `Vec` iterator that only implements `Iterator`, so it has no
/// `next_back` and no `len`.
struct Forwards<S>(std::vec::IntoIter<S>);

impl<S> Iterator for Forwards<S> {
    type Item = S;

    fn next(&mut self) -> Option<S> {
        self.0.next()
    }
}

impl<S> IntoIterator for Bare<S, true> {
    type Item = S;
    type IntoIter = Forwards<S>;

    fn into_iter(self) -> Forwards<S> {
        Forwards(self.0.into_iter())
    }
}

struct Keys;

impl KeyConfig for Keys {
    type Idx = u32;
    type Gen = u32;
    type Layout = Split;
}

/// Its storage has no owning iterator.
struct NoIter;

impl<S: SlotItem> MapConfig<S> for NoIter {
    type KeyConfig = Keys;
    type Storage = Bare<S, false>;
}

/// Its storage's owning iterator is a `Forwards`.
struct ForwardIter;

impl<S: SlotItem> MapConfig<S> for ForwardIter {
    type KeyConfig = Keys;
    type Storage = Bare<S, true>;
}

#[test]
fn a_map_works_without_an_owning_iterator() {
    let mut map = GenMap::<u32, NoIter>::new_with_config();
    let a = map.insert(1);
    let b = map.insert(2);
    assert_eq!(map.remove(a), Some(1));
    let c = map.insert(3);
    assert_eq!(c.idx(), a.idx());

    // The borrowing iterators do not need anything from the storage.
    let backwards: Vec<_> = map.iter().rev().map(|(_, value)| *value).collect();
    assert_eq!(backwards, [2, 3]);
    let copy = map.clone();
    assert_eq!((copy[b], copy[c]), (2, 3));
    let drained: Vec<_> = map.drain().collect();
    assert_eq!(drained, [(c, 3), (b, 2)]);
}

#[test]
fn into_iter_works_when_the_storage_iterator_has_no_next_back() {
    let mut map = GenMap::<u32, ForwardIter>::new_with_config();
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
