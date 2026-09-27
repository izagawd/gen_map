#[cfg(feature = "alloc")]
use crate::config::DefaultMapConfig;
use crate::config::{MapConfig, SecondaryMapConfig, SecondaryMapConfigFor};
use crate::error::{GetDisjointMutAtError, GetDisjointMutError, SecondaryInsertError};
use crate::key::Key;
use crate::key_layout::KeyLayout;
use crate::key_piece::KeyPiece;
use crate::map::{Layout, MapGen, MapIdx, MapKeyConfig};
use crate::parity::Odd;
use crate::replace_strategy::ReplaceStrategy;
use crate::slot::SecondarySlot;
use crate::storage::SlotStorage;
use core::fmt;
use core::iter::{Enumerate, FusedIterator};
use core::ops::{Index, IndexMut};
use core::slice;

/// The [`SecondarySlot`] a [`SecondaryMap<T, C>`](SecondaryMap) keeps each
/// value in.
pub type SecondaryMapSlot<T, C> = SecondarySlot<MapKeyConfig<T, C>, T>;

/// The storage a config gives the map for its slots.
type Slots<T, C> = <C as SecondaryMapConfig<SecondaryMapSlot<T, C>>>::Storage;

/// The strategy a config gives the map.
type Strategy<T, C> = <C as SecondaryMapConfig<SecondaryMapSlot<T, C>>>::ReplaceStrategy;

/// The error the storage of a `SecondaryMap<T, C>` gives when it cannot make
/// room for a slot. It is `TryReserveError` for a `Vec`, `CapacityError` for
/// an `ArrayVec` and `CollectionAllocErr` for a `SmallVec`.
pub type SecondaryStorageError<T, C> = <Slots<T, C> as SlotStorage>::Error;

/// What [`SecondaryMap::insert`] returns.
type InsertResult<T, C> = Result<Option<T>, SecondaryInsertError<T, SecondaryStorageError<T, C>>>;

/// Builds the key whose index is `position` and whose generation is
/// `generation`, without checking that they fit.
///
/// # Safety
///
/// `position` must fit in the key's index type, and that index and
/// `generation` must fit the key's layout together. Both hold for the
/// position and generation of a slot that holds a value, as the comment on
/// the map's `slots` field explains.
#[inline]
unsafe fn key_from_parts_unchecked<T, C: MapConfig<T>>(
    position: usize,
    generation: Odd<MapGen<T, C>>,
) -> Key<MapKeyConfig<T, C>> {
    debug_assert!(
        MapIdx::<T, C>::from_usize(position)
            .and_then(|idx| {
                <Layout<T, C> as KeyLayout<MapIdx<T, C>, MapGen<T, C>>>::pack(idx, generation)
            })
            .is_some(),
        "the index and the generation fit the key"
    );
    // SAFETY: the caller promises that `position` fits in the index type,
    // and that the index and `generation` fit the layout.
    unsafe {
        let idx = MapIdx::<T, C>::from_usize_unchecked(position);
        Key::from_repr(
            <Layout<T, C> as KeyLayout<MapIdx<T, C>, MapGen<T, C>>>::pack_unchecked(
                idx, generation,
            ),
        )
    }
}

/// A map from the keys of a [`GenMap`](crate::GenMap) to values of type
/// `T`, configured by `C`.
///
/// A `SecondaryMap` does not hand out keys. It stores values under keys
/// that a `GenMap` handed out, to add data to the values of a `GenMap`
/// without changing its value type. `C` must have the same
/// [`KeyConfig`](crate::KeyConfig) as the `GenMap`'s config, so that both
/// maps use the same key type.
///
/// Each value sits in a slot at its key's index, together with the key's
/// generation, and only a key with that index and generation matches it.
/// Removing a value from the `GenMap` does not remove it from here. Its key
/// keeps matching it until it is removed, or replaced by an insert under a
/// key with the same index and a different generation, which `C`'s
/// [`ReplaceStrategy`](crate::SecondaryMapConfig::ReplaceStrategy) allows or
/// refuses.
///
/// The map keeps a slot at every index up to the largest one inserted, in
/// the [`SlotStorage`] its [`SecondaryMapConfig`] picks, the same kind of
/// storage a `GenMap` uses.
///
/// With the `alloc` feature, `C` defaults to [`DefaultMapConfig`], which
/// keeps the slots in a `Vec` and uses
/// [`NewerWinsWrapping`](crate::NewerWinsWrapping) to decide whether an
/// insert replaces a value that was inserted under a different generation.
///
/// # Examples
///
/// ```
/// use gen_map::{GenMap, SecondaryMap};
///
/// let mut names = GenMap::new();
/// let mut ages = SecondaryMap::new();
///
/// let alice = names.insert("Alice");
/// let bob = names.insert("Bob");
/// ages.insert(alice, 30).unwrap();
///
/// assert_eq!(ages.get(alice), Some(&30));
/// assert_eq!(ages.get(bob), None);
/// ```
pub struct SecondaryMap<
    T,
    #[cfg(feature = "alloc")] C: SecondaryMapConfigFor<T> = DefaultMapConfig,
    #[cfg(not(feature = "alloc"))] C: SecondaryMapConfigFor<T>,
> {
    // A slot that holds a value always sits at the index of the key the value
    // was inserted under, and has that key's generation. `insert` is the only
    // method that puts a value in a slot, and `clone` copies each slot to the
    // same position. The map builds keys from its slots without checks because
    // of this.
    slots: Slots<T, C>,
    len: usize,
}

