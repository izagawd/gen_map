#[cfg(feature = "alloc")]
use crate::config::DefaultMapConfig;
use crate::config::DenseSecondaryMapConfig;
use crate::dense::map::{
    to_position, to_stored, DenseIntoIter, DenseIter, DenseIterMut, DenseKeys, DenseValues,
    DenseValuesMut,
};
use crate::error::{
    check_disjoint_idxs, check_disjoint_keys, DenseError, GetDisjointMutAtError,
    GetDisjointMutError, SecondaryInsertError,
};
use crate::key::parity::{Even, Odd};
use crate::key::piece::KeyPiece;
use crate::key::Key;
use crate::map::{MapGen, MapIdx, MapKeyConfig};
use crate::secondary::replace_strategy::ReplaceStrategy;
use crate::secondary::{get_or_grow_slot, key_from_parts_unchecked};
use crate::slot::{ParityRef, Slot};
use crate::storage::pair::{ClearPairsOnUnwind, PairStorage, ReservePairStorage};
use crate::storage::{ClearOnUnwind, ReserveStorage, SliceStorage, WithCapacity};
use core::fmt;
use core::iter::FusedIterator;
use core::ops::{Index, IndexMut};

/// The storage a config gives the map for its slots.
type Slots<C> = <C as DenseSecondaryMapConfig>::SlotStorage<DenseSecondaryMapSlot<C>>;

/// The strategy a config gives the map.
type Strategy<C> = <C as DenseSecondaryMapConfig>::ReplaceStrategy;

/// The storage a config gives the map for its keys and values.
type Pairs<T, C> = <C as DenseSecondaryMapConfig>::PairStorage<Key<MapKeyConfig<C>>, T>;

/// A [`DenseSecondaryMap`] keeps one [`Slot`] of this type for each index.
/// While the slot holds a value, its generation is the generation of the
/// value's key, which is odd, and it stores the position of that value in the
/// pair storage. While it holds no value, its generation is zero, which is
/// even, and it stores `()` in place of a position.
pub type DenseSecondaryMapSlot<C> = Slot<MapGen<C>, MapIdx<C>, ()>;

/// Returns the generation and the stored position of `slot` if it holds a
/// value, or `None` if it holds none.
#[inline]
fn occupied<C: DenseSecondaryMapConfig>(
    slot: &DenseSecondaryMapSlot<C>,
) -> Option<(Odd<MapGen<C>>, MapIdx<C>)> {
    match slot.as_parity() {
        ParityRef::Odd(&generation, &stored_position) => Some((generation, stored_position)),
        ParityRef::Even(..) => None,
    }
}

/// The error a `DenseSecondaryMap<T, C>` gives when one of its storages cannot
/// make room.
pub type DenseSecondaryStorageError<T, C> = DenseError<
    <<C as DenseSecondaryMapConfig>::SlotStorage<DenseSecondaryMapSlot<C>> as SliceStorage>::Error,
    <<C as DenseSecondaryMapConfig>::PairStorage<Key<MapKeyConfig<C>>, T> as PairStorage>::Error,
>;

/// What [`DenseSecondaryMap::insert`] returns.
type InsertResult<T, C> =
    Result<Option<T>, SecondaryInsertError<T, DenseSecondaryStorageError<T, C>>>;

/// A map from the keys of a [`GenMap`](crate::GenMap) to values of type `T`,
/// which keeps its values one after another with no gaps, and is configured by
/// `C`.
///
/// It stores values under the same keys a [`SecondaryMap`](crate::SecondaryMap)
/// with the same config would, and decides the same way whether an insert
/// replaces a value. The difference is where the values live. A `SecondaryMap`
/// keeps each value in the slot at its key's index, while a `DenseSecondaryMap`
/// keeps its values one after another in a [`PairStorage`], and each slot
/// only stores the generation of the value's key and the position of the
/// value. Iterating over the values is then as fast as iterating over a slice,
/// and a lookup takes one more step.
///
/// The pair storage also holds the key of each value, at the same position as
/// the value. Removing a value moves the last value into its place, and the
/// map reads the moved value's key to point that value's slot at the new
/// position.
///
/// With the `alloc` feature, `C` defaults to [`DefaultMapConfig`]. To use
/// your own config, implement [`MapConfig`](crate::MapConfig) and
/// [`DenseSecondaryMapConfig`] for it.
///
/// # Examples
///
/// ```
/// use gen_map::{DenseSecondaryMap, GenMap};
///
/// let mut people = GenMap::new();
/// let mut ages = DenseSecondaryMap::new();
///
/// let alice = people.insert("Alice");
/// let bob = people.insert("Bob");
/// ages.insert(alice, 30).unwrap();
/// ages.insert(bob, 25).unwrap();
///
/// assert_eq!(ages.get(alice), Some(&30));
/// assert_eq!(ages.values().sum::<u32>(), 55);
/// ```
pub struct DenseSecondaryMap<
    T,
    #[cfg(feature = "alloc")] C: DenseSecondaryMapConfig = DefaultMapConfig,
    #[cfg(not(feature = "alloc"))] C: DenseSecondaryMapConfig,
> {
    /// A slot for every index up to the highest index that an insert has
    /// used. A slot that holds a value stores the generation of the value's
    /// key and the position of the value in `pairs`, and every position below
    /// the number of values is stored by exactly one slot.
    slots: Slots<C>,
    /// The values one after another in the second slice, and the key of each
    /// value at the same position in the first slice.
    pairs: Pairs<T, C>,
}

#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
impl<T> DenseSecondaryMap<T> {
    /// Creates an empty map with the [`DefaultMapConfig`].
    ///
    /// This method only exists for the default config, so that
    /// `DenseSecondaryMap::new()` compiles without a type annotation. Use
    /// [`new_with_config`](Self::new_with_config) for any other config.
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self::new_with_config()
    }

    /// Creates an empty map with the [`DefaultMapConfig`] and room for
    /// `capacity` values, slots and keys.
    #[inline]
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self::with_capacity_and_config(capacity)
    }
}

