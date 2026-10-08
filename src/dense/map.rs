#[cfg(feature = "alloc")]
use crate::config::DefaultMapConfig;
use crate::config::{DenseGenMapConfig, KeyConfig, MapConfig};
use crate::error::{
    check_disjoint_idxs, check_disjoint_keys, DenseError, FullError, GetDisjointMutAtError,
    GetDisjointMutError, InsertError, InsertWithError,
};
use crate::key::parity::{Even, Odd};
use crate::key::piece::KeyPiece;
use crate::key::Key;
use crate::map::{
    detached_generation, detached_slot, freed_parts, index_of as idx_of_slot, max_slot_idx,
    no_slot, position_of as slot_index_of, slot_key, MapGen, MapIdx, MapKeyConfig,
};
use crate::slot::{ParityRef, Slot};
use crate::storage::pair::{PairStorage, ReservePairStorage};
use crate::storage::{clone_storage, ReserveStorage, SliceStorage};
use core::fmt;
use core::iter::FusedIterator;
use core::ops::{Index, IndexMut};

/// The storage a config gives the map for its slots.
type Slots<C> = <C as DenseGenMapConfig>::SlotStorage<DenseMapSlot<C>>;

/// The storage a config gives the map for its keys and values.
type Pairs<T, C> = <C as DenseGenMapConfig>::PairStorage<Key<MapKeyConfig<C>>, T>;

/// A [`DenseGenMap`] keeps one [`Slot`] of this type for each index. While the
/// slot holds a value, its generation is odd and it stores the position of that
/// value in the pair storage. While it holds no value, its generation is even
/// and it stores an index instead. A free slot stores the index of the next
/// free slot, or the largest value of the index type if it is the last free
/// slot. A detached slot stores its own index, which is how
/// [`DenseGenMap::reattach`] tells a detached slot apart from a free or retired
/// one. When the map retires a slot, it stores the largest value of the index
/// type there, so the slot never looks detached. [`Slot::as_parity`] checks the
/// parity of the generation and returns either the position in
/// [`ParityRef::Odd`] or the index in [`ParityRef::Even`].
pub type DenseMapSlot<C> = Slot<MapGen<C>, MapIdx<C>, MapIdx<C>>;

/// The error a `DenseGenMap<T, C>` gives when one of its storages cannot make
/// room for another value.
pub type DenseStorageError<T, C> = DenseError<
    <<C as DenseGenMapConfig>::SlotStorage<DenseMapSlot<C>> as SliceStorage>::Error,
    <<C as DenseGenMapConfig>::PairStorage<Key<MapKeyConfig<C>>, T> as PairStorage>::Error,
>;

/// The slot the next inserted value gets, worked out before anything is
/// written.
struct Target<C: MapConfig> {
    /// The index of the slot.
    idx: MapIdx<C>,
    /// The generation the new key gets.
    generation: Odd<MapGen<C>>,
    /// `true` if the slot came off the free list, and `false` if it has to be
    /// pushed onto the slot storage.
    from_free_list: bool,
}

impl<C: MapConfig> Target<C> {
    /// The key that a value gets when it is put at this target.
    #[inline]
    fn key(&self) -> Key<MapKeyConfig<C>> {
        // SAFETY: a `Target` only ever comes from `next_target`, which picks an
        // index and a generation that a key can hold.
        Key::from_repr(unsafe {
            <MapKeyConfig<C> as KeyConfig>::pack_unchecked(self.idx, self.generation)
        })
    }
}

/// Converts the position a slot stores back to a `usize`.
///
/// # Safety
///
/// `stored_position` must be a position the map stored in a slot. Every such
/// position was made from a `usize` by [`to_stored`], so it fits in one.
#[inline]
pub(crate) unsafe fn to_position<C: MapConfig>(stored_position: MapIdx<C>) -> usize {
    debug_assert!(stored_position.into_usize().is_some());
    // SAFETY: the caller promises that `stored_position` was made from a
    // `usize`.
    unsafe { stored_position.into_usize_unchecked() }
}

/// Converts the position of a value to the type a slot stores it as.
///
/// # Safety
///
/// `position` must be at most the number of values. A map never has more
/// values than slots, nor more slots than the largest value of the index type,
/// so such a position fits in the index type.
#[inline]
pub(crate) unsafe fn to_stored<C: MapConfig>(position: usize) -> MapIdx<C> {
    debug_assert!(MapIdx::<C>::from_usize(position).is_some());
    // SAFETY: the caller promises that `position` is at most the number of
    // values, which fits as explained above.
    unsafe { MapIdx::<C>::from_usize_unchecked(position) }
}

/// Panics with the message `insert` gives for a full map. It is kept out of
/// line so that the insert paths stay small.
#[cold]
#[inline(never)]
fn panic_full<E: fmt::Debug>(full: FullError<E>, slots_len: usize) -> ! {
    match full {
        FullError::IndexExhausted => panic!(
            "DenseGenMap is full, its keys cannot address more than {} slots",
            slots_len
        ),
        FullError::StorageFull(error) => panic!(
            "DenseGenMap is full, one of its storages cannot make room for another value: {:?}",
            error
        ),
    }
}

/// The slot the next insert would use, handed out by
/// [`DenseGenMap::vacant_entry`]. No slot is written until
/// [`insert`](Self::insert) is called, so dropping the entry inserts nothing.
pub struct DenseVacantEntry<'a, T, C: DenseGenMapConfig> {
    map: &'a mut DenseGenMap<T, C>,
    target: Target<C>,
}

impl<T, C: DenseGenMapConfig> DenseVacantEntry<'_, T, C> {
    /// The key the value will get. It matches nothing until
    /// [`insert`](Self::insert) is called.
    #[inline]
    pub fn key(&self) -> Key<MapKeyConfig<C>> {
        self.target.key()
    }

    /// Puts `value` in the map and returns its key, the same one
    /// [`key`](Self::key) returns.
    #[inline(always)]
    pub fn insert(self, value: T) -> Key<MapKeyConfig<C>> {
        // SAFETY: `target` came from `next_target`, and this entry has held
        // `&mut` on the map since, so nothing has touched it.
        unsafe { self.map.fill(self.target, value) }
    }
}

impl<T, C: DenseGenMapConfig> fmt::Debug for DenseVacantEntry<'_, T, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DenseVacantEntry")
            .field("key", &self.key())
            .finish()
    }
}