#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
impl<T> SecondaryMap<T> {
    /// Creates an empty map with the [`DefaultMapConfig`].
    ///
    /// Use [`new_with_config`](Self::new_with_config) for any other config,
    /// since Rust does not use a default type parameter when it infers
    /// types.
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self::new_with_config()
    }
}

impl<T, C: SecondaryMapConfigFor<T>> SecondaryMap<T, C> {
    /// Creates an empty map with config `C`.
    #[inline]
    #[must_use]
    pub fn new_with_config() -> Self {
        Self {
            slots: Slots::<T, C>::empty(),
            len: 0,
        }
    }

    /// Returns the number of values in the map.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` if the map holds no values.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns `true` if a value is stored under `key`.
    #[inline]
    pub fn contains_key(&self, key: Key<MapKeyConfig<T, C>>) -> bool {
        self.get(key).is_some()
    }

    /// Returns a reference to the value stored under `key`, or `None` if
    /// there is none.
    #[inline]
    pub fn get(&self, key: Key<MapKeyConfig<T, C>>) -> Option<&T> {
        let position = key.idx().into_usize()?;
        self.slots
            .as_slice()
            .get(position)?
            .get_odd(key.generation())
    }

    /// Returns a mutable reference to the value stored under `key`, or
    /// `None` if there is none.
    #[inline]
    pub fn get_mut(&mut self, key: Key<MapKeyConfig<T, C>>) -> Option<&mut T> {
        let position = key.idx().into_usize()?;
        self.slots
            .as_mut_slice()
            .get_mut(position)?
            .get_odd_mut(key.generation())
    }

    /// [`get`](Self::get) without the bounds and generation checks.
    ///
    /// # Safety
    ///
    /// The map must have a value for `key`, meaning
    /// [`contains_key`](Self::contains_key) returns `true` for it.
    #[inline]
    pub unsafe fn get_unchecked(&self, key: Key<MapKeyConfig<T, C>>) -> &T {
        debug_assert!(self.contains_key(key));
        // SAFETY: the caller promises the map has a value for `key`, so the
        // key's index is the position of a slot in the storage. That
        // position fits in `usize`, and the slot holds a value.
        unsafe {
            self.slots
                .as_slice()
                .get_unchecked(key.idx().into_usize_unchecked())
                .get_odd_unchecked()
        }
    }

    /// [`get_mut`](Self::get_mut) without the bounds and generation checks.
    ///
    /// # Safety
    ///
    /// The map must have a value for `key`, meaning
    /// [`contains_key`](Self::contains_key) returns `true` for it.
    #[inline]
    pub unsafe fn get_unchecked_mut(&mut self, key: Key<MapKeyConfig<T, C>>) -> &mut T {
        debug_assert!(self.contains_key(key));
        // SAFETY: the same as in `get_unchecked`.
        unsafe {
            self.slots
                .as_mut_slice()
                .get_unchecked_mut(key.idx().into_usize_unchecked())
                .get_odd_unchecked_mut()
        }
    }

    /// Returns the key of the value in the slot at `idx`, or `None` if there
    /// is no slot at `idx` or the slot holds no value.
    #[inline]
    pub fn key_at(&self, idx: MapIdx<T, C>) -> Option<Key<MapKeyConfig<T, C>>> {
        let position = idx.into_usize()?;
        let (generation, _) = self.slots.as_slice().get(position)?.get()?;
        // SAFETY: the slot at `position` holds a value under `generation`, so
        // the two fit the key.
        Some(unsafe { key_from_parts_unchecked::<T, C>(position, generation) })
    }

    /// [`key_at`](Self::key_at) without the bounds and occupancy checks.
    ///
    /// # Safety
    ///
    /// There must be a slot at `idx` and it must hold a value, meaning
    /// [`key_at`](Self::key_at) returns `Some` for it. A slot that holds no
    /// value has generation zero, and a key's generation is an
    /// [`Odd`](crate::Odd), so building the key is undefined behavior.
    #[inline]
    pub unsafe fn key_at_unchecked(&self, idx: MapIdx<T, C>) -> Key<MapKeyConfig<T, C>> {
        debug_assert!(self.key_at(idx).is_some());
        // SAFETY: the caller promises a slot at `idx` that holds a value. So
        // `idx` fits in `usize` and names a slot in bounds, the slot's
        // generation is odd, and the position and generation fit the key.
        unsafe {
            let position = idx.into_usize_unchecked();
            let slot = self.slots.as_slice().get_unchecked(position);
            key_from_parts_unchecked::<T, C>(position, Odd::new_unchecked(slot.generation()))
        }
    }

    /// Returns the key and a reference to the value in the slot at `idx`, or
    /// `None` if there is no slot at `idx` or the slot holds no value.
    #[inline]
    pub fn get_at(&self, idx: MapIdx<T, C>) -> Option<(Key<MapKeyConfig<T, C>>, &T)> {
        let position = idx.into_usize()?;
        let (generation, value) = self.slots.as_slice().get(position)?.get()?;
        // SAFETY: the slot at `position` holds a value under `generation`, so
        // the two fit the key.
        let key = unsafe { key_from_parts_unchecked::<T, C>(position, generation) };
        Some((key, value))
    }