/// A map has this method only when its slot storage and its pair storage
/// implement [`WithCapacity`].
impl<T, C: DenseSecondaryMapConfig> DenseSecondaryMap<T, C>
where
    <C as DenseSecondaryMapConfig>::SlotStorage<DenseSecondaryMapSlot<C>>: WithCapacity,
    Pairs<T, C>: WithCapacity,
{
    /// Creates an empty map with config `C` and room for `capacity` values,
    /// slots and keys.
    #[inline]
    #[must_use]
    pub fn with_capacity_and_config(capacity: usize) -> Self {
        Self {
            slots: <Slots<C> as WithCapacity>::with_capacity(capacity),
            pairs: <Pairs<T, C> as WithCapacity>::with_capacity(capacity),
        }
    }
}

/// These methods need slot and pair storages that can grow on request, so a
/// map whose storages have a fixed capacity does not have them.
impl<T, C: DenseSecondaryMapConfig> DenseSecondaryMap<T, C>
where
    <C as DenseSecondaryMapConfig>::SlotStorage<DenseSecondaryMapSlot<C>>: ReserveStorage,
    Pairs<T, C>: ReservePairStorage,
{
    /// Reserves room for at least `additional` more values, and for as many
    /// more slots and keys.
    ///
    /// # Panics
    ///
    /// Panics if a storage cannot make the room. Use
    /// [`try_reserve`](Self::try_reserve) to get an error instead.
    #[inline]
    pub fn reserve(&mut self, additional: usize) {
        if let Err(error) = self.try_reserve(additional) {
            panic!("DenseSecondaryMap cannot make room for {additional} more values: {error:?}");
        }
    }

    /// The fallible form of [`reserve`](Self::reserve).
    ///
    /// # Errors
    ///
    /// Returns the error of the first storage that cannot make the room. The
    /// map tries the slot storage first, then the pair storage.
    #[inline]
    pub fn try_reserve(
        &mut self,
        additional: usize,
    ) -> Result<(), DenseSecondaryStorageError<T, C>> {
        self.slots
            .ensure_room(additional)
            .map_err(DenseError::Slots)?;
        self.pairs
            .ensure_room(additional)
            .map_err(DenseError::Pairs)
    }
}

impl<T, C: DenseSecondaryMapConfig> DenseSecondaryMap<T, C> {
    /// Creates an empty map with config `C`.
    #[inline]
    #[must_use]
    pub fn new_with_config() -> Self {
        Self {
            slots: <Slots<C> as SliceStorage>::empty(),
            pairs: <Pairs<T, C> as PairStorage>::empty(),
        }
    }