/// A generational map that holds values of type `T`, one after another with
/// no gaps, and is configured by `C`.
///
/// Its slots, and so its keys, work the same way as those of a
/// [`GenMap`](crate::GenMap) with the same config. The difference is where the
/// values live. A `GenMap` keeps each value in its slot, while a `DenseGenMap`
/// keeps its values one after another in a [`PairStorage`], and each slot
/// only stores the position of its value. Iterating over the values is then
/// as fast as iterating over a slice, and a lookup takes one more step.
///
/// The pair storage also holds the key of each value, at the same position as
/// the value. Removing a value moves the last value into its place, and the
/// map reads the moved value's key to point that value's slot at the new
/// position.
///
/// With the `alloc` feature, `C` defaults to [`DefaultMapConfig`]. To use
/// your own config, implement [`MapConfig`] and [`DenseGenMapConfig`] for it.
///
/// # Examples
///
/// ```
/// use gen_map::DenseGenMap;
///
/// let mut map = DenseGenMap::new();
/// let a = map.insert("a");
/// let b = map.insert("b");
/// let c = map.insert("c");
///
/// // "c" moves into the place of "a".
/// assert_eq!(map.remove(a), Some("a"));
/// assert!(map.get(a).is_none());
/// assert_eq!(map.values().copied().collect::<Vec<_>>(), ["c", "b"]);
/// assert_eq!(map[c], "c");
/// assert_eq!(map[b], "b");
/// ```
pub struct DenseGenMap<
    T,
    #[cfg(feature = "alloc")] C: DenseGenMapConfig = DefaultMapConfig,
    #[cfg(not(feature = "alloc"))] C: DenseGenMapConfig,
> {
    /// A slot for every index the map has used. A slot that holds a value
    /// stores the position of the value in `pairs`, and every position below
    /// the number of values is stored by exactly one slot. A free slot links to
    /// the next free slot, as a slot of a `GenMap` does.
    slots: Slots<C>,
    /// The index of the first free slot, or `no_slot` if no slot is free.
    next_free: MapIdx<C>,
    /// The values one after another in the second slice, and the key of each
    /// value at the same position in the first slice.
    pairs: Pairs<T, C>,
}

#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
impl<T> DenseGenMap<T> {
    /// Creates an empty map with the [`DefaultMapConfig`].
    ///
    /// This method only exists for the default config, so that
    /// `DenseGenMap::new()` compiles without a type annotation. Use
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

/// These methods need slot and pair storages that can grow on request, so a
/// map whose storages have a fixed capacity does not have them.
impl<T, C: DenseGenMapConfig> DenseGenMap<T, C>
where
    <C as DenseGenMapConfig>::SlotStorage<DenseMapSlot<C>>: ReserveStorage,
    Pairs<T, C>: ReservePairStorage,
{
    /// Creates an empty map with config `C` and room for `capacity` values,
    /// slots and keys.
    #[inline]
    #[must_use]
    pub fn with_capacity_and_config(capacity: usize) -> Self {
        Self {
            slots: <Slots<C> as SliceStorage>::with_capacity(capacity),
            next_free: no_slot::<C>(),
            pairs: <Pairs<T, C> as PairStorage>::with_capacity(capacity),
        }
    }

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
            panic!("DenseGenMap cannot make room for {additional} more values: {error:?}");
        }
    }

    /// The fallible form of [`reserve`](Self::reserve). After it returns `Ok`,
    /// the next `additional` inserts cannot fail for lack of storage.
    ///
    /// # Errors
    ///
    /// Returns the error of the first storage that cannot make the room. The
    /// map tries the slot storage first, then the pair storage.
    #[inline]
    pub fn try_reserve(&mut self, additional: usize) -> Result<(), DenseStorageError<T, C>> {
        self.slots
            .ensure_room(additional)
            .map_err(DenseError::Slots)?;
        self.pairs
            .ensure_room(additional)
            .map_err(DenseError::Pairs)
    }
}