    /// Returns the key and a mutable reference to the value in the slot at
    /// `idx`, or `None` if there is no slot at `idx` or the slot holds no
    /// value. See [`get_at`](Self::get_at).
    #[inline]
    pub fn get_at_mut(&mut self, idx: MapIdx<T, C>) -> Option<(Key<MapKeyConfig<T, C>>, &mut T)> {
        let position = idx.into_usize()?;
        let (generation, value) = self.slots.as_mut_slice().get_mut(position)?.get_mut()?;
        // SAFETY: the slot at `position` holds a value under `generation`, so
        // the two fit the key.
        let key = unsafe { key_from_parts_unchecked::<T, C>(position, generation) };
        Some((key, value))
    }

    /// [`get_at`](Self::get_at) without the bounds and occupancy checks.
    ///
    /// # Safety
    ///
    /// There must be a slot at `idx` and it must hold a value, meaning
    /// [`key_at`](Self::key_at) returns `Some` for it. Otherwise the slot is
    /// read as if it held a value and the key gets built using generation
    /// zero, both of which are undefined behavior.
    #[inline]
    pub unsafe fn get_at_unchecked(&self, idx: MapIdx<T, C>) -> (Key<MapKeyConfig<T, C>>, &T) {
        debug_assert!(self.key_at(idx).is_some());
        // SAFETY: the same as in `key_at_unchecked`. The slot holds a value,
        // so reading it as one is sound.
        unsafe {
            let position = idx.into_usize_unchecked();
            let slot = self.slots.as_slice().get_unchecked(position);
            let generation = Odd::new_unchecked(slot.generation());
            let key = key_from_parts_unchecked::<T, C>(position, generation);
            (key, slot.get_odd_unchecked())
        }
    }

    /// [`get_at_mut`](Self::get_at_mut) without the bounds and occupancy
    /// checks.
    ///
    /// # Safety
    ///
    /// The same as for [`get_at_unchecked`](Self::get_at_unchecked).
    #[inline]
    pub unsafe fn get_at_unchecked_mut(
        &mut self,
        idx: MapIdx<T, C>,
    ) -> (Key<MapKeyConfig<T, C>>, &mut T) {
        debug_assert!(self.key_at(idx).is_some());
        // SAFETY: the same as in `get_at_unchecked`.
        unsafe {
            let position = idx.into_usize_unchecked();
            let slot = self.slots.as_mut_slice().get_unchecked_mut(position);
            let generation = Odd::new_unchecked(slot.generation());
            let key = key_from_parts_unchecked::<T, C>(position, generation);
            (key, slot.get_odd_unchecked_mut())
        }
    }

    /// Returns the generation of the slot at `idx`, or `None` if there is no
    /// slot at `idx`.
    ///
    /// A slot that holds a value has the generation of the value's key, which
    /// is odd. A slot that holds no value has generation zero.
    #[inline]
    pub fn generation_at(&self, idx: MapIdx<T, C>) -> Option<MapGen<T, C>> {
        Some(self.slots.as_slice().get(idx.into_usize()?)?.generation())
    }

    /// [`generation_at`](Self::generation_at) without the bounds check.
    ///
    /// # Safety
    ///
    /// There must be a slot at `idx`, meaning
    /// [`generation_at`](Self::generation_at) returns `Some` for it. The
    /// slot does not have to hold a value.
    #[inline]
    pub unsafe fn generation_at_unchecked(&self, idx: MapIdx<T, C>) -> MapGen<T, C> {
        debug_assert!(self.generation_at(idx).is_some());
        // SAFETY: the caller promises a slot at `idx`, so `idx` fits in
        // `usize` and names a slot in bounds.
        unsafe {
            self.slots
                .as_slice()
                .get_unchecked(idx.into_usize_unchecked())
                .generation()
        }
    }

    /// Returns the key and a mutable reference to the value in the slot at
    /// each index, all at once. This is [`get_disjoint_mut`] for indices
    /// instead of keys, and like [`get_at`](Self::get_at) it hands each key
    /// back with its value.
    ///
    /// Every index gets the same checks as [`get_at_mut`](Self::get_at_mut),
    /// and every pair of indices is checked to be different, so the
    /// references cannot alias. Checking every pair takes O(N²) time.
    ///
    /// # Errors
    ///
    /// Returns [`GetDisjointMutAtError::NoValue`] if there is no slot at one
    /// of the indices or the slot holds no value, and
    /// [`GetDisjointMutAtError::OverlappingIndices`] if two indices are the
    /// same. The error does not borrow the map.
    ///
    /// [`get_disjoint_mut`]: Self::get_disjoint_mut
    ///
    /// # Examples
    ///
    /// ```
    /// use gen_map::{GenMap, GetDisjointMutAtError, SecondaryMap};
    ///
    /// let mut keys = GenMap::new();
    /// let a = keys.insert(());
    /// let b = keys.insert(());
    /// let mut map = SecondaryMap::new();
    /// map.insert(a, 1).unwrap();
    /// map.insert(b, 2).unwrap();
    ///
    /// let [(key_a, x), (key_b, y)] = map.get_disjoint_mut_at([a.idx(), b.idx()]).unwrap();
    /// assert_eq!((key_a, key_b), (a, b));
    /// core::mem::swap(x, y);
    /// assert_eq!(map[a], 2);
    ///
    /// assert_eq!(
    ///     map.get_disjoint_mut_at([a.idx(), a.idx()]),
    ///     Err(GetDisjointMutAtError::OverlappingIndices)
    /// );
    /// ```
    #[inline]
    #[allow(clippy::type_complexity)]
    pub fn get_disjoint_mut_at<const N: usize>(
        &mut self,
        idxs: [MapIdx<T, C>; N],
    ) -> Result<[(Key<MapKeyConfig<T, C>>, &mut T); N], GetDisjointMutAtError> {
        for (i, idx) in idxs.iter().enumerate() {
            if self.key_at(*idx).is_none() {
                return Err(GetDisjointMutAtError::NoValue);
            }
            if idxs[..i].contains(idx) {
                return Err(GetDisjointMutAtError::OverlappingIndices);
            }
        }
        // SAFETY: every index was just found to name a slot that holds a
        // value, and every index is distinct.
        Ok(unsafe { self.get_disjoint_mut_at_unchecked(idxs) })
    }