    /// The smaller capacity of the map's slot and pair storages. The map can
    /// hold that many slots and values before one of its storages has to
    /// grow, or in total if they cannot grow.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.slots.capacity().min(self.pairs.capacity())
    }

    /// Returns the number of values in the map.
    #[inline]
    pub fn len(&self) -> usize {
        self.pairs.len()
    }

    /// Returns `true` if the map holds no values.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    /// The number of slots, whether they hold a value or not.
    #[inline]
    pub fn slots_len(&self) -> usize {
        self.slots.len()
    }

    /// Returns `true` if a value is stored under `key`.
    #[inline]
    pub fn contains_key(&self, key: Key<MapKeyConfig<C>>) -> bool {
        self.stored_position(key).is_some()
    }

    /// Returns a reference to the value stored under `key`, or `None` if
    /// there is none.
    #[inline]
    pub fn get(&self, key: Key<MapKeyConfig<C>>) -> Option<&T> {
        let stored_position = self.stored_position(key)?;
        // SAFETY: `stored_position` is the position the key's slot stores,
        // which is below the number of values.
        Some(unsafe {
            self.pairs
                .second_slice()
                .get_unchecked(to_position::<C>(stored_position))
        })
    }

    /// Returns a mutable reference to the value stored under `key`, or `None`
    /// if there is none.
    #[inline]
    pub fn get_mut(&mut self, key: Key<MapKeyConfig<C>>) -> Option<&mut T> {
        let stored_position = self.stored_position(key)?;
        // SAFETY: the same as in `get`.
        Some(unsafe {
            self.pairs
                .second_slice_mut()
                .get_unchecked_mut(to_position::<C>(stored_position))
        })
    }

    /// Returns a reference to the value stored under `key`, like
    /// [`get`](Self::get), but without checking that there is one.
    ///
    /// # Safety
    ///
    /// A value must be stored under `key`, meaning
    /// [`contains_key`](Self::contains_key) returns `true` for it.
    #[inline]
    pub unsafe fn get_unchecked(&self, key: Key<MapKeyConfig<C>>) -> &T {
        debug_assert!(self.contains_key(key));
        // SAFETY: the caller promises that a value is stored under `key`, so
        // the key's index is the position of a slot that holds a value, and the
        // position that slot stores is below the number of values.
        unsafe {
            let slot = self
                .slots
                .as_slice()
                .get_unchecked(key.idx().into_usize_unchecked());
            let position = to_position::<C>(*slot.get_odd_unchecked());
            self.pairs.second_slice().get_unchecked(position)
        }
    }

    /// Returns a mutable reference to the value stored under `key`, like
    /// [`get_mut`](Self::get_mut), but without checking that there is one.
    ///
    /// # Safety
    ///
    /// A value must be stored under `key`, meaning
    /// [`contains_key`](Self::contains_key) returns `true` for it.
    #[inline]
    pub unsafe fn get_unchecked_mut(&mut self, key: Key<MapKeyConfig<C>>) -> &mut T {
        debug_assert!(self.contains_key(key));
        // SAFETY: the same as in `get_unchecked`.
        unsafe {
            let slot = self
                .slots
                .as_slice()
                .get_unchecked(key.idx().into_usize_unchecked());
            let position = to_position::<C>(*slot.get_odd_unchecked());
            self.pairs.second_slice_mut().get_unchecked_mut(position)
        }
    }

    /// Returns the key of the value in the slot at `idx`, or `None` if there
    /// is no slot at `idx` or the slot holds no value.
    #[inline]
    pub fn key_at(&self, idx: MapIdx<C>) -> Option<Key<MapKeyConfig<C>>> {
        let slot_index = idx.into_usize()?;
        let (generation, _) = occupied::<C>(self.slots.as_slice().get(slot_index)?)?;
        // SAFETY: the slot at `slot_index` holds a value under `generation`, so
        // the two fit the key.
        Some(unsafe { key_from_parts_unchecked::<C>(slot_index, generation) })
    }

    /// Returns the key of the value in the slot at `idx`, like
    /// [`key_at`](Self::key_at), but without checking that there is a slot at
    /// `idx` or that it holds a value.
    ///
    /// # Safety
    ///
    /// There must be a slot at `idx` and it must hold a value, meaning
    /// [`key_at`](Self::key_at) returns `Some` for it.
    #[inline]
    pub unsafe fn key_at_unchecked(&self, idx: MapIdx<C>) -> Key<MapKeyConfig<C>> {
        debug_assert!(self.key_at(idx).is_some());
        // SAFETY: the caller promises a slot at `idx` that holds a value. So
        // `idx` fits in `usize` and points at a slot in bounds, the slot's
        // generation is odd, and the position and generation fit the key.
        unsafe {
            let slot_index = idx.into_usize_unchecked();
            let slot = self.slots.as_slice().get_unchecked(slot_index);
            key_from_parts_unchecked::<C>(slot_index, Odd::new_unchecked(slot.generation()))
        }
    }

    /// Returns the key and a reference to the value in the slot at `idx`, or
    /// `None` if there is no slot at `idx` or the slot holds no value.
    #[inline]
    pub fn get_at(&self, idx: MapIdx<C>) -> Option<(Key<MapKeyConfig<C>>, &T)> {
        let slot_index = idx.into_usize()?;
        let (generation, stored_position) = occupied::<C>(self.slots.as_slice().get(slot_index)?)?;
        // SAFETY: the slot at `slot_index` holds a value under `generation`, so
        // the two fit the key, and the position the slot stores is below the
        // number of values.
        unsafe {
            let key = key_from_parts_unchecked::<C>(slot_index, generation);
            Some((
                key,
                self.pairs
                    .second_slice()
                    .get_unchecked(to_position::<C>(stored_position)),
            ))
        }
    }

    /// Returns the key and a mutable reference to the value in the slot at
    /// `idx`, or `None` if there is no slot at `idx` or the slot holds no
    /// value.
    #[inline]
    pub fn get_at_mut(&mut self, idx: MapIdx<C>) -> Option<(Key<MapKeyConfig<C>>, &mut T)> {
        let slot_index = idx.into_usize()?;
        let (generation, stored_position) = occupied::<C>(self.slots.as_slice().get(slot_index)?)?;
        // SAFETY: the same as in `get_at`.
        unsafe {
            let key = key_from_parts_unchecked::<C>(slot_index, generation);
            let value = self
                .pairs
                .second_slice_mut()
                .get_unchecked_mut(to_position::<C>(stored_position));
            Some((key, value))
        }
    }

    /// Returns the key and a reference to the value in the slot at `idx`, like
    /// [`get_at`](Self::get_at), but without checking that there is a slot at
    /// `idx` or that it holds a value.
    ///
    /// # Safety
    ///
    /// There must be a slot at `idx` and it must hold a value, meaning
    /// [`key_at`](Self::key_at) returns `Some` for it.
    #[inline]
    pub unsafe fn get_at_unchecked(&self, idx: MapIdx<C>) -> (Key<MapKeyConfig<C>>, &T) {
        debug_assert!(self.key_at(idx).is_some());
        // SAFETY: the caller promises a slot at `idx` that holds a value. So
        // `idx` points at a slot in bounds, the slot's generation is odd, the
        // position and generation fit the key, and the position the slot stores
        // is below the number of values.
        unsafe {
            let slot_index = idx.into_usize_unchecked();
            let slot = self.slots.as_slice().get_unchecked(slot_index);
            let key =
                key_from_parts_unchecked::<C>(slot_index, Odd::new_unchecked(slot.generation()));
            let stored_position = *slot.get_odd_unchecked();
            (
                key,
                self.pairs
                    .second_slice()
                    .get_unchecked(to_position::<C>(stored_position)),
            )
        }
    }

    /// Returns the key and a mutable reference to the value in the slot at
    /// `idx`, like [`get_at_mut`](Self::get_at_mut), but without checking that
    /// there is a slot at `idx` or that it holds a value.
    ///
    /// # Safety
    ///
    /// There must be a slot at `idx` and it must hold a value, meaning
    /// [`key_at`](Self::key_at) returns `Some` for it.
    #[inline]
    pub unsafe fn get_at_unchecked_mut(
        &mut self,
        idx: MapIdx<C>,
    ) -> (Key<MapKeyConfig<C>>, &mut T) {
        debug_assert!(self.key_at(idx).is_some());
        // SAFETY: the same as in `get_at_unchecked`.
        unsafe {
            let slot_index = idx.into_usize_unchecked();
            let slot = self.slots.as_slice().get_unchecked(slot_index);
            let key =
                key_from_parts_unchecked::<C>(slot_index, Odd::new_unchecked(slot.generation()));
            let stored_position = *slot.get_odd_unchecked();
            let value = self
                .pairs
                .second_slice_mut()
                .get_unchecked_mut(to_position::<C>(stored_position));
            (key, value)
        }
    }

    /// Returns the generation of the slot at `idx`, or `None` if there is no
    /// slot at `idx`. It works the same way as
    /// [`SecondaryMap::generation_at`](crate::SecondaryMap::generation_at).
    #[inline]
    pub fn generation_at(&self, idx: MapIdx<C>) -> Option<MapGen<C>> {
        Some(self.slots.as_slice().get(idx.into_usize()?)?.generation())
    }

    /// Returns the generation of the slot at `idx`, like
    /// [`generation_at`](Self::generation_at), but without checking that there
    /// is a slot at `idx`.
    ///
    /// # Safety
    ///
    /// There must be a slot at `idx`, meaning
    /// [`generation_at`](Self::generation_at) returns `Some` for it.
    #[inline]
    pub unsafe fn generation_at_unchecked(&self, idx: MapIdx<C>) -> MapGen<C> {
        debug_assert!(self.generation_at(idx).is_some());
        // SAFETY: the caller promises a slot at `idx`, so `idx` fits in `usize`
        // and points at a slot in bounds.
        unsafe {
            self.slots
                .as_slice()
                .get_unchecked(idx.into_usize_unchecked())
                .generation()
        }
    }

    /// Returns the key and a mutable reference to the value in the slot at each
    /// index, all at once. It checks the indices the same way
    /// [`SecondaryMap::get_disjoint_mut_at`](crate::SecondaryMap::get_disjoint_mut_at)
    /// does.
    ///
    /// # Errors
    ///
    /// Returns [`GetDisjointMutAtError::NoValue`] if there is no slot at one
    /// of the indices or the slot holds no value, and
    /// [`GetDisjointMutAtError::OverlappingIndices`] if two indices are the
    /// same.
    #[inline]
    #[allow(clippy::type_complexity)]
    pub fn get_disjoint_mut_at<const N: usize>(
        &mut self,
        idxs: [MapIdx<C>; N],
    ) -> Result<[(Key<MapKeyConfig<C>>, &mut T); N], GetDisjointMutAtError> {
        check_disjoint_idxs(&idxs, |idx| self.key_at(*idx).is_some())?;
        // SAFETY: every index was just found to point at a slot that holds a
        // value, and every index is distinct.
        Ok(unsafe { self.get_disjoint_mut_at_unchecked(idxs) })
    }

    /// Returns the key and a mutable reference to the value in the slot at
    /// each index, like [`get_disjoint_mut_at`](Self::get_disjoint_mut_at),
    /// but without any of its checks.
    ///
    /// # Safety
    ///
    /// There must be a slot at every index and each must hold a value,
    /// meaning [`key_at`](Self::key_at) returns `Some` for every one of them,
    /// and no two of the indices may be the same.
    #[inline]
    pub unsafe fn get_disjoint_mut_at_unchecked<const N: usize>(
        &mut self,
        idxs: [MapIdx<C>; N],
    ) -> [(Key<MapKeyConfig<C>>, &mut T); N] {
        debug_assert!(check_disjoint_idxs(&idxs, |idx| self.key_at(*idx).is_some()).is_ok());
        // SAFETY: the caller promises that every index has a slot that holds a
        // value and that the indices are different. So each index points at a
        // slot in bounds with an odd generation, the slots are different, and
        // so are the positions they store.
        unsafe {
            let slots = self.slots.as_slice();
            let stored_positions = idxs.map(|idx| {
                let slot_index = idx.into_usize_unchecked();
                let slot = slots.get_unchecked(slot_index);
                let generation = Odd::new_unchecked(slot.generation());
                (
                    key_from_parts_unchecked::<C>(slot_index, generation),
                    *slot.get_odd_unchecked(),
                )
            });
            self.values_at(stored_positions)
        }
    }

    /// Returns a mutable reference to the value stored under each key, all at
    /// once. It checks the keys the same way
    /// [`SecondaryMap::get_disjoint_mut`](crate::SecondaryMap::get_disjoint_mut)
    /// does.
    ///
    /// # Errors
    ///
    /// Returns [`GetDisjointMutError::InvalidKey`] if no value is stored
    /// under one of the keys, and [`GetDisjointMutError::OverlappingKeys`] if
    /// two keys point at the same slot.
    #[inline]
    pub fn get_disjoint_mut<const N: usize>(
        &mut self,
        keys: [Key<MapKeyConfig<C>>; N],
    ) -> Result<[&mut T; N], GetDisjointMutError> {
        check_disjoint_keys(&keys, |key| self.contains_key(key))?;
        // SAFETY: every key was just found to have a value, and every index is
        // distinct.
        Ok(unsafe { self.get_disjoint_mut_unchecked(keys) })
    }

    /// Returns a mutable reference to the value stored under each key, like
    /// [`get_disjoint_mut`](Self::get_disjoint_mut), but without any of its
    /// checks.
    ///
    /// # Safety
    ///
    /// A value must be stored under every key, meaning
    /// [`contains_key`](Self::contains_key) returns `true` for each of them,
    /// and no two keys may point at the same slot.
    #[inline]
    pub unsafe fn get_disjoint_mut_unchecked<const N: usize>(
        &mut self,
        keys: [Key<MapKeyConfig<C>>; N],
    ) -> [&mut T; N] {
        debug_assert!(check_disjoint_keys(&keys, |key| self.contains_key(key)).is_ok());
        // SAFETY: the caller promises that a value is stored under every key
        // and that no two keys point at the same slot. So each key's index
        // points at a slot in bounds that holds a value, and the positions
        // those slots store are different.
        unsafe {
            let slots = self.slots.as_slice();
            let stored_positions = keys.map(|key| {
                let slot = slots.get_unchecked(key.idx().into_usize_unchecked());
                ((), *slot.get_odd_unchecked())
            });
            self.values_at(stored_positions).map(|((), value)| value)
        }
    }

    /// Returns a mutable reference to the value at each stored position, paired
    /// with the item that came with that position. The item is the value's key
    /// for the `_at` methods, and `()` for the others.
    ///
    /// # Safety
    ///
    /// Every position must be one that a slot holding a value stores, and no
    /// two of them may be the same.
    #[inline]
    unsafe fn values_at<X, const N: usize>(
        &mut self,
        stored_positions: [(X, MapIdx<C>); N],
    ) -> [(X, &mut T); N] {
        let values = self.pairs.second_slice_mut().as_mut_ptr();
        stored_positions.map(|(item, stored_position)| {
            // SAFETY: the caller promises that a slot holding a value stores
            // `stored_position`, so it is below the number of values, and that
            // no other position is the same, so the references do not alias.
            (item, unsafe {
                &mut *values.add(to_position::<C>(stored_position))
            })
        })
    }

    /// Stores `value` under `key`.
    ///
    /// - If no value is stored at the key's index, the value goes in at the
    ///   end of the values and `Ok(None)` is returned.
    /// - If a value is stored under the same key, that value is replaced and
    ///   returned as `Ok(Some(old))`.
    /// - If the value stored at the key's index was inserted under a
    ///   different generation, `C`'s
    ///   [`ReplaceStrategy`](DenseSecondaryMapConfig::ReplaceStrategy) decides
    ///   whether `value` replaces it. If it does, the old value is returned as
    ///   `Ok(Some(old))`, and the slot now belongs to `key`.
    ///
    /// # Errors
    ///
    /// Hands `value` back, and leaves the map as it was, if the strategy
    /// kept the old value, the key's index is the largest value of the index
    /// type, or a storage could not make room. The [`SecondaryInsertError`]
    /// variant says which of the three happened, and the [`DenseError`] in a
    /// storage error says which storage it was.
    ///
    /// # Examples
    ///
    /// ```
    /// use gen_map::{DenseSecondaryMap, GenMap, SecondaryInsertError};
    ///
    /// let mut people = GenMap::new();
    /// let mut ages = DenseSecondaryMap::new();
    ///
    /// let alice = people.insert("Alice");
    /// assert_eq!(ages.insert(alice, 30).unwrap(), None);
    /// assert_eq!(ages.insert(alice, 31).unwrap(), Some(30));
    ///
    /// // Bob gets Alice's slot, with a newer generation, so his age replaces
    /// // hers, and Alice's old key can no longer replace his.
    /// people.remove(alice);
    /// let bob = people.insert("Bob");
    /// assert_eq!(ages.insert(bob, 25).unwrap(), Some(31));
    /// assert!(matches!(ages.insert(alice, 32), Err(SecondaryInsertError::Refused(32))));
    /// assert_eq!(ages[bob], 25);
    /// ```
    pub fn insert(&mut self, key: Key<MapKeyConfig<C>>, value: T) -> InsertResult<T, C> {
        // No `GenMap` gives a slot the largest value of the index type as its
        // index, so only a hand-built key can have it. Rejecting it caps the
        // number of slots at that largest value, so every position fits in the
        // index type.
        if key.idx() == MapIdx::<C>::MAX {
            return Err(SecondaryInsertError::IndexReserved(value));
        }
        let new_generation = key.generation();
        let existing = key
            .idx()
            .into_usize()
            .and_then(|slot_index| self.slots.as_slice().get(slot_index))
            .and_then(occupied::<C>);
        let Some((existing_generation, stored_position)) = existing else {
            return self.insert_new(key, value);
        };
        if existing_generation != new_generation
            && !<Strategy<C> as ReplaceStrategy<MapKeyConfig<C>>>::replaces(
                existing_generation,
                new_generation,
            )
        {
            return Err(SecondaryInsertError::Refused(value));
        }
        // SAFETY: `stored_position` is the position the slot at the key's index
        // stores.
        let value_position = unsafe { to_position::<C>(stored_position) };
        if existing_generation != new_generation {
            // The value keeps its place. Its slot now belongs to `key`, and
            // `key` replaces the key stored for the value.
            // SAFETY: the slot at the key's index exists, since it was just
            // read.
            unsafe { self.slot_unchecked_mut(key) }.set_odd(new_generation, stored_position);
            let keys = self.pairs.first_slice_mut();
            // SAFETY: the pair storage holds a key for each value, so it has a
            // key at `value_position`, the position of a value.
            unsafe { *keys.get_unchecked_mut(value_position) = key };
        }
        let values = self.pairs.second_slice_mut();
        // SAFETY: `value_position` is the position of a value, which is below
        // the number of values.
        let stored_value = unsafe { values.get_unchecked_mut(value_position) };
        Ok(Some(core::mem::replace(stored_value, value)))
    }

    /// Stores `value` under `key`, at the end of the values, when no value is
    /// stored at the key's index.
    fn insert_new(&mut self, key: Key<MapKeyConfig<C>>, value: T) -> InsertResult<T, C> {
        if let Err(error) = self.pairs.ensure_room(1) {
            return Err(SecondaryInsertError::StorageFull(
                value,
                DenseError::Pairs(error),
            ));
        }
        // SAFETY: `self.pairs.len()` is the number of values.
        let stored_position = unsafe { to_stored::<C>(self.pairs.len()) };
        match get_or_grow_slot(&mut self.slots, key.idx()) {
            Ok(slot) => {
                slot.set_odd(key.generation(), stored_position);
            }
            Err(error) => {
                return Err(SecondaryInsertError::StorageFull(
                    value,
                    DenseError::Slots(error),
                ));
            }
        }
        // SAFETY: `ensure_room(1)` returned `Ok` above, and no other `&mut`
        // method of the pair storage has run since, so `PairStorage` promises
        // room for this push.
        unsafe { self.pairs.push_unchecked(key, value) };
        Ok(None)
    }

    /// Removes every value. The slots stay, as they do after
    /// [`remove`](Self::remove).
    pub fn clear(&mut self) {
        for slot in self.slots.as_mut_slice() {
            if slot.is_odd() {
                slot.set_even(Even::ZERO, ());
            }
        }
        // Emptying a slot drops no value, so the slots already describe an
        // empty map when the pair storage drops the keys and values, and
        // `PairStorage` promises that the pair storage is empty afterwards,
        // even when dropping a value panics.
        self.pairs.clear();
    }

    /// Returns an iterator over references to the values, in the order they
    /// are stored.
    #[inline]
    pub fn values(&self) -> DenseValues<'_, T> {
        DenseValues(self.pairs.second_slice().iter())
    }

    /// Returns an iterator over mutable references to the values, in the
    /// order they are stored.
    #[inline]
    pub fn values_mut(&mut self) -> DenseValuesMut<'_, T> {
        DenseValuesMut(self.pairs.second_slice_mut().iter_mut())
    }

    /// Returns the keys and the values as two slices, in the order the values
    /// are stored. The value at each position in the second slice is stored
    /// under the key at the same position in the first slice.
    ///
    /// # Examples
    ///
    /// ```
    /// use gen_map::{DenseSecondaryMap, GenMap};
    ///
    /// let mut people = GenMap::new();
    /// let mut ages = DenseSecondaryMap::new();
    /// let alice = people.insert("Alice");
    /// let bob = people.insert("Bob");
    /// ages.insert(alice, 30).unwrap();
    /// ages.insert(bob, 25).unwrap();
    ///
    /// let (keys, values) = ages.as_slices();
    /// assert_eq!(keys, [alice, bob]);
    /// assert_eq!(values, [30, 25]);
    /// ```
    #[inline]
    pub fn as_slices(&self) -> (&[Key<MapKeyConfig<C>>], &[T]) {
        self.pairs.slices()
    }

    /// Returns the keys as a slice and the values as a mutable slice, in the
    /// order the values are stored. The value at each position in the second
    /// slice is stored under the key at the same position in the first slice.
    ///
    /// # Examples
    ///
    /// ```
    /// use gen_map::{DenseSecondaryMap, GenMap};
    ///
    /// let mut people = GenMap::new();
    /// let mut ages = DenseSecondaryMap::new();
    /// let alice = people.insert("Alice");
    /// let bob = people.insert("Bob");
    /// ages.insert(alice, 30).unwrap();
    /// ages.insert(bob, 25).unwrap();
    ///
    /// let (_, values) = ages.as_slices_mut();
    /// values[0] = 31;
    /// assert_eq!((ages[alice], ages[bob]), (31, 25));
    /// ```
    #[inline]
    pub fn as_slices_mut(&mut self) -> (&[Key<MapKeyConfig<C>>], &mut [T]) {
        let (keys, values) = self.pairs.slices_mut();
        (keys, values)
    }

    /// The position of the value stored under `key`, as its slot stores it, or
    /// `None` if no value is stored under `key`.
    #[inline]
    fn stored_position(&self, key: Key<MapKeyConfig<C>>) -> Option<MapIdx<C>> {
        let slot_index = key.idx().into_usize()?;
        self.slots
            .as_slice()
            .get(slot_index)?
            .get_odd(key.generation())
            .copied()
    }

    /// Returns the slot at the index of `key` without checking that it exists.
    ///
    /// # Safety
    ///
    /// There must be a slot at the key's index, which is the case whenever a
    /// value is stored under `key`.
    #[inline]
    unsafe fn slot_unchecked_mut(
        &mut self,
        key: Key<MapKeyConfig<C>>,
    ) -> &mut DenseSecondaryMapSlot<C> {
        debug_assert!(key
            .idx()
            .into_usize()
            .is_some_and(|position| position < self.slots.len()));
        // SAFETY: the caller promises a slot at the key's index, so the index
        // fits in `usize` and is in bounds.
        unsafe {
            self.slots
                .as_mut_slice()
                .get_unchecked_mut(key.idx().into_usize_unchecked())
        }
    }

    /// Removes the value stored under `key` and returns it, or returns
    /// `None` if there is none. The slot stays without a value, and the last
    /// value moves into the place of the removed one.
    #[inline]
    pub fn remove(&mut self, key: Key<MapKeyConfig<C>>) -> Option<T> {
        let slot_index = key.idx().into_usize()?;
        let slot = self.slots.as_mut_slice().get_mut(slot_index)?;
        slot.get_odd(key.generation())?;
        // SAFETY: the slot's generation matches the key's, which is odd.
        let stored_position = unsafe { slot.replace_odd_unchecked(Even::ZERO, ()) };
        // SAFETY: `stored_position` is the position the key's slot stored,
        // which is below the number of values.
        Some(unsafe { self.swap_remove(to_position::<C>(stored_position)) })
    }

    /// Keeps only the values `f` returns `true` for, and removes the rest.
    /// `f` sees every value once.
    pub fn retain<F: FnMut(Key<MapKeyConfig<C>>, &mut T) -> bool>(&mut self, mut f: F) {
        let mut position = 0;
        while position < self.pairs.len() {
            let (keys, values) = self.pairs.slices_mut();
            // SAFETY: `position` is below the number of values, and the pair
            // storage holds a key for each value.
            let (key, value) = unsafe {
                (
                    *keys.get_unchecked(position),
                    values.get_unchecked_mut(position),
                )
            };
            if !f(key, value) {
                // SAFETY: `key` is the key of the value at `position`, so its
                // slot exists and holds that value, and `position` is below the
                // number of values.
                unsafe {
                    self.slot_unchecked_mut(key)
                        .replace_odd_unchecked(Even::ZERO, ());
                    drop(self.swap_remove(position));
                }
                // The last value moved to `position`, so `position` is checked
                // again.
                continue;
            }
            position += 1;
        }
    }

    /// Returns an iterator over the keys and references to the values, in the
    /// order the values are stored.
    #[inline]
    pub fn iter(&self) -> DenseIter<'_, T, MapKeyConfig<C>> {
        let (keys, values) = self.pairs.slices();
        DenseIter {
            keys: keys.iter(),
            values: values.iter(),
        }
    }

    /// Returns an iterator over the keys, in the order the values are stored.
    #[inline]
    pub fn keys(&self) -> DenseKeys<'_, MapKeyConfig<C>> {
        DenseKeys(self.pairs.first_slice().iter())
    }

    /// Returns an iterator over the keys and mutable references to the
    /// values, in the order the values are stored.
    #[inline]
    pub fn iter_mut(&mut self) -> DenseIterMut<'_, T, MapKeyConfig<C>> {
        let (keys, values) = self.pairs.slices_mut();
        DenseIterMut {
            keys: keys.iter(),
            values: values.iter_mut(),
        }
    }

    /// Removes every value, and returns an iterator over them and their keys,
    /// from the last value to the first. The slots stay, and dropping the
    /// iterator removes the values it has not reached.
    #[inline]
    pub fn drain(&mut self) -> DenseSecondaryDrain<'_, T, C> {
        DenseSecondaryDrain { map: self }
    }

    /// Takes the value at `position` and its key out of the pair storage by
    /// moving the last key and value into their place, and points the slot of
    /// the moved value at `position`.
    ///
    /// # Safety
    ///
    /// `position` must be below the number of values, and the caller must
    /// already have emptied the slot of the value at `position`.
    unsafe fn swap_remove(&mut self, position: usize) -> T {
        debug_assert!(position < self.pairs.len());
        let last = self.pairs.len() - 1;
        // SAFETY: the value at `position` exists, so the pair storage is not
        // empty, and `PairStorage` promises that `pop` takes out the last pair.
        let (last_key, last_value) = unsafe { self.pairs.pop().unwrap_unchecked() };
        if position == last {
            return last_value;
        }
        // SAFETY: `position` is below `last`, which is now the number of
        // values. `last_key` is the key of a value the map holds, so its slot
        // exists and holds that value.
        unsafe {
            *self.slot_unchecked_mut(last_key).get_odd_unchecked_mut() = to_stored::<C>(position);
            let (keys, values) = self.pairs.slices_mut();
            *keys.get_unchecked_mut(position) = last_key;
            core::mem::replace(values.get_unchecked_mut(position), last_value)
        }
    }
}