impl<T, C: DenseGenMapConfig> DenseGenMap<T, C> {
    /// Creates an empty map with config `C`.
    #[inline]
    #[must_use]
    pub fn new_with_config() -> Self {
        Self {
            slots: <Slots<C> as SliceStorage>::empty(),
            next_free: no_slot::<C>(),
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

    /// The number of values in the map.
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

    /// Returns `true` if the map has a value for `key`.
    #[inline]
    pub fn contains_key(&self, key: Key<MapKeyConfig<C>>) -> bool {
        self.stored_position(key).is_some()
    }

    /// Returns a reference to the value corresponding to `key`.
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

    /// Returns a mutable reference to the value corresponding to `key`.
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

    /// Returns a reference to the value corresponding to `key`, like
    /// [`get`](Self::get), but without checking that the key's slot exists or
    /// that the slot's generation matches the key's.
    ///
    /// # Safety
    ///
    /// The map must have a value for `key`, meaning
    /// [`contains_key`](Self::contains_key) returns `true` for it.
    #[inline]
    pub unsafe fn get_unchecked(&self, key: Key<MapKeyConfig<C>>) -> &T {
        debug_assert!(self.contains_key(key));
        // SAFETY: the caller promises that the map has a value for `key`, so
        // the key's index is the position of a slot that holds a value, and the
        // position that slot stores is below the number of values.
        unsafe {
            let slot = self
                .slots
                .as_slice()
                .get_unchecked(slot_index_of::<C>(key.idx()));
            let position = to_position::<C>(*slot.get_odd_unchecked());
            self.pairs.second_slice().get_unchecked(position)
        }
    }

    /// Returns a mutable reference to the value corresponding to `key`, like
    /// [`get_mut`](Self::get_mut), but without checking that the key's slot
    /// exists or that the slot's generation matches the key's.
    ///
    /// # Safety
    ///
    /// The map must have a value for `key`, meaning
    /// [`contains_key`](Self::contains_key) returns `true` for it.
    #[inline]
    pub unsafe fn get_unchecked_mut(&mut self, key: Key<MapKeyConfig<C>>) -> &mut T {
        debug_assert!(self.contains_key(key));
        // SAFETY: the same as in `get_unchecked`.
        unsafe {
            let slot = self
                .slots
                .as_slice()
                .get_unchecked(slot_index_of::<C>(key.idx()));
            let position = to_position::<C>(*slot.get_odd_unchecked());
            self.pairs.second_slice_mut().get_unchecked_mut(position)
        }
    }

    /// Returns the key of the value in the slot at `idx`, or `None` if there
    /// is no slot at `idx` or the slot holds no value. A slot holds no value
    /// while it is vacant, detached or retired.
    #[inline]
    pub fn key_at(&self, idx: MapIdx<C>) -> Option<Key<MapKeyConfig<C>>> {
        match self.slots.as_slice().get(idx.into_usize()?)?.as_parity() {
            // SAFETY: `idx` is the index of the slot just read.
            ParityRef::Odd(&generation, _) => Some(unsafe { slot_key::<C>(idx, generation) }),
            ParityRef::Even(..) => None,
        }
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
        // SAFETY: the caller promises a slot at `idx` that holds a value, so
        // `idx` is the position of a slot in bounds, and that slot's generation
        // is odd.
        unsafe {
            let slot = self.slots.as_slice().get_unchecked(slot_index_of::<C>(idx));
            slot_key::<C>(idx, Odd::new_unchecked(slot.generation()))
        }
    }

    /// Returns the key and a reference to the value in the slot at `idx`, or
    /// `None` if there is no slot at `idx` or the slot holds no value. A slot
    /// holds no value while it is vacant, detached or retired.
    #[inline]
    pub fn get_at(&self, idx: MapIdx<C>) -> Option<(Key<MapKeyConfig<C>>, &T)> {
        match self.slots.as_slice().get(idx.into_usize()?)?.as_parity() {
            ParityRef::Odd(&generation, &stored_position) => {
                // SAFETY: `idx` is the index of the slot just read, and the
                // position the slot stores is below the number of values.
                Some(unsafe {
                    (
                        slot_key::<C>(idx, generation),
                        self.pairs
                            .second_slice()
                            .get_unchecked(to_position::<C>(stored_position)),
                    )
                })
            }
            ParityRef::Even(..) => None,
        }
    }

    /// Returns the key and a mutable reference to the value in the slot at
    /// `idx`, or `None` if there is no slot at `idx` or the slot holds no
    /// value. See [`get_at`](Self::get_at).
    #[inline]
    pub fn get_at_mut(&mut self, idx: MapIdx<C>) -> Option<(Key<MapKeyConfig<C>>, &mut T)> {
        let (generation, stored_position) =
            match self.slots.as_slice().get(idx.into_usize()?)?.as_parity() {
                ParityRef::Odd(&generation, &stored_position) => (generation, stored_position),
                ParityRef::Even(..) => return None,
            };
        // SAFETY: the same as in `get_at`.
        Some(unsafe {
            (
                slot_key::<C>(idx, generation),
                self.pairs
                    .second_slice_mut()
                    .get_unchecked_mut(to_position::<C>(stored_position)),
            )
        })
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
        // SAFETY: the caller promises a slot at `idx` that holds a value, so
        // `idx` is the position of a slot in bounds, that slot's generation is
        // odd, and the position it stores is below the number of values.
        unsafe {
            let slot = self.slots.as_slice().get_unchecked(slot_index_of::<C>(idx));
            let key = slot_key::<C>(idx, Odd::new_unchecked(slot.generation()));
            let position = to_position::<C>(*slot.get_odd_unchecked());
            (key, self.pairs.second_slice().get_unchecked(position))
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
            let slot = self.slots.as_slice().get_unchecked(slot_index_of::<C>(idx));
            let key = slot_key::<C>(idx, Odd::new_unchecked(slot.generation()));
            let position = to_position::<C>(*slot.get_odd_unchecked());
            (
                key,
                self.pairs.second_slice_mut().get_unchecked_mut(position),
            )
        }
    }

    /// Returns the generation of the slot at `idx`, or `None` if there is no
    /// slot at `idx`. It works the same way as
    /// [`GenMap::generation_at`](crate::GenMap::generation_at).
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
    /// [`generation_at`](Self::generation_at) returns `Some` for it. The slot
    /// does not have to hold a value.
    #[inline]
    pub unsafe fn generation_at_unchecked(&self, idx: MapIdx<C>) -> MapGen<C> {
        debug_assert!(self.generation_at(idx).is_some());
        // SAFETY: the caller promises a slot at `idx`, so `idx` is the position
        // of a slot in bounds.
        unsafe {
            self.slots
                .as_slice()
                .get_unchecked(slot_index_of::<C>(idx))
                .generation()
        }
    }

    /// Returns the key and a mutable reference to the value in the slot at each
    /// index, all at once. It checks the indices the same way
    /// [`GenMap::get_disjoint_mut_at`](crate::GenMap::get_disjoint_mut_at)
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
        // value and that the indices are different. So each index is the
        // position of a slot in bounds with an odd generation, the slots are
        // different, and so are the positions they store.
        unsafe {
            let slots = self.slots.as_slice();
            let stored_positions = idxs.map(|idx| {
                let slot = slots.get_unchecked(slot_index_of::<C>(idx));
                let key = slot_key::<C>(idx, Odd::new_unchecked(slot.generation()));
                (key, *slot.get_odd_unchecked())
            });
            self.values_at(stored_positions)
        }
    }

    /// Returns a mutable reference to the value of each key, all at once. It
    /// checks the keys the same way
    /// [`GenMap::get_disjoint_mut`](crate::GenMap::get_disjoint_mut) does.
    ///
    /// # Errors
    ///
    /// Returns [`GetDisjointMutError::InvalidKey`] if the map has no value
    /// for one of the keys, and [`GetDisjointMutError::OverlappingKeys`] if
    /// two keys point at the same slot.
    ///
    /// # Examples
    ///
    /// ```
    /// use gen_map::DenseGenMap;
    ///
    /// let mut map = DenseGenMap::new();
    /// let a = map.insert(1);
    /// let b = map.insert(2);
    ///
    /// let [x, y] = map.get_disjoint_mut([a, b]).unwrap();
    /// core::mem::swap(x, y);
    /// assert_eq!(map[a], 2);
    /// assert_eq!(map[b], 1);
    /// ```
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

    /// Returns a mutable reference to the value of each key, like
    /// [`get_disjoint_mut`](Self::get_disjoint_mut), but without any of its
    /// checks.
    ///
    /// # Safety
    ///
    /// The map must have a value for every key, meaning
    /// [`contains_key`](Self::contains_key) returns `true` for each of them,
    /// and no two keys may point at the same slot.
    #[inline]
    pub unsafe fn get_disjoint_mut_unchecked<const N: usize>(
        &mut self,
        keys: [Key<MapKeyConfig<C>>; N],
    ) -> [&mut T; N] {
        debug_assert!(check_disjoint_keys(&keys, |key| self.contains_key(key)).is_ok());
        // SAFETY: the caller promises that every key has a value and that no
        // two keys point at the same slot. So each key's index is the position
        // of a slot in bounds that holds a value, and the positions those slots
        // store are different.
        unsafe {
            let slots = self.slots.as_slice();
            let stored_positions = keys.map(|key| {
                let slot = slots.get_unchecked(slot_index_of::<C>(key.idx()));
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

    /// Inserts a value and returns its key.
    ///
    /// # Panics
    ///
    /// Panics if the map is full, meaning either none of the slots are free and
    /// its keys have no index left for a new slot, or one of its storages
    /// cannot make room for the value. Use [`try_insert`](Self::try_insert) to
    /// get the value back instead.
    pub fn insert(&mut self, value: T) -> Key<MapKeyConfig<C>> {
        let target = match self.next_target() {
            Ok(target) => target,
            Err(full) => panic_full(full, self.slots.len()),
        };
        // SAFETY: `target` came from `next_target`, and nothing has touched
        // the map since.
        unsafe { self.fill(target, value) }
    }

    /// Inserts a value and returns its key, or hands the value back if the
    /// map is full.
    ///
    /// # Errors
    ///
    /// Returns [`InsertError::IndexExhausted`] if none of the slots are free
    /// and the map's keys have no index left for a new one, and
    /// [`InsertError::StorageFull`] if one of the storages cannot make room.
    /// The [`DenseError`] in it says which storage that was.
    #[allow(clippy::type_complexity)]
    pub fn try_insert(
        &mut self,
        value: T,
    ) -> Result<Key<MapKeyConfig<C>>, InsertError<T, DenseStorageError<T, C>>> {
        match self.next_target() {
            // SAFETY: `target` came from `next_target`, and nothing has
            // touched the map since.
            Ok(target) => Ok(unsafe { self.fill(target, value) }),
            Err(FullError::IndexExhausted) => Err(InsertError::IndexExhausted(value)),
            Err(FullError::StorageFull(error)) => Err(InsertError::StorageFull(value, error)),
        }
    }

    /// Calls `f` with the key that the new value will get, and inserts the
    /// value it returns. This lets a value hold its own key.
    ///
    /// # Panics
    ///
    /// Panics if the map is full, as [`insert`](Self::insert) does. Use
    /// [`try_insert_with_key`](Self::try_insert_with_key) or
    /// [`vacant_entry`](Self::vacant_entry) to get an error instead.
    pub fn insert_with_key<F>(&mut self, f: F) -> Key<MapKeyConfig<C>>
    where
        F: FnOnce(Key<MapKeyConfig<C>>) -> T,
    {
        let target = match self.next_target() {
            Ok(target) => target,
            Err(full) => panic_full(full, self.slots.len()),
        };
        let value = f(target.key());
        // SAFETY: `target` came from `next_target`, and nothing has touched
        // the map since, because `f` cannot reach the map while this method
        // holds `&mut self`.
        unsafe { self.fill(target, value) }
    }

    /// Like [`insert_with_key`](Self::insert_with_key), but `f` may fail and
    /// a full map is an error rather than a panic. On either error, nothing is
    /// inserted.
    ///
    /// # Errors
    ///
    /// Returns [`InsertWithError::Full`] if the map is full. `f` is not
    /// called in that case. Returns [`InsertWithError::Rejected`] with
    /// whatever `f` returned if `f` fails.
    #[allow(clippy::type_complexity)]
    pub fn try_insert_with_key<F, E>(
        &mut self,
        f: F,
    ) -> Result<Key<MapKeyConfig<C>>, InsertWithError<E, DenseStorageError<T, C>>>
    where
        F: FnOnce(Key<MapKeyConfig<C>>) -> Result<T, E>,
    {
        let target = self.next_target()?;
        let value = f(target.key()).map_err(InsertWithError::Rejected)?;
        // SAFETY: `target` came from `next_target`, and nothing has touched
        // the map since, because `f` cannot reach the map while this method
        // holds `&mut self`.
        Ok(unsafe { self.fill(target, value) })
    }

    /// Hands out the slot the next insert would use, without writing to it.
    /// [`DenseVacantEntry::key`] is the key the value will get, and
    /// [`DenseVacantEntry::insert`] puts the value in the map. Dropping the
    /// entry inserts nothing.
    ///
    /// # Errors
    ///
    /// Returns [`FullError::IndexExhausted`] if none of the slots are free
    /// and the map's keys have no index left for a new one, and
    /// [`FullError::StorageFull`] if one of the storages cannot make room.
    #[inline]
    #[allow(clippy::type_complexity)]
    pub fn vacant_entry(
        &mut self,
    ) -> Result<DenseVacantEntry<'_, T, C>, FullError<DenseStorageError<T, C>>> {
        let target = self.next_target()?;
        Ok(DenseVacantEntry { map: self, target })
    }

    /// Puts a value back under a key whose value [`detach`](Self::detach)
    /// took out. Afterwards, the map has a value for the key again, at the end
    /// of the values.
    ///
    /// # Errors
    ///
    /// Hands `value` back, and leaves the map as it was, if the key was not
    /// detached, if its slot has since been freed by
    /// [`release`](Self::release) or removed by [`reset`](Self::reset), or if
    /// the pair storage cannot make room for it.
    #[inline]
    pub fn reattach(&mut self, key: Key<MapKeyConfig<C>>, value: T) -> Result<(), T> {
        let Some(slot) = detached_slot::<_, C>(self.slots.as_mut_slice(), key) else {
            return Err(value);
        };
        if self.pairs.ensure_room(1).is_err() {
            return Err(value);
        }
        // SAFETY: `self.pairs.len()` is the number of values.
        let stored_position = unsafe { to_stored::<C>(self.pairs.len()) };
        // SAFETY: a detached slot's generation is even.
        unsafe { slot.replace_even_unchecked(key.generation(), stored_position) };
        // SAFETY: `ensure_room(1)` returned `Ok` above, and no other `&mut`
        // method of the pair storage has run since, so `PairStorage` promises
        // that this push succeeds.
        unsafe { self.pairs.push_unchecked(key, value) };
        Ok(())
    }

    /// Frees the slot of a key whose value [`detach`](Self::detach) took out,
    /// without putting a value back. It works the same way as
    /// [`GenMap::release`](crate::GenMap::release). Returns `false`, and leaves
    /// the map as it was, if the key is not detached.
    #[inline]
    pub fn release(&mut self, key: Key<MapKeyConfig<C>>) -> bool {
        let Some(slot) = detached_slot::<_, C>(self.slots.as_mut_slice(), key) else {
            return false;
        };
        let (generation, link) = freed_parts::<C>(
            &mut self.next_free,
            key.idx(),
            key.generation(),
            C::WRAP_ON_OVERFLOW,
        );
        slot.set_even(generation, link);
        true
    }

    /// Removes every value. The slots stay, and old keys stop matching just
    /// as they do after [`remove`](Self::remove). A detached slot stays
    /// detached.
    pub fn clear(&mut self) {
        for slot_index in 0..self.slots.len() {
            // SAFETY: `slot_index` is below the number of slots, which freeing
            // a slot does not change.
            let holds_value = unsafe { self.slots.as_slice().get_unchecked(slot_index).is_odd() };
            if holds_value {
                // SAFETY: the slot was just found to hold a value, and
                // `slot_index` is its position.
                unsafe { self.free_slot_at(slot_index) };
            }
        }
        // Freeing a slot drops no value, so the slots already describe an
        // empty map when the pair storage drops the keys and values, and
        // `PairStorage` promises that the pair storage is empty afterwards,
        // even when dropping a value panics.
        self.pairs.clear();
    }

    /// Removes every value and every slot, keeping the allocations. It works
    /// the same way as [`GenMap::reset`](crate::GenMap::reset), so a key from
    /// before the call can match a value inserted after it.
    #[inline]
    pub fn reset(&mut self) {
        self.next_free = no_slot::<C>();
        self.slots.clear();
        self.pairs.clear();
    }

    /// Iterates over references to the values, in the order they are stored.
    #[inline]
    pub fn values(&self) -> DenseValues<'_, T> {
        DenseValues(self.pairs.second_slice().iter())
    }

    /// Iterates over mutable references to the values, in the order they are
    /// stored.
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
    /// use gen_map::DenseGenMap;
    ///
    /// let mut map = DenseGenMap::new();
    /// let a = map.insert(1);
    /// let b = map.insert(2);
    ///
    /// let (keys, values) = map.as_slices();
    /// assert_eq!(keys, [a, b]);
    /// assert_eq!(values, [1, 2]);
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
    /// use gen_map::DenseGenMap;
    ///
    /// let mut map = DenseGenMap::new();
    /// let a = map.insert(1);
    /// let b = map.insert(2);
    ///
    /// let (_, values) = map.as_slices_mut();
    /// values[0] = 10;
    /// assert_eq!((map[a], map[b]), (10, 2));
    /// ```
    #[inline]
    pub fn as_slices_mut(&mut self) -> (&[Key<MapKeyConfig<C>>], &mut [T]) {
        let (keys, values) = self.pairs.slices_mut();
        (keys, values)
    }

    /// The position of the value corresponding to `key`, as its slot stores it,
    /// or `None` if the map has no value for `key`.
    #[inline]
    fn stored_position(&self, key: Key<MapKeyConfig<C>>) -> Option<MapIdx<C>> {
        self.slots
            .as_slice()
            .get(key.idx().into_usize()?)?
            .get_odd(key.generation())
            .copied()
    }

    /// Frees the slot at `slot_index` the way removing its value does, and
    /// returns the position the slot stored. The slot goes on the free list,
    /// unless its generation has run out and the config retires such slots.
    ///
    /// # Safety
    ///
    /// The slot at `slot_index` must hold a value.
    #[inline]
    unsafe fn free_slot_at(&mut self, slot_index: usize) -> MapIdx<C> {
        // SAFETY: the caller promises a slot at `slot_index` that holds a
        // value, so the position is in bounds, the slot's generation is odd,
        // and the position fits in the index type.
        let (slot, generation, idx) = unsafe {
            let slot = self.slots.as_mut_slice().get_unchecked_mut(slot_index);
            let generation = Odd::new_unchecked(slot.generation());
            (slot, generation, idx_of_slot::<C>(slot_index))
        };
        let (next, link) =
            freed_parts::<C>(&mut self.next_free, idx, generation, C::WRAP_ON_OVERFLOW);
        // SAFETY: the slot's generation is odd.
        unsafe { slot.replace_odd_unchecked(next, link) }
    }

    /// Works out which slot the next value gets, without writing any slot, and
    /// makes room for one more pair in the pair storage. When a slot has to be
    /// pushed, this also makes sure there is room for it, so that the pushes in
    /// [`fill`](Self::fill) cannot fail.
    ///
    /// When the keys have no index left for a new slot and the slot storage is
    /// also full, the error is `IndexExhausted`.
    #[inline]
    fn next_target(&mut self) -> Result<Target<C>, FullError<DenseStorageError<T, C>>> {
        let (idx, generation, from_free_list) = if self.next_free != no_slot::<C>() {
            let idx = self.next_free;
            // SAFETY: `idx` is on the free list, so it is the index of a slot
            // that still exists, and that slot holds no value, so its
            // generation is even.
            let generation = unsafe {
                let slot = self.slots.as_slice().get_unchecked(slot_index_of::<C>(idx));
                Even::new_unchecked(slot.generation())
            };
            (idx, generation, true)
        } else {
            let idx = MapIdx::<C>::from_usize(self.slots.len())
                .filter(|idx| *idx <= max_slot_idx::<C>())
                .ok_or(FullError::IndexExhausted)?;
            self.slots
                .ensure_room(1)
                .map_err(|error| FullError::StorageFull(DenseError::Slots(error)))?;
            (idx, Even::ZERO, false)
        };
        self.pairs
            .ensure_room(1)
            .map_err(|error| FullError::StorageFull(DenseError::Pairs(error)))?;
        // A new slot has generation zero, and a free slot has a generation
        // below the largest one a key can hold, so a key can hold the
        // generation after either one.
        let generation = generation.next();
        Ok(Target {
            idx,
            generation,
            from_free_list,
        })
    }

    /// Writes `value` and its key at the end of the pair storage, and the
    /// value's position into the slot `target` picks. Returns the key.
    ///
    /// # Safety
    ///
    /// `target` must have come from [`next_target`](Self::next_target) with
    /// nothing having touched the map since.
    #[inline]
    unsafe fn fill(&mut self, target: Target<C>, value: T) -> Key<MapKeyConfig<C>> {
        let key = target.key();
        // SAFETY: `self.pairs.len()` is the number of values.
        let stored_position = unsafe { to_stored::<C>(self.pairs.len()) };
        // SAFETY: `next_target` got `Ok` from `ensure_room(1)` for this pair,
        // and no other `&mut` method of the pair storage has run since, so
        // `PairStorage` promises that this push succeeds.
        unsafe { self.pairs.push_unchecked(key, value) };
        if target.from_free_list {
            // SAFETY: `target.idx` is the index of the slot that was first on
            // the free list when `next_target` looked, and nothing has changed
            // since, so that slot still exists.
            let slot = unsafe {
                self.slots
                    .as_mut_slice()
                    .get_unchecked_mut(slot_index_of::<C>(target.idx))
            };
            // SAFETY: the slot came off the free list, so it holds no value and
            // its generation is even.
            self.next_free =
                unsafe { slot.replace_even_unchecked(target.generation, stored_position) };
        } else {
            let slot = Slot::new_odd(target.generation, stored_position);
            // SAFETY: `next_target` got `Ok` from `ensure_room(1)` for this
            // slot, and no other `&mut` method of the slot storage has run
            // since, so `SliceStorage` promises that this push succeeds.
            unsafe { self.slots.push_unchecked(slot) };
        }
        key
    }

    /// Removes and returns the value corresponding to `key`, or `None` if
    /// there is none. The last value moves into the place of the removed one.
    #[inline]
    pub fn remove(&mut self, key: Key<MapKeyConfig<C>>) -> Option<T> {
        let slot_index = key.idx().into_usize()?;
        self.slots
            .as_slice()
            .get(slot_index)?
            .get_odd(key.generation())?;
        // SAFETY: the slot at `slot_index` holds a value, since its generation
        // matches the key's.
        let stored_position = unsafe { self.free_slot_at(slot_index) };
        // SAFETY: `stored_position` is the position the key's slot stored,
        // which is below the number of values.
        Some(unsafe { self.swap_remove(to_position::<C>(stored_position)) })
    }

    /// Removes and returns the value corresponding to `key` like
    /// [`remove`](Self::remove), but retires its slot instead of freeing it. It
    /// works the same way as [`GenMap::retire`](crate::GenMap::retire). Returns
    /// `None` if there is no value for `key`.
    #[inline]
    pub fn retire(&mut self, key: Key<MapKeyConfig<C>>) -> Option<T> {
        let slot = self.slots.as_mut_slice().get_mut(key.idx().into_usize()?)?;
        slot.get_odd(key.generation())?;
        // A retired slot has generation zero and `no_slot` as its link. This
        // method does not put the slot on the free list, so no insert can use
        // it again.
        // SAFETY: the slot's generation matched the key's, which is odd.
        let stored_position = unsafe { slot.replace_odd_unchecked(Even::ZERO, no_slot::<C>()) };
        // SAFETY: `stored_position` is the position the key's slot stored,
        // which is below the number of values.
        Some(unsafe { self.swap_remove(to_position::<C>(stored_position)) })
    }

    /// Takes the value out of the map while keeping its slot reserved for
    /// `key`, so that [`reattach`](Self::reattach) can put a value back under
    /// the same key, or [`release`](Self::release) can free the slot. The slot
    /// works the same way as in [`GenMap::detach`](crate::GenMap::detach).
    /// Returns `None` if there is no value for `key`.
    ///
    /// # Examples
    ///
    /// ```
    /// use gen_map::DenseGenMap;
    ///
    /// let mut map = DenseGenMap::new();
    /// let key = map.insert(1);
    ///
    /// let value = map.detach(key).unwrap();
    /// assert!(map.get(key).is_none());
    /// map.reattach(key, value + 10).unwrap();
    /// assert_eq!(map[key], 11);
    /// ```
    #[inline]
    pub fn detach(&mut self, key: Key<MapKeyConfig<C>>) -> Option<T> {
        let slot = self.slots.as_mut_slice().get_mut(key.idx().into_usize()?)?;
        slot.get_odd(key.generation())?;
        let detached = detached_generation::<C>(key.generation());
        // A detached slot has its own index in its `U`. A slot on the free list
        // has another slot's index or `no_slot` in its `U`, and a retired slot
        // has `no_slot`, so neither looks detached.
        // SAFETY: the slot's generation matched the key's, which is odd.
        let stored_position = unsafe { slot.replace_odd_unchecked(detached, key.idx()) };
        // SAFETY: `stored_position` is the position the key's slot stored,
        // which is below the number of values.
        Some(unsafe { self.swap_remove(to_position::<C>(stored_position)) })
    }

    /// Keeps only the values for which `f` returns `true`. `f` may mutate
    /// them, and it sees every value once. If `f` panics, the values it
    /// already rejected stay removed and the rest stay in the map.
    pub fn retain<F>(&mut self, mut f: F)
    where
        F: FnMut(Key<MapKeyConfig<C>>, &mut T) -> bool,
    {
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
            if f(key, value) {
                position += 1;
                continue;
            }
            // SAFETY: `key` is the key of the value at `position`, so its index
            // is the position of a slot that holds that value, and `position`
            // is below the number of values.
            unsafe {
                self.free_slot_at(slot_index_of::<C>(key.idx()));
                drop(self.swap_remove(position));
            }
            // The last value moved to `position`, so `position` is checked
            // again.
        }
    }

    /// Removes every value, yielding each with its key, from the last value
    /// to the first. The slots stay, and dropping the iterator removes the
    /// values it has not yielded yet.
    #[inline]
    pub fn drain(&mut self) -> DenseDrain<'_, T, C> {
        DenseDrain { map: self }
    }

    /// Iterates over every value together with its key, in the order the
    /// values are stored.
    #[inline]
    pub fn iter(&self) -> DenseIter<'_, T, MapKeyConfig<C>> {
        let (keys, values) = self.pairs.slices();
        DenseIter {
            keys: keys.iter(),
            values: values.iter(),
        }
    }