    /// [`get_disjoint_mut_at`](Self::get_disjoint_mut_at) without any of the
    /// checks.
    ///
    /// # Safety
    ///
    /// There must be a slot at every index and each must hold a value,
    /// meaning [`key_at`](Self::key_at) returns `Some` for every one of them,
    /// and no two of the indices may be the same.
    #[inline]
    pub unsafe fn get_disjoint_mut_at_unchecked<const N: usize>(
        &mut self,
        idxs: [MapIdx<T, C>; N],
    ) -> [(Key<MapKeyConfig<T, C>>, &mut T); N] {
        debug_assert!(idxs.iter().all(|idx| self.key_at(*idx).is_some()));
        debug_assert!(idxs
            .iter()
            .enumerate()
            .all(|(i, idx)| !idxs[..i].contains(idx)));
        let slots = self.slots.as_mut_slice().as_mut_ptr();
        idxs.map(|idx| {
            // SAFETY: the caller promises that the slot exists and holds a
            // value, so its position is in bounds, its generation is odd, and
            // the two fit the key. The caller also promises that no other
            // index names the same slot, so the references do not alias.
            unsafe {
                let position = idx.into_usize_unchecked();
                let slot = &mut *slots.add(position);
                let generation = Odd::new_unchecked(slot.generation());
                let key = key_from_parts_unchecked::<T, C>(position, generation);
                (key, slot.get_odd_unchecked_mut())
            }
        })
    }

    /// Returns a mutable reference to the value of each key, all at once.
    ///
    /// Every key is checked the same way [`get_mut`](Self::get_mut) checks
    /// it, and every pair of keys is checked to point at different slots, so
    /// the references cannot alias. Checking every pair takes O(N²) time.
    ///
    /// # Errors
    ///
    /// Returns [`GetDisjointMutError::InvalidKey`] if the map has no value
    /// for one of the keys, and [`GetDisjointMutError::OverlappingKeys`] if
    /// two keys point at the same slot. The error does not borrow the map.
    ///
    /// # Examples
    ///
    /// ```
    /// use gen_map::{GenMap, GetDisjointMutError, SecondaryMap};
    ///
    /// let mut keys = GenMap::new();
    /// let a = keys.insert(());
    /// let b = keys.insert(());
    /// let mut map = SecondaryMap::new();
    /// map.insert(a, 1).unwrap();
    /// map.insert(b, 2).unwrap();
    ///
    /// let [x, y] = map.get_disjoint_mut([a, b]).unwrap();
    /// core::mem::swap(x, y);
    /// assert_eq!(map[a], 2);
    /// assert_eq!(map[b], 1);
    ///
    /// assert_eq!(
    ///     map.get_disjoint_mut([a, a]),
    ///     Err(GetDisjointMutError::OverlappingKeys)
    /// );
    /// ```
    #[inline]
    pub fn get_disjoint_mut<const N: usize>(
        &mut self,
        keys: [Key<MapKeyConfig<T, C>>; N],
    ) -> Result<[&mut T; N], GetDisjointMutError> {
        for (i, key) in keys.iter().enumerate() {
            if !self.contains_key(*key) {
                return Err(GetDisjointMutError::InvalidKey);
            }
            // Two keys that both have a value in one slot carry the slot's
            // generation, so the index alone says whether they overlap.
            if keys[..i].iter().any(|earlier| earlier.idx() == key.idx()) {
                return Err(GetDisjointMutError::OverlappingKeys);
            }
        }
        // SAFETY: every key was just found to have a value, and every index
        // is distinct.
        Ok(unsafe { self.get_disjoint_mut_unchecked(keys) })
    }

    /// [`get_disjoint_mut`](Self::get_disjoint_mut) without any of the
    /// checks.
    ///
    /// # Safety
    ///
    /// The map must have a value for every key, meaning
    /// [`contains_key`](Self::contains_key) returns `true` for each of them,
    /// and no two keys may point at the same slot. Otherwise a slot is read
    /// as if it held a value, or two of the references point at the same
    /// value, either of which is undefined behavior.
    #[inline]
    pub unsafe fn get_disjoint_mut_unchecked<const N: usize>(
        &mut self,
        keys: [Key<MapKeyConfig<T, C>>; N],
    ) -> [&mut T; N] {
        debug_assert!(keys.iter().all(|key| self.contains_key(*key)));
        debug_assert!(keys
            .iter()
            .enumerate()
            .all(|(i, key)| keys[..i].iter().all(|earlier| earlier.idx() != key.idx())));
        let slots = self.slots.as_mut_slice().as_mut_ptr();
        keys.map(|key| {
            // SAFETY: the caller promises the key has a value, so its index
            // fits in `usize` and names a slot in bounds that holds a value,
            // and that no other key names the same slot, so the references
            // do not alias.
            unsafe { (*slots.add(key.idx().into_usize_unchecked())).get_odd_unchecked_mut() }
        })
    }