impl<T, C: DenseSecondaryMapConfig> Default for DenseSecondaryMap<T, C> {
    #[inline]
    fn default() -> Self {
        Self::new_with_config()
    }
}

impl<T, C: DenseSecondaryMapConfig> Clone for DenseSecondaryMap<T, C>
where
    Slots<C>: Clone,
    Pairs<T, C>: Clone,
{
    /// The clone has the same slots, values and keys, so every key of the
    /// original works on it.
    fn clone(&self) -> Self {
        let pairs = self.pairs.clone();
        Self {
            slots: self.slots.clone(),
            pairs,
        }
    }

    /// Clones the slots, keys and values of `source` with the `clone_from` of
    /// this map's storages. If a value's `clone` panics, this map is left
    /// empty.
    fn clone_from(&mut self, source: &Self) {
        let slots = ClearOnUnwind(&mut self.slots);
        let pairs = ClearPairsOnUnwind(&mut self.pairs);
        pairs.0.clone_from(&source.pairs);
        slots.0.clone_from(&source.slots);
        core::mem::forget(pairs);
        core::mem::forget(slots);
    }
}

impl<T: fmt::Debug, C: DenseSecondaryMapConfig> fmt::Debug for DenseSecondaryMap<T, C> {
    /// Lists every key with its value, in index order.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let values = self.pairs.second_slice();
        let entries = self
            .slots
            .as_slice()
            .iter()
            .enumerate()
            .filter_map(|(slot_index, slot)| {
                let (generation, stored_position) = occupied::<C>(slot)?;
                // SAFETY: the slot at `slot_index` holds a value under
                // `generation`, so the two fit the key, and the position the
                // slot stores is below the number of values.
                Some(unsafe {
                    (
                        key_from_parts_unchecked::<C>(slot_index, generation),
                        values.get_unchecked(to_position::<C>(stored_position)),
                    )
                })
            });
        f.debug_map().entries(entries).finish()
    }
}