    /// Iterates over every value together with its key, with mutable
    /// references to the values, in the order the values are stored.
    #[inline]
    pub fn iter_mut(&mut self) -> DenseIterMut<'_, T, MapKeyConfig<C>> {
        let (keys, values) = self.pairs.slices_mut();
        DenseIterMut {
            keys: keys.iter(),
            values: values.iter_mut(),
        }
    }

    /// Iterates over the keys, in the order the values are stored.
    #[inline]
    pub fn keys(&self) -> DenseKeys<'_, MapKeyConfig<C>> {
        DenseKeys(self.pairs.first_slice().iter())
    }

    /// Takes the value at `position` and its key out of the pair storage by
    /// moving the last key and value into their place, and points the slot of
    /// the moved value at `position`.
    ///
    /// # Safety
    ///
    /// `position` must be below the number of values, and the caller must
    /// already have freed, retired or detached the slot of the value at
    /// `position`.
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
        // values. `last_key` is the key of a value the map holds, so its index
        // is the position of a slot that holds that value.
        unsafe {
            *self
                .slots
                .as_mut_slice()
                .get_unchecked_mut(slot_index_of::<C>(last_key.idx()))
                .get_odd_unchecked_mut() = to_stored::<C>(position);
            let (keys, values) = self.pairs.slices_mut();
            *keys.get_unchecked_mut(position) = last_key;
            core::mem::replace(values.get_unchecked_mut(position), last_value)
        }
    }
}