    /// Stores `value` under `key`.
    ///
    /// - If the slot at the key's index is empty, the value goes in and
    ///   `Ok(None)` is returned.
    /// - If the slot holds a value under the same key, that value is
    ///   replaced and returned as `Ok(Some(old))`.
    /// - If the slot holds a value that was inserted under a different
    ///   generation, `C`'s
    ///   [`ReplaceStrategy`](crate::SecondaryMapConfig::ReplaceStrategy)
    ///   decides whether `value` replaces the value in the slot. If it does,
    ///   the old value is returned as `Ok(Some(old))`, and the slot now belongs
    ///   to `key`.
    ///
    /// # Errors
    ///
    /// Hands `value` back, and leaves the map as it was, if the strategy
    /// kept the old value or the storage could not make room for the slots.
    /// The [`SecondaryInsertError`] variant says which of the two happened.
    ///
    /// # Examples
    ///
    /// ```
    /// use gen_map::{GenMap, SecondaryInsertError, SecondaryMap};
    ///
    /// let mut names = GenMap::new();
    /// let mut ages = SecondaryMap::new();
    ///
    /// let alice = names.insert("Alice");
    /// assert_eq!(ages.insert(alice, 30).unwrap(), None);
    /// assert_eq!(ages.insert(alice, 31).unwrap(), Some(30));
    ///
    /// // Bob gets Alice's slot, with a newer generation, so his age replaces
    /// // hers, and Alice's old key can no longer replace his.
    /// names.remove(alice);
    /// let bob = names.insert("Bob");
    /// assert_eq!(bob.idx(), alice.idx());
    /// assert_eq!(ages.insert(bob, 25).unwrap(), Some(31));
    /// assert!(matches!(ages.insert(alice, 32), Err(SecondaryInsertError::Refused(32))));
    /// assert_eq!(ages[bob], 25);
    /// ```
    pub fn insert(&mut self, key: Key<MapKeyConfig<T, C>>, value: T) -> InsertResult<T, C> {
        let generation = key.generation();
        let slot = match Self::get_or_grow_slot(&mut self.slots, key.idx()) {
            Ok(slot) => slot,
            Err(error) => return Err(SecondaryInsertError::StorageFull(value, error)),
        };
        if let Some(old) = slot.get_odd_mut(generation) {
            return Ok(Some(core::mem::replace(old, value)));
        }
        match slot.get().map(|(current, _)| current) {
            None => {
                slot.replace(generation, value);
                self.len += 1;
                Ok(None)
            }
            Some(current)
                if <Strategy<T, C> as ReplaceStrategy<MapKeyConfig<T, C>>>::replaces(
                    current, generation,
                ) =>
            {
                Ok(slot.replace(generation, value))
            }
            Some(_) => Err(SecondaryInsertError::Refused(value)),
        }
    }

    /// Returns the slot at `idx`. If the storage has no slot there yet, it
    /// first grows to hold every slot up to and including `idx`, which can
    /// allocate, and the new slots start out empty.
    ///
    /// This function takes the storage rather than the whole map, so that
    /// `insert` can still change the map's length while it holds the slot.
    ///
    /// # Errors
    ///
    /// Returns the storage's error, and adds no slots, if the storage cannot
    /// make room for all of them.
    fn get_or_grow_slot(
        slots: &mut Slots<T, C>,
        idx: MapIdx<T, C>,
    ) -> Result<&mut SecondaryMapSlot<T, C>, SecondaryStorageError<T, C>> {
        let Some(position) = idx.into_usize() else {
            // No storage can hold a slot past `usize::MAX`. Asking for
            // `usize::MAX` more slots gets the error the storage gives when
            // it has no room.
            return Err(slots
                .ensure_room(usize::MAX)
                .expect_err("no storage has room for `usize::MAX` more slots"));
        };
        let len = slots.len();
        if position >= len {
            // `position - len + 1` slots are missing. That count overflows
            // when `position` is `usize::MAX` and the storage is empty, so
            // the map asks for `usize::MAX` slots instead, which no storage
            // can hold either.
            slots.ensure_room((position - len).saturating_add(1))?;
            for _ in len..=position {
                // `ensure_room` made room for every one of these slots, and
                // `SlotStorage` promises that the pushes succeed after
                // that. A push that fails here means the storage broke that
                // promise.
                if slots.try_push(SecondarySlot::empty()).is_err() {
                    panic!("SlotStorage::try_push failed although ensure_room returned Ok");
                }
            }
        }
        debug_assert!(position < slots.len());
        // SAFETY: the storage either had a slot at `position` already, or
        // the loop above pushed a slot for every position up to it.
        // `SlotStorage` promises that each push adds one slot at the end,
        // so `position` is in bounds.
        Ok(unsafe { slots.as_mut_slice().get_unchecked_mut(position) })
    }

    /// Removes the value stored under `key` and returns it, or returns
    /// `None` if there is none. The slot stays in the storage without a
    /// value.
    #[inline]
    pub fn remove(&mut self, key: Key<MapKeyConfig<T, C>>) -> Option<T> {
        let position = key.idx().into_usize()?;
        let slot = self.slots.as_mut_slice().get_mut(position)?;
        slot.get_odd(key.generation())?;
        let value = slot.take()?;
        self.len -= 1;
        Some(value)
    }