impl<T, C: DenseSecondaryMapConfig> Index<Key<MapKeyConfig<C>>> for DenseSecondaryMap<T, C> {
    type Output = T;

    /// Returns a reference to the value stored under `key`.
    ///
    /// # Panics
    ///
    /// Panics if no value is stored under `key`. Use [`get`](Self::get) to get
    /// `None` instead.
    #[inline]
    fn index(&self, key: Key<MapKeyConfig<C>>) -> &T {
        self.get(key).expect("invalid DenseSecondaryMap key")
    }
}

impl<T, C: DenseSecondaryMapConfig> IndexMut<Key<MapKeyConfig<C>>> for DenseSecondaryMap<T, C> {
    /// Returns a mutable reference to the value stored under `key`.
    ///
    /// # Panics
    ///
    /// Panics if no value is stored under `key`. Use
    /// [`get_mut`](Self::get_mut) to get `None` instead.
    #[inline]
    fn index_mut(&mut self, key: Key<MapKeyConfig<C>>) -> &mut T {
        self.get_mut(key).expect("invalid DenseSecondaryMap key")
    }
}

impl<T, C: DenseSecondaryMapConfig> IntoIterator for DenseSecondaryMap<T, C>
where
    Pairs<T, C>: IntoIterator<Item = (Key<MapKeyConfig<C>>, T)>,
{
    type Item = (Key<MapKeyConfig<C>>, T);
    type IntoIter = DenseIntoIter<Pairs<T, C>>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        DenseIntoIter {
            remaining: self.pairs.len(),
            pairs: self.pairs.into_iter(),
        }
    }
}