impl<T, C: DenseGenMapConfig> Default for DenseGenMap<T, C> {
    #[inline]
    fn default() -> Self {
        Self::new_with_config()
    }
}

impl<T, C: DenseGenMapConfig> Index<Key<MapKeyConfig<C>>> for DenseGenMap<T, C> {
    type Output = T;

    /// Returns a reference to the value corresponding to `key`.
    ///
    /// # Panics
    ///
    /// Panics if the map has no value for `key`. Use [`get`](Self::get) to
    /// get `None` instead.
    #[inline]
    fn index(&self, key: Key<MapKeyConfig<C>>) -> &T {
        self.get(key).expect("invalid DenseGenMap key")
    }
}

impl<T, C: DenseGenMapConfig> IndexMut<Key<MapKeyConfig<C>>> for DenseGenMap<T, C> {
    /// Returns a mutable reference to the value corresponding to `key`.
    ///
    /// # Panics
    ///
    /// Panics if the map has no value for `key`. Use
    /// [`get_mut`](Self::get_mut) to get `None` instead.
    #[inline]
    fn index_mut(&mut self, key: Key<MapKeyConfig<C>>) -> &mut T {
        self.get_mut(key).expect("invalid DenseGenMap key")
    }
}

impl<T: fmt::Debug, C: DenseGenMapConfig> fmt::Debug for DenseGenMap<T, C> {
    /// Lists every key with its value, in slot order.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let values = self.pairs.second_slice();
        let entries = self
            .slots
            .as_slice()
            .iter()
            .enumerate()
            .filter_map(|(slot_index, slot)| match slot.as_parity() {
                ParityRef::Odd(&generation, &stored_position) => {
                    // SAFETY: the slot at `slot_index` holds a value under
                    // `generation`, so its position fits in the index type and
                    // the two fit the key, and the position the slot stores is
                    // below the number of values.
                    Some(unsafe {
                        (
                            slot_key::<C>(idx_of_slot::<C>(slot_index), generation),
                            values.get_unchecked(to_position::<C>(stored_position)),
                        )
                    })
                }
                ParityRef::Even(..) => None,
            });
        f.debug_map().entries(entries).finish()
    }
}