    /// Keeps only the values `f` returns `true` for, and removes the rest.
    /// It calls `f` on the values in index order.
    ///
    /// # Examples
    ///
    /// ```
    /// use gen_map::{GenMap, SecondaryMap};
    ///
    /// let mut keys = GenMap::new();
    /// let mut map = SecondaryMap::new();
    /// for i in 0..4 {
    ///     map.insert(keys.insert(()), i).unwrap();
    /// }
    /// map.retain(|_, value| *value % 2 == 0);
    /// assert_eq!(map.len(), 2);
    /// ```
    pub fn retain<F: FnMut(Key<MapKeyConfig<T, C>>, &mut T) -> bool>(&mut self, mut f: F) {
        for (position, slot) in self.slots.as_mut_slice().iter_mut().enumerate() {
            if let Some((generation, value)) = slot.get_mut() {
                // SAFETY: the slot at `position` holds a value under
                // `generation`, so the two fit the key.
                let key = unsafe { key_from_parts_unchecked::<T, C>(position, generation) };
                if !f(key, value) {
                    self.len -= 1;
                    drop(slot.take());
                }
            }
        }
    }

    /// Removes every value and every slot.
    #[inline]
    pub fn clear(&mut self) {
        self.len = 0;
        self.slots.clear();
    }

    /// Returns an iterator over the keys and references to the values, in
    /// index order.
    #[inline]
    pub fn iter(&self) -> SecondaryIter<'_, T, C> {
        SecondaryIter {
            slots: self.slots.as_slice().iter().enumerate(),
            remaining: self.len,
        }
    }

    /// Returns an iterator over the keys, in index order.
    #[inline]
    pub fn keys(&self) -> SecondaryKeys<'_, T, C> {
        SecondaryKeys { inner: self.iter() }
    }

    /// Returns an iterator over references to the values, in index order.
    #[inline]
    pub fn values(&self) -> SecondaryValues<'_, T, C> {
        SecondaryValues { inner: self.iter() }
    }

    /// Returns an iterator over the keys and mutable references to the
    /// values, in index order.
    #[inline]
    pub fn iter_mut(&mut self) -> SecondaryIterMut<'_, T, C> {
        SecondaryIterMut {
            slots: self.slots.as_mut_slice().iter_mut().enumerate(),
            remaining: self.len,
        }
    }

    /// Returns an iterator over mutable references to the values, in index
    /// order.
    #[inline]
    pub fn values_mut(&mut self) -> SecondaryValuesMut<'_, T, C> {
        SecondaryValuesMut {
            inner: self.iter_mut(),
        }
    }

    /// Removes every value, and returns an iterator over them and their
    /// keys, in index order. When the iterator is dropped, it removes the
    /// values it has not reached, and every slot.
    #[inline]
    pub fn drain(&mut self) -> SecondaryDrain<'_, T, C> {
        SecondaryDrain {
            map: self,
            position: 0,
        }
    }

    /// Returns the number of slots in the storage, whether they hold a value
    /// or not.
    pub fn slots_len(&self) -> usize {
        self.slots.len()
    }
}

impl<T, C: SecondaryMapConfigFor<T>> Default for SecondaryMap<T, C> {
    #[inline]
    fn default() -> Self {
        Self::new_with_config()
    }
}

impl<T: Clone, C: SecondaryMapConfigFor<T>> Clone for SecondaryMap<T, C> {
    /// The clone has the same slots, so every key of the original works on
    /// it.
    fn clone(&self) -> Self {
        // This pushes a clone of each slot rather than calling the storage's
        // own `Clone`, which the `SlotStorage` contract does not cover.
        // The map builds keys from its slots without checks, which relies on
        // every slot being where `insert` put it.
        let mut slots = Slots::<T, C>::with_capacity(self.slots.len());
        for slot in self.slots.as_slice() {
            // A push only fails here for a storage that cannot hold as many
            // slots as another of its type.
            if slots.try_push(slot.clone()).is_err() {
                panic!("SlotStorage::try_push failed while cloning a storage of the same type");
            }
        }
        Self {
            slots,
            len: self.len,
        }
    }
}

impl<T: fmt::Debug, C: SecondaryMapConfigFor<T>> fmt::Debug for SecondaryMap<T, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<T, C: SecondaryMapConfigFor<T>> Index<Key<MapKeyConfig<T, C>>> for SecondaryMap<T, C> {
    type Output = T;

    /// Returns a reference to the value stored under `key`.
    ///
    /// # Panics
    ///
    /// Panics if no value is stored under `key`.
    #[inline]
    fn index(&self, key: Key<MapKeyConfig<T, C>>) -> &T {
        self.get(key).expect("invalid SecondaryMap key")
    }
}

impl<T, C: SecondaryMapConfigFor<T>> IndexMut<Key<MapKeyConfig<T, C>>> for SecondaryMap<T, C> {
    /// Returns a mutable reference to the value stored under `key`.
    ///
    /// # Panics
    ///
    /// Panics if no value is stored under `key`.
    #[inline]
    fn index_mut(&mut self, key: Key<MapKeyConfig<T, C>>) -> &mut T {
        self.get_mut(key).expect("invalid SecondaryMap key")
    }
}