impl<'a, T, C: DenseSecondaryMapConfig> IntoIterator for &'a DenseSecondaryMap<T, C> {
    type Item = (Key<MapKeyConfig<C>>, &'a T);
    type IntoIter = DenseIter<'a, T, MapKeyConfig<C>>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T, C: DenseSecondaryMapConfig> IntoIterator for &'a mut DenseSecondaryMap<T, C> {
    type Item = (Key<MapKeyConfig<C>>, &'a mut T);
    type IntoIter = DenseIterMut<'a, T, MapKeyConfig<C>>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

/// Iterator that takes each value out, with its key, from the last value to
/// the first. It is created using [`DenseSecondaryMap::drain`].
pub struct DenseSecondaryDrain<'a, T, C: DenseSecondaryMapConfig> {
    map: &'a mut DenseSecondaryMap<T, C>,
}

impl<T, C: DenseSecondaryMapConfig> Iterator for DenseSecondaryDrain<'_, T, C> {
    type Item = (Key<MapKeyConfig<C>>, T);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let (key, value) = self.map.pairs.pop()?;
        // SAFETY: `key` is the key of the value just taken out, so its slot
        // exists and still holds that value. The slot is emptied like the slot
        // of a removed value.
        unsafe {
            self.map
                .slot_unchecked_mut(key)
                .replace_odd_unchecked(Even::ZERO, ())
        };
        Some((key, value))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.map.len();
        (len, Some(len))
    }
}

impl<T, C: DenseSecondaryMapConfig> ExactSizeIterator for DenseSecondaryDrain<'_, T, C> {}

impl<T, C: DenseSecondaryMapConfig> FusedIterator for DenseSecondaryDrain<'_, T, C> {}

impl<T, C: DenseSecondaryMapConfig> Drop for DenseSecondaryDrain<'_, T, C> {
    fn drop(&mut self) {
        for _ in self.by_ref() {}
    }
}