impl<T: Clone, C: DenseGenMapConfig> Clone for DenseGenMap<T, C> {
    /// The clone has the same slots, values and keys, so every key of the
    /// original works on it.
    fn clone(&self) -> Self {
        let pairs = clone_pairs(&self.pairs);
        Self {
            slots: clone_storage(&self.slots),
            next_free: self.next_free,
            pairs,
        }
    }
}

/// Returns a new pair storage of the same type as `pairs` that holds a clone
/// of each of its pairs, in the same order.
#[inline]
pub(crate) fn clone_pairs<P>(pairs: &P) -> P
where
    P: PairStorage,
    P::First: Clone,
    P::Second: Clone,
{
    let mut clone = P::with_capacity(pairs.len());
    let (firsts, seconds) = pairs.slices();
    for (first, second) in firsts.iter().zip(seconds) {
        // SAFETY: `pairs` has the same type as `clone` and holds the pairs
        // being pushed, and only these pushes have run on `clone` since
        // `with_capacity` made it, so `PairStorage` promises that each push
        // succeeds.
        unsafe { clone.push_unchecked(first.clone(), second.clone()) };
    }
    clone
}

impl<T, C: DenseGenMapConfig> IntoIterator for DenseGenMap<T, C>
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

impl<'a, T, C: DenseGenMapConfig> IntoIterator for &'a DenseGenMap<T, C> {
    type Item = (Key<MapKeyConfig<C>>, &'a T);
    type IntoIter = DenseIter<'a, T, MapKeyConfig<C>>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T, C: DenseGenMapConfig> IntoIterator for &'a mut DenseGenMap<T, C> {
    type Item = (Key<MapKeyConfig<C>>, &'a mut T);
    type IntoIter = DenseIterMut<'a, T, MapKeyConfig<C>>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

/// Draining iterator over `(key, value)` pairs, from the last value to the
/// first. It is created using [`DenseGenMap::drain`]. Dropping it removes the
/// values it has not yielded yet.
pub struct DenseDrain<'a, T, C: DenseGenMapConfig> {
    map: &'a mut DenseGenMap<T, C>,
}

impl<T, C: DenseGenMapConfig> Iterator for DenseDrain<'_, T, C> {
    type Item = (Key<MapKeyConfig<C>>, T);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let (key, value) = self.map.pairs.pop()?;
        // SAFETY: `key` is the key of the value just taken out, so its index is
        // the position of a slot that still holds that value. The slot is
        // freed like the slot of a removed value.
        unsafe { self.map.free_slot_at(slot_index_of::<C>(key.idx())) };
        Some((key, value))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.map.len();
        (len, Some(len))
    }
}

impl<T, C: DenseGenMapConfig> ExactSizeIterator for DenseDrain<'_, T, C> {}

impl<T, C: DenseGenMapConfig> FusedIterator for DenseDrain<'_, T, C> {}

impl<T, C: DenseGenMapConfig> Drop for DenseDrain<'_, T, C> {
    fn drop(&mut self) {
        for _ in self.by_ref() {}
    }
}