impl<T, C: SecondaryMapConfigFor<T>> Extend<(Key<MapKeyConfig<T, C>>, T)> for SecondaryMap<T, C> {
    /// Inserts each pair with [`insert`](SecondaryMap::insert). A value
    /// whose insert is refused by the strategy is dropped.
    ///
    /// # Panics
    ///
    /// Panics if the storage cannot make room for a slot.
    fn extend<I: IntoIterator<Item = (Key<MapKeyConfig<T, C>>, T)>>(&mut self, iter: I) {
        for (key, value) in iter {
            match self.insert(key, value) {
                Ok(_) | Err(SecondaryInsertError::Refused(_)) => {}
                Err(error) => panic!("SecondaryMap cannot insert: {error:?}"),
            }
        }
    }
}

impl<T, C: SecondaryMapConfigFor<T>> FromIterator<(Key<MapKeyConfig<T, C>>, T)>
    for SecondaryMap<T, C>
{
    /// Creates a map with config `C` and fills it the way
    /// [`extend`](Extend::extend) does, with the same panics.
    fn from_iter<I: IntoIterator<Item = (Key<MapKeyConfig<T, C>>, T)>>(iter: I) -> Self {
        let mut map = Self::new_with_config();
        map.extend(iter);
        map
    }
}

/// Iterator over `(key, &value)` pairs, in index order. Created by
/// [`SecondaryMap::iter`].
pub struct SecondaryIter<'a, T: 'a, C: SecondaryMapConfigFor<T> + 'a> {
    slots: Enumerate<slice::Iter<'a, SecondaryMapSlot<T, C>>>,
    remaining: usize,
}

impl<'a, T, C: SecondaryMapConfigFor<T>> Iterator for SecondaryIter<'a, T, C> {
    type Item = (Key<MapKeyConfig<T, C>>, &'a T);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        for (position, slot) in self.slots.by_ref() {
            if let Some((generation, value)) = slot.get() {
                self.remaining -= 1;
                // SAFETY: the slot at `position` holds a value under
                // `generation`, so the two fit the key.
                let key = unsafe { key_from_parts_unchecked::<T, C>(position, generation) };
                return Some((key, value));
            }
        }
        // Only a storage that lost a value gets here. Stopping for good
        // keeps the iterator fused.
        self.remaining = 0;
        None
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<T, C: SecondaryMapConfigFor<T>> ExactSizeIterator for SecondaryIter<'_, T, C> {}
impl<T, C: SecondaryMapConfigFor<T>> FusedIterator for SecondaryIter<'_, T, C> {}

// This impl is written by hand, because a derive would require `T: Clone`
// and `C: Clone`.
impl<T, C: SecondaryMapConfigFor<T>> Clone for SecondaryIter<'_, T, C> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            slots: self.slots.clone(),
            remaining: self.remaining,
        }
    }
}

/// Iterator over `(key, &mut value)` pairs, in index order. Created by
/// [`SecondaryMap::iter_mut`].
pub struct SecondaryIterMut<'a, T: 'a, C: SecondaryMapConfigFor<T> + 'a> {
    slots: Enumerate<slice::IterMut<'a, SecondaryMapSlot<T, C>>>,
    remaining: usize,
}

impl<'a, T, C: SecondaryMapConfigFor<T>> Iterator for SecondaryIterMut<'a, T, C> {
    type Item = (Key<MapKeyConfig<T, C>>, &'a mut T);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        for (position, slot) in self.slots.by_ref() {
            if let Some((generation, value)) = slot.get_mut() {
                self.remaining -= 1;
                // SAFETY: the slot at `position` holds a value under
                // `generation`, so the two fit the key.
                let key = unsafe { key_from_parts_unchecked::<T, C>(position, generation) };
                return Some((key, value));
            }
        }
        // Only a storage that lost a value gets here. Stopping for good
        // keeps the iterator fused.
        self.remaining = 0;
        None
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<T, C: SecondaryMapConfigFor<T>> ExactSizeIterator for SecondaryIterMut<'_, T, C> {}
impl<T, C: SecondaryMapConfigFor<T>> FusedIterator for SecondaryIterMut<'_, T, C> {}

/// Iterator over keys, in index order. Created by [`SecondaryMap::keys`].
pub struct SecondaryKeys<'a, T: 'a, C: SecondaryMapConfigFor<T> + 'a> {
    inner: SecondaryIter<'a, T, C>,
}

impl<T, C: SecondaryMapConfigFor<T>> Iterator for SecondaryKeys<'_, T, C> {
    type Item = Key<MapKeyConfig<T, C>>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(key, _)| key)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<T, C: SecondaryMapConfigFor<T>> ExactSizeIterator for SecondaryKeys<'_, T, C> {}
impl<T, C: SecondaryMapConfigFor<T>> FusedIterator for SecondaryKeys<'_, T, C> {}

impl<T, C: SecondaryMapConfigFor<T>> Clone for SecondaryKeys<'_, T, C> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

/// Iterator over references to the values, in index order. Created by
/// [`SecondaryMap::values`].
pub struct SecondaryValues<'a, T: 'a, C: SecondaryMapConfigFor<T> + 'a> {
    inner: SecondaryIter<'a, T, C>,
}