/// `DenseSecondaryMapRawParts` holds the fields of a [`DenseSecondaryMap`].
/// [`into_raw_parts`](DenseSecondaryMap::into_raw_parts) takes a map apart into
/// these fields, and [`from_raw_parts`](DenseSecondaryMap::from_raw_parts)
/// builds a map from them.
///
/// The parts given to `from_raw_parts` must follow every rule below, and the
/// parts that `into_raw_parts` returns always do.
///
/// Editing or replacing a value in the second slice of `pairs` never breaks a
/// rule, and neither does changing the capacity of `slots` or `pairs`.
///
/// # Rules
///
/// - No slot sits at a position above the largest index the map's keys can
///   hold, or at the largest value of the index type.
/// - A slot that holds a value has a generation no larger than the largest
///   one the map's keys can hold, and a slot that holds no value has
///   generation zero.
/// - `pairs` holds one pair for each slot that holds a value.
/// - Each slot that holds a value stores the position of its value in the
///   second slice of `pairs`, and no two of those slots store the same
///   position.
/// - The key at each position in the first slice of `pairs` is the key of
///   the value at the same position in the second slice. The key has the
///   index of the slot that stores that position, and the generation of that
///   slot.
///
/// # Examples
///
/// ```
/// use gen_map::{DenseSecondaryMap, DenseSecondaryMapRawParts, Even, GenMap, PairVec, Slot};
///
/// let mut people = GenMap::new();
/// let alice = people.insert("Alice");
/// let bob = people.insert("Bob");
///
/// // Only Bob has an age, so the slot at Alice's index holds no value.
/// let mut pairs = PairVec::new();
/// pairs.push(bob, 25);
/// let parts = DenseSecondaryMapRawParts {
///     slots: [Slot::new_even(Even::ZERO, ()), Slot::new_odd(bob.generation(), 0)]
///         .into_iter()
///         .collect(),
///     pairs,
/// };
/// // SAFETY: both slots sit at indices that a `GenMap` hands out, and the slot
/// // without a value has generation zero. Bob's slot stores position zero,
/// // where `pairs` holds his key and his age.
/// let ages: DenseSecondaryMap<u32> = unsafe { DenseSecondaryMap::from_raw_parts(parts) };
/// assert_eq!(ages.get(alice), None);
/// assert_eq!(ages[bob], 25);
/// ```
pub struct DenseSecondaryMapRawParts<
    T,
    #[cfg(feature = "alloc")] C: DenseSecondaryMapConfig = DefaultMapConfig,
    #[cfg(not(feature = "alloc"))] C: DenseSecondaryMapConfig,