/// Iterator over `(key, &value)` pairs, in the order the values are stored.
/// It is created using [`DenseGenMap::iter`] or
/// [`DenseSecondaryMap::iter`](crate::DenseSecondaryMap::iter).
pub struct DenseIter<'a, T, K> {
    pub(crate) keys: core::slice::Iter<'a, Key<K>>,
    pub(crate) values: core::slice::Iter<'a, T>,
}

impl<'a, T, K: KeyConfig> Iterator for DenseIter<'a, T, K> {
    type Item = (Key<K>, &'a T);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let key = *self.keys.next()?;
        // SAFETY: the map keeps one key for each value, and the iterator takes
        // out a key and a value together, so a value is left whenever a key is.
        Some((key, unsafe { self.values.next().unwrap_unchecked() }))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.values.size_hint()
    }
}

impl<T, K: KeyConfig> DoubleEndedIterator for DenseIter<'_, T, K> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        let key = *self.keys.next_back()?;
        // SAFETY: the same as in `next`.
        Some((key, unsafe { self.values.next_back().unwrap_unchecked() }))
    }
}

impl<T, K: KeyConfig> ExactSizeIterator for DenseIter<'_, T, K> {}
impl<T, K: KeyConfig> FusedIterator for DenseIter<'_, T, K> {}

impl<T, K: KeyConfig> Clone for DenseIter<'_, T, K> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            keys: self.keys.clone(),
            values: self.values.clone(),
        }
    }
}

/// Iterator over `(key, &mut value)` pairs, in the order the values are
/// stored. It is created using [`DenseGenMap::iter_mut`] or
/// [`DenseSecondaryMap::iter_mut`](crate::DenseSecondaryMap::iter_mut).
pub struct DenseIterMut<'a, T, K> {
    pub(crate) keys: core::slice::Iter<'a, Key<K>>,
    pub(crate) values: core::slice::IterMut<'a, T>,
}

impl<'a, T, K: KeyConfig> Iterator for DenseIterMut<'a, T, K> {
    type Item = (Key<K>, &'a mut T);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let key = *self.keys.next()?;
        // SAFETY: the same as in `DenseIter::next`.
        Some((key, unsafe { self.values.next().unwrap_unchecked() }))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.values.size_hint()
    }
}

impl<T, K: KeyConfig> DoubleEndedIterator for DenseIterMut<'_, T, K> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        let key = *self.keys.next_back()?;
        // SAFETY: the same as in `DenseIter::next`.
        Some((key, unsafe { self.values.next_back().unwrap_unchecked() }))
    }
}

impl<T, K: KeyConfig> ExactSizeIterator for DenseIterMut<'_, T, K> {}
impl<T, K: KeyConfig> FusedIterator for DenseIterMut<'_, T, K> {}

/// Iterator over the keys, in the order the values are stored. It is created
/// using [`DenseGenMap::keys`] or
/// [`DenseSecondaryMap::keys`](crate::DenseSecondaryMap::keys).
pub struct DenseKeys<'a, K>(pub(crate) core::slice::Iter<'a, Key<K>>);

impl<K: KeyConfig> Iterator for DenseKeys<'_, K> {
    type Item = Key<K>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().copied()
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<K: KeyConfig> DoubleEndedIterator for DenseKeys<'_, K> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back().copied()
    }
}

impl<K: KeyConfig> ExactSizeIterator for DenseKeys<'_, K> {}
impl<K: KeyConfig> FusedIterator for DenseKeys<'_, K> {}

impl<K: KeyConfig> Clone for DenseKeys<'_, K> {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// Iterator over references to the values, in the order they are stored. It
/// is created using [`DenseGenMap::values`] or
/// [`DenseSecondaryMap::values`](crate::DenseSecondaryMap::values).
pub struct DenseValues<'a, T>(pub(crate) core::slice::Iter<'a, T>);

impl<'a, T> Iterator for DenseValues<'a, T> {
    type Item = &'a T;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<T> DoubleEndedIterator for DenseValues<'_, T> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back()
    }
}

impl<T> ExactSizeIterator for DenseValues<'_, T> {}
impl<T> FusedIterator for DenseValues<'_, T> {}

