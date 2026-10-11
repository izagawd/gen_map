//! These tests cover the `Len` choice of a map's config, mostly for maps whose
//! config picks `NoLen`. Such a map has no `len`, so the tests count its values
//! through the iterators.

use super::DropTracker;
use crate::{
    GenMap, GenMapConfig, GenMapRawParts, GenSlotItem, KeepLen, Len, MapConfig, NewerWins, NoLen,
    SecondaryMap, SecondaryMapConfig, SecondaryMapRawParts, SecondarySlotItem, Split,
};
use core::marker::PhantomData;
use core::mem::size_of;
use core::panic::{RefUnwindSafe, UnwindSafe};
use std::vec::Vec;

/// This config gives keys a `u64` index and a `u64` generation, keeps the
/// slots in a `Vec`, and picks `L` to decide whether the maps keep a count of
/// their values. It is only used as a type parameter, never created as a
/// value.
struct Wide<L>(PhantomData<L>);

impl<L: Len> MapConfig for Wide<L> {
    type KeyConfig = Split<u64, u64>;
}

impl<L: Len> GenMapConfig for Wide<L> {
    type Len = L;
    type Storage<S: GenSlotItem> = Vec<S>;
}

impl<L: Len> SecondaryMapConfig for Wide<L> {
    type ReplaceStrategy = NewerWins;
    type Len = L;
    type Storage<S: SecondarySlotItem> = Vec<S>;
}

/// Maps with this config keep no count of their values.
type Uncounted = Wide<NoLen>;

/// Maps with this config keep a count of their values.
type Counted = Wide<KeepLen>;

#[test]
fn a_map_without_a_count_takes_no_room_for_one() {
    assert_eq!(
        size_of::<GenMap<u8, Uncounted>>() + size_of::<u64>(),
        size_of::<GenMap<u8, Counted>>()
    );
    assert_eq!(
        size_of::<SecondaryMap<u8, Uncounted>>() + size_of::<u64>(),
        size_of::<SecondaryMap<u8, Counted>>()
    );
}

#[test]
fn a_gen_map_without_a_count_inserts_and_removes() {
    let mut map = GenMap::<u32, Uncounted>::new_with_config();
    let keys: Vec<_> = (0..6).map(|i| map.insert(i)).collect();
    assert_eq!(map.remove(keys[0]), Some(0));
    assert_eq!(map.retire(keys[1]), Some(1));
    assert_eq!(map.detach(keys[2]), Some(2));
    map.reattach(keys[2], 20).unwrap();
    assert_eq!(map.detach(keys[3]), Some(3));
    assert!(map.release(keys[3]));
    map.retain(|_, value| *value != 4);

    let entry = map.vacant_entry().unwrap();
    let promised = entry.key();
    assert_eq!(entry.insert(6), promised);
    let values: Vec<_> = map.values().copied().collect();
    assert_eq!(values, [20, 6, 5]);

    map.clear();
    assert_eq!(map.iter().count(), 0);
    assert_eq!(map.slots_len(), 6);
    map.reset();
    assert_eq!(map.slots_len(), 0);
}

#[test]
fn the_iterators_of_a_gen_map_without_a_count_only_bound_the_values_left() {
    let mut map = GenMap::<u32, Uncounted>::new_with_config();
    let keys: Vec<_> = (0..4).map(|i| map.insert(i)).collect();
    map.remove(keys[1]);

    // Three values sit in four slots. Without a count, each iterator only
    // knows that it has at most one value left for each slot it has left.
    assert_eq!(map.iter().size_hint(), (0, Some(4)));
    assert_eq!(map.keys().size_hint(), (0, Some(4)));
    assert_eq!(map.values().size_hint(), (0, Some(4)));
    assert_eq!(map.iter_mut().size_hint(), (0, Some(4)));
    assert_eq!(map.values_mut().size_hint(), (0, Some(4)));
    assert_eq!(map.clone().into_iter().size_hint(), (0, Some(4)));

    // The second `next` skips the empty slot at index one.
    let mut iter = map.iter();
    iter.next();
    assert_eq!(iter.size_hint(), (0, Some(3)));
    iter.next();
    assert_eq!(iter.size_hint(), (0, Some(1)));
    assert_eq!(iter.count(), 1);

    let mut drain = map.drain();
    assert_eq!(drain.size_hint(), (0, Some(4)));
    drain.next();
    assert_eq!(drain.size_hint(), (0, Some(3)));
    assert_eq!(drain.count(), 2);
    assert_eq!(map.iter().count(), 0);
}

#[test]
fn a_gen_map_without_a_count_drops_every_value_once() {
    let tracker = DropTracker::new();
    let mut map = GenMap::<_, Uncounted>::new_with_config();
    let keys: Vec<_> = (0..8).map(|_| map.insert(tracker.make_item())).collect();
    drop(map.remove(keys[0]));

    let mut copy = map.clone();
    copy.clone_from(&map);
    assert_eq!(copy.drain().take(2).count(), 2);
    copy.clear();
    map.reset();
    drop((map, copy));

    // The map made eight items. The copy took a clone of each of the seven
    // left in the map twice, once in `clone` and again in `clone_from`.
    tracker.assert_all_dropped_exactly_once(8 + 7 + 7);
}