> {
    // These fields are in the same order as the fields of `DenseSecondaryMap`,
    // so the two structs can be read side by side. `into_raw_parts` and
    // `from_raw_parts` list every field of both, so the compiler catches a
    // field that only one struct has, but not a change in order.
    /// The map keeps its slots in this storage, which the map's config picks.
    /// A slot's index is its position in the storage, and a slot that holds a
    /// value stores the position of that value in `pairs`.
    pub slots: <C as DenseSecondaryMapConfig>::SlotStorage<DenseSecondaryMapSlot<C>>,
    /// The map keeps its values one after another in the second slice of
    /// this storage, and the key of each value at the same position in the
    /// first slice.
    pub pairs: <C as DenseSecondaryMapConfig>::PairStorage<Key<MapKeyConfig<C>>, T>,
}

impl<T, C: DenseSecondaryMapConfig> DenseSecondaryMap<T, C> {
    /// Takes the map apart into its fields.
    /// [`from_raw_parts`](Self::from_raw_parts) builds a map from them again,
    /// and [`DenseSecondaryMapRawParts`] lists the rules they follow.
    #[inline]
    pub fn into_raw_parts(self) -> DenseSecondaryMapRawParts<T, C> {
        let Self { slots, pairs } = self;
        DenseSecondaryMapRawParts { slots, pairs }
    }

    /// Builds a map from the fields that
    /// [`into_raw_parts`](Self::into_raw_parts) takes a map apart into.
    ///
    /// # Safety
    ///
    /// `parts` must follow every rule listed on [`DenseSecondaryMapRawParts`].
    /// The map's methods rely on those rules, and parts that break one can
    /// make them cause undefined behavior.
    #[inline]
    #[must_use]
    pub unsafe fn from_raw_parts(parts: DenseSecondaryMapRawParts<T, C>) -> Self {
        let DenseSecondaryMapRawParts { slots, pairs } = parts;
        Self { slots, pairs }
    }
}