impl<T> Clone for DenseValues<'_, T> {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// Iterator over mutable references to the values, in the order they are
/// stored. It is created using [`DenseGenMap::values_mut`] or
/// [`DenseSecondaryMap::values_mut`](crate::DenseSecondaryMap::values_mut).
pub struct DenseValuesMut<'a, T>(pub(crate) core::slice::IterMut<'a, T>);

impl<'a, T> Iterator for DenseValuesMut<'a, T> {
    type Item = &'a mut T;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<T> DoubleEndedIterator for DenseValuesMut<'_, T> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back()
    }
}

impl<T> ExactSizeIterator for DenseValuesMut<'_, T> {}
impl<T> FusedIterator for DenseValuesMut<'_, T> {}

/// Owning iterator over `(key, value)` pairs, in the order the values were
/// stored. It is created by consuming a dense map with `into_iter`, which a
/// dense map only has when its pair storage implements `IntoIterator`. Its
/// type parameter is the map's pair storage. It implements
/// `DoubleEndedIterator` when the pair storage's iterator implements
/// `DoubleEndedIterator` and `ExactSizeIterator`.
pub struct DenseIntoIter<P: IntoIterator> {
    pub(crate) pairs: P::IntoIter,
    /// The number of pairs not yet yielded.
    pub(crate) remaining: usize,
}

impl<P: IntoIterator> Iterator for DenseIntoIter<P> {
    type Item = P::Item;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let pair = self.pairs.next()?;
        self.remaining -= 1;
        Some(pair)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<P: IntoIterator> DoubleEndedIterator for DenseIntoIter<P>
where
    P::IntoIter: DoubleEndedIterator + ExactSizeIterator,
{
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let pair = self.pairs.next_back()?;
        self.remaining -= 1;
        Some(pair)
    }
}

impl<P: IntoIterator> ExactSizeIterator for DenseIntoIter<P> {}
impl<P: IntoIterator> FusedIterator for DenseIntoIter<P> {}

/// `DenseGenMapRawParts` holds the fields of a [`DenseGenMap`].
/// [`into_raw_parts`](DenseGenMap::into_raw_parts) takes a map apart into
/// these fields, and [`from_raw_parts`](DenseGenMap::from_raw_parts) builds a
/// map from them.
///
/// The parts given to `from_raw_parts` must follow every rule below, and the
/// parts that `into_raw_parts` returns always do.
///
/// The values are the data that the map stores under its keys, and the second
/// slice of `pairs` holds them. Editing or replacing an item in that slice
/// never breaks a rule, and neither does changing the capacity of `slots` or
/// `pairs`.
///
/// # Rules
///
/// - No slot sits at a position above the largest index the map's keys can
///   hold, or at the largest value of the index type.
/// - A slot that holds a value has a generation no larger than the largest
///   one the map's keys can hold.
/// - The free list starts at the slot whose index is in `next_free`. Each
///   slot on the list stores the index of the next slot on the list, and the
///   last one stores the largest value of the index type, which `next_free`
///   also holds when the list is empty. Every slot on the list holds no value
///   and has a generation below the largest one the map's keys can hold, and
///   no slot is on the list twice.
/// - `pairs` holds one pair for each slot that holds a value.
/// - Each slot that holds a value stores the position of its value in the
///   second slice of `pairs`, and no two of those slots store the same
///   position.
/// - The key at each position in the first slice of `pairs` is the key of
///   the value at the same position in the second slice. The key has the
///   index of the slot that stores that position, and the generation of that
///   slot.
///
/// [`DenseMapSlot`] explains what a free, detached or retired slot stores in
/// place of a position.
///
/// # Examples
///
/// ```
/// use gen_map::{DenseGenMap, PairStorage};
///
/// let mut map = DenseGenMap::new();
/// let a = map.insert("a");
/// let b = map.insert("b");
///
/// // The two values swap places, and so do their keys and the positions
/// // that their slots store.
/// let mut parts = map.into_raw_parts();
/// let (keys, values) = parts.pairs.slices_mut();
/// keys.swap(0, 1);
/// values.swap(0, 1);
/// *parts.slots[0].get_odd_mut(a.generation()).unwrap() = 1;
/// *parts.slots[1].get_odd_mut(b.generation()).unwrap() = 0;
/// // SAFETY: each slot that holds a value stores the new position of its
/// // value, and each key moved together with its value.
/// let map = unsafe { DenseGenMap::from_raw_parts(parts) };
/// assert_eq!(map.values().copied().collect::<Vec<_>>(), ["b", "a"]);
/// assert_eq!((map[a], map[b]), ("a", "b"));
/// ```
pub struct DenseGenMapRawParts<
    T,
    #[cfg(feature = "alloc")] C: DenseGenMapConfig = DefaultMapConfig,
    #[cfg(not(feature = "alloc"))] C: DenseGenMapConfig,
> {
    // These fields are in the same order as the fields of `DenseGenMap` on
    // purpose, so that the two structs can be read side by side.
    // `into_raw_parts` and `from_raw_parts` list every field of both structs,
    // so the compiler catches a field that only one of them has, but nothing
    // catches a change in order. Think twice before removing this comment,
    // because it is the only thing that keeps the two orders the same.
    /// The map keeps its slots in this storage, which the map's config picks.
    /// A slot's index is its position in the storage, and a slot that holds a
    /// value stores the position of that value in `pairs`.
    pub slots: <C as DenseGenMapConfig>::SlotStorage<DenseMapSlot<C>>,
    /// `next_free` holds the index of the first slot on the free list, or the
    /// largest value of the index type if no slot is free.
    pub next_free: MapIdx<C>,
    /// The map keeps its values one after another in the second slice of
    /// this storage, and the key of each value at the same position in the
    /// first slice.
    pub pairs: <C as DenseGenMapConfig>::PairStorage<Key<MapKeyConfig<C>>, T>,
}

impl<T, C: DenseGenMapConfig> DenseGenMap<T, C> {
    /// Takes the map apart into its fields.
    /// [`from_raw_parts`](Self::from_raw_parts) builds a map from them again,
    /// and [`DenseGenMapRawParts`] lists the rules they follow.
    #[inline]
    pub fn into_raw_parts(self) -> DenseGenMapRawParts<T, C> {
        let Self {
            slots,
            next_free,
            pairs,
        } = self;
        DenseGenMapRawParts {
            slots,
            next_free,
            pairs,
        }
    }

    /// Builds a map from the fields that
    /// [`into_raw_parts`](Self::into_raw_parts) takes a map apart into.
    ///
    /// # Safety
    ///
    /// `parts` must follow every rule listed on [`DenseGenMapRawParts`]. The
    /// map's methods rely on those rules, and parts that break one can make
    /// them cause undefined behavior.
    #[inline]
    #[must_use]
    pub unsafe fn from_raw_parts(parts: DenseGenMapRawParts<T, C>) -> Self {
        let DenseGenMapRawParts {
            slots,
            next_free,
            pairs,
        } = parts;
        Self {
            slots,
            next_free,
            pairs,
        }
    }
}