impl<'a, T, C: SecondaryMapConfigFor<T>> Iterator for SecondaryValues<'a, T, C> {
    type Item = &'a T;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(_, value)| value)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<T, C: SecondaryMapConfigFor<T>> ExactSizeIterator for SecondaryValues<'_, T, C> {}
impl<T, C: SecondaryMapConfigFor<T>> FusedIterator for SecondaryValues<'_, T, C> {}

impl<T, C: SecondaryMapConfigFor<T>> Clone for SecondaryValues<'_, T, C> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

/// Iterator over mutable references to the values, in index order. Created
/// by [`SecondaryMap::values_mut`].
pub struct SecondaryValuesMut<'a, T: 'a, C: SecondaryMapConfigFor<T> + 'a> {
    inner: SecondaryIterMut<'a, T, C>,
}

impl<'a, T, C: SecondaryMapConfigFor<T>> Iterator for SecondaryValuesMut<'a, T, C> {
    type Item = &'a mut T;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(_, value)| value)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<T, C: SecondaryMapConfigFor<T>> ExactSizeIterator for SecondaryValuesMut<'_, T, C> {}
impl<T, C: SecondaryMapConfigFor<T>> FusedIterator for SecondaryValuesMut<'_, T, C> {}

/// Iterator that takes each value out, with its key, in index order.
/// Created by [`SecondaryMap::drain`].
pub struct SecondaryDrain<'a, T: 'a, C: SecondaryMapConfigFor<T> + 'a> {
    map: &'a mut SecondaryMap<T, C>,
    /// The position of the slot the iterator checks next.
    position: usize,
}

impl<T, C: SecondaryMapConfigFor<T>> Iterator for SecondaryDrain<'_, T, C> {
    type Item = (Key<MapKeyConfig<T, C>>, T);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.map.len == 0 {
            return None;
        }
        while let Some(slot) = self.map.slots.as_mut_slice().get_mut(self.position) {
            let position = self.position;
            self.position += 1;
            if let Some((generation, value)) =
                core::mem::replace(slot, SecondarySlot::empty()).into_inner()
            {
                self.map.len -= 1;
                // SAFETY: the slot at `position` held this value under
                // `generation`, so the two fit the key.
                let key = unsafe { key_from_parts_unchecked::<T, C>(position, generation) };
                return Some((key, value));
            }
        }
        // Only a storage that lost a value gets here. The map has nothing
        // left to take, which also keeps the iterator fused.
        self.map.len = 0;
        None
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.map.len, Some(self.map.len))
    }
}

impl<T, C: SecondaryMapConfigFor<T>> ExactSizeIterator for SecondaryDrain<'_, T, C> {}
impl<T, C: SecondaryMapConfigFor<T>> FusedIterator for SecondaryDrain<'_, T, C> {}

impl<T, C: SecondaryMapConfigFor<T>> Drop for SecondaryDrain<'_, T, C> {
    fn drop(&mut self) {
        self.map.clear();
    }
}

/// Owning iterator over `(key, value)` pairs, in index order. Created by
/// consuming a map with `into_iter`, which a map only has when its storage
/// implements `IntoIterator`.
pub struct SecondaryIntoIter<T, C: SecondaryMapConfigFor<T>>
where
    Slots<T, C>: IntoIterator<Item = SecondaryMapSlot<T, C>>,
{
    slots: Enumerate<<Slots<T, C> as IntoIterator>::IntoIter>,
    remaining: usize,
}

impl<T, C: SecondaryMapConfigFor<T>> Iterator for SecondaryIntoIter<T, C>
where
    Slots<T, C>: IntoIterator<Item = SecondaryMapSlot<T, C>>,
{
    type Item = (Key<MapKeyConfig<T, C>>, T);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        for (position, slot) in self.slots.by_ref() {
            if let Some((generation, value)) = slot.into_inner() {
                self.remaining -= 1;
                // SAFETY: the slot at `position` holds a value under
                // `generation`, so the two fit the key.
                let key = unsafe { key_from_parts_unchecked::<T, C>(position, generation) };
                return Some((key, value));
            }
        }
        // Only a storage that lost a value gets here. Stopping for good
        // keeps the iterator fused.
        self.remaining = 0;
        None
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<T, C: SecondaryMapConfigFor<T>> ExactSizeIterator for SecondaryIntoIter<T, C> where
    Slots<T, C>: IntoIterator<Item = SecondaryMapSlot<T, C>>
{
}
impl<T, C: SecondaryMapConfigFor<T>> FusedIterator for SecondaryIntoIter<T, C> where
    Slots<T, C>: IntoIterator<Item = SecondaryMapSlot<T, C>>
{
}

impl<T, C: SecondaryMapConfigFor<T>> IntoIterator for SecondaryMap<T, C>
where
    Slots<T, C>: IntoIterator<Item = SecondaryMapSlot<T, C>>,
{
    type Item = (Key<MapKeyConfig<T, C>>, T);
    type IntoIter = SecondaryIntoIter<T, C>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        SecondaryIntoIter {
            remaining: self.len,
            slots: self.slots.into_iter().enumerate(),
        }
    }
}

impl<'a, T, C: SecondaryMapConfigFor<T>> IntoIterator for &'a SecondaryMap<T, C> {
    type Item = (Key<MapKeyConfig<T, C>>, &'a T);
    type IntoIter = SecondaryIter<'a, T, C>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T, C: SecondaryMapConfigFor<T>> IntoIterator for &'a mut SecondaryMap<T, C> {
    type Item = (Key<MapKeyConfig<T, C>>, &'a mut T);
    type IntoIter = SecondaryIterMut<'a, T, C>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}