#[test]
fn the_parts_of_a_gen_map_without_a_count_have_no_count() {
    let mut map = GenMap::<u32, Uncounted>::new_with_config();
    let a = map.insert(1);
    let b = map.insert(2);
    map.remove(a);

    let GenMapRawParts {
        slots,
        next_free,
        len: (),
    } = map.into_raw_parts();
    assert_eq!(slots.len(), 2);
    assert_eq!(next_free, a.idx());

    let parts: GenMapRawParts<u32, Uncounted> = GenMapRawParts {
        slots,
        next_free,
        len: (),
    };
    // SAFETY: the parts came from `into_raw_parts` unchanged.
    let map = unsafe { GenMap::from_raw_parts(parts) };
    assert_eq!(map[b], 2);
    assert!(map.get(a).is_none());
}

#[test]
fn a_secondary_map_without_a_count_inserts_and_removes() {
    let mut keys = GenMap::<(), Uncounted>::new_with_config();
    let all: Vec<_> = (0..5).map(|_| keys.insert(())).collect();
    let mut map = SecondaryMap::<u32, Uncounted>::new_with_config();
    for (i, &key) in all.iter().enumerate() {
        assert_eq!(map.insert(key, i as u32).unwrap(), None);
    }
    assert_eq!(map.insert(all[0], 10).unwrap(), Some(0));
    assert_eq!(map.remove(all[1]), Some(1));
    map.retain(|_, value| *value != 2);

    let values: Vec<_> = map.values().copied().collect();
    assert_eq!(values, [10, 3, 4]);

    let drained: Vec<_> = map.drain().map(|(_, value)| value).collect();
    assert_eq!(drained, [10, 3, 4]);
    map.insert(all[4], 4).unwrap();
    map.clear();
    assert_eq!(map.iter().count(), 0);
    assert_eq!(map.slots_len(), 5);
}

#[test]
fn the_iterators_of_a_secondary_map_without_a_count_only_bound_the_values_left() {
    let mut keys = GenMap::<(), Uncounted>::new_with_config();
    let all: Vec<_> = (0..4).map(|_| keys.insert(())).collect();
    let mut map = SecondaryMap::<u32, Uncounted>::new_with_config();
    for key in [all[0], all[2], all[3]] {
        map.insert(key, 0).unwrap();
    }

    assert_eq!(map.iter().size_hint(), (0, Some(4)));
    assert_eq!(map.keys().size_hint(), (0, Some(4)));
    assert_eq!(map.values().size_hint(), (0, Some(4)));
    assert_eq!(map.iter_mut().size_hint(), (0, Some(4)));
    assert_eq!(map.values_mut().size_hint(), (0, Some(4)));
    assert_eq!(map.clone().into_iter().size_hint(), (0, Some(4)));

    // The second `next` skips the empty slot at index one.
    let mut iter = map.iter();
    iter.next();
    iter.next();
    assert_eq!(iter.size_hint(), (0, Some(1)));

    let mut drain = map.drain();
    drain.next();
    assert_eq!(drain.size_hint(), (0, Some(3)));
    assert_eq!(drain.count(), 2);
}

#[test]
fn the_parts_of_a_secondary_map_without_a_count_have_no_count() {
    let mut keys = GenMap::<(), Uncounted>::new_with_config();
    let a = keys.insert(());
    let mut map = SecondaryMap::<u32, Uncounted>::new_with_config();
    map.insert(a, 1).unwrap();

    let SecondaryMapRawParts { slots, len: () } = map.into_raw_parts();
    assert_eq!(slots.len(), 1);
    let parts: SecondaryMapRawParts<u32, Uncounted> = SecondaryMapRawParts { slots, len: () };
    // SAFETY: the parts came from `into_raw_parts` unchanged.
    let map = unsafe { SecondaryMap::from_raw_parts(parts) };
    assert_eq!(map[a], 1);
}

/// Fails to compile unless `X` is `Send`, `Sync` and `Unpin`.
fn assert_send_sync_unpin<X: Send + Sync + Unpin>(_value: &X) {}

/// Fails to compile unless `X` is `Send`, `Sync`, `Unpin` and unwind safe.
fn assert_auto_traits<X: Send + Sync + Unpin + UnwindSafe + RefUnwindSafe>(_value: &X) {}

/// Fails to compile unless the borrowing iterators of a `GenMap` and a
/// `SecondaryMap` have the auto traits in code that is generic over the
/// config. An iterator that hands out `&mut T` is not unwind safe, so only
/// the other iterators are checked for that.
fn check_borrowing_iterators<T, C>(map: &mut GenMap<T, C>, secondary: &mut SecondaryMap<T, C>)
where
    T: Send + Sync + RefUnwindSafe,
    C: GenMapConfig + SecondaryMapConfig,
{
    assert_auto_traits(&map.iter());
    assert_auto_traits(&map.keys());
    assert_auto_traits(&map.values());
    assert_send_sync_unpin(&map.iter_mut());
    assert_send_sync_unpin(&map.values_mut());
    assert_auto_traits(&secondary.iter());
    assert_auto_traits(&secondary.keys());
    assert_auto_traits(&secondary.values());
    assert_send_sync_unpin(&secondary.iter_mut());
    assert_send_sync_unpin(&secondary.values_mut());
}

#[test]
fn borrowing_iterators_have_the_auto_traits_for_any_config() {
    check_borrowing_iterators(
        &mut GenMap::<u8, Uncounted>::new_with_config(),
        &mut SecondaryMap::<u8, Uncounted>::new_with_config(),
    );
    check_borrowing_iterators(
        &mut GenMap::<u8, Counted>::new_with_config(),
        &mut SecondaryMap::<u8, Counted>::new_with_config(),
    );
}
