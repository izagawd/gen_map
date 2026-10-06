#[cfg(feature = "alloc")]
use crate::config::DefaultMapConfig;
use crate::config::{KeyConfig, MapConfig, SecondaryMapConfig};
use crate::error::{GetDisjointMutAtError, GetDisjointMutError, SecondaryInsertError};
use crate::key::Key;
use crate::key_piece::KeyPiece;
use crate::map::{decrement_len, increment_len, MapGen, MapIdx, MapKeyConfig};
use crate::parity::{Even, Odd};
use crate::replace_strategy::ReplaceStrategy;
use crate::slot::{Parity, Slot};
use crate::storage::{ReserveStorage, SliceStorage};
use core::fmt;
use core::iter::{Enumerate, FusedIterator};
use core::ops::{Index, IndexMut};
use core::slice;

/// A [`SecondaryMap`] keeps one [`Slot`] of this type for each index. While
/// the slot holds a value, its generation is the generation of the value's
/// key, which is odd, and it stores that value. While it holds no value, its
/// generation is zero, which is even, and it stores nothing in place of a
/// value. [`Slot::as_parity`] checks the parity of the generation and returns
/// either the value in [`Parity::Odd`] or `()` in [`Parity::Even`].
pub type SecondaryMapSlot<T, C> = Slot<MapGen<C>, T, ()>;

/// Returns the generation and a reference to the value of `slot` if it holds
/// a value, or `None` if it holds none.
#[inline]
fn occupied<G: KeyPiece, T>(slot: &Slot<G, T, ()>) -> Option<(Odd<G>, &T)> {
    match slot.as_parity() {
        Parity::Odd(generation, value) => Some((generation, value)),
        Parity::Even(..) => None,
    }
}

/// Returns the generation and a mutable reference to the value of `slot` if it
/// holds a value, or `None` if it holds none.
#[inline]
fn occupied_mut<G: KeyPiece, T>(slot: &mut Slot<G, T, ()>) -> Option<(Odd<G>, &mut T)> {
    match slot.as_parity_mut() {
        Parity::Odd(generation, value) => Some((generation, value)),
        Parity::Even(..) => None,
    }
}

/// Puts `value` in `slot` under `generation`, and returns the value the slot
/// held before, if it held one.
#[inline]
fn replace<G: KeyPiece, T>(slot: &mut Slot<G, T, ()>, generation: Odd<G>, value: T) -> Option<T> {
    match slot.set_odd(generation, value) {
        Parity::Odd(_, old) => Some(old),
        Parity::Even(..) => None,
    }
}

/// Takes the value out of `slot` and leaves the slot with generation zero and
/// no value. Returns `None` if the slot held no value.
#[inline]
fn take<G: KeyPiece, T>(slot: &mut Slot<G, T, ()>) -> Option<T> {
    match slot.set_even(Even::ZERO, ()) {
        Parity::Odd(_, value) => Some(value),
        Parity::Even(..) => None,
    }
}

/// Takes the generation and the value out of `slot`, or returns `None` if it
/// holds no value.
#[inline]
fn into_parts<G: KeyPiece, T>(slot: Slot<G, T, ()>) -> Option<(Odd<G>, T)> {
    match slot.into_parity() {
        Parity::Odd(generation, value) => Some((generation, value)),
        Parity::Even(..) => None,
    }
}

/// Returns a slot with generation zero, which holds no value.
#[inline]
fn empty_slot<G: KeyPiece, T>() -> Slot<G, T, ()> {
    Slot::new_even(Even::ZERO, ())
}

/// The storage a config gives the map for its slots.
type Slots<T, C> = <C as SecondaryMapConfig>::Storage<SecondaryMapSlot<T, C>>;

/// The strategy a config gives the map.
type Strategy<C> = <C as SecondaryMapConfig>::ReplaceStrategy;

/// The error the storage of a `SecondaryMap<T, C>` gives when it cannot make
/// room for a slot. It is `TryReserveError` for a `Vec`, `CapacityError` for
/// an `ArrayVec` and `CollectionAllocErr` for a `SmallVec`.
pub type SecondaryStorageError<T, C> = <Slots<T, C> as SliceStorage>::Error;

/// What [`SecondaryMap::insert`] returns.
type InsertResult<T, C> = Result<Option<T>, SecondaryInsertError<T, SecondaryStorageError<T, C>>>;

/// Builds the key whose index is `position` and whose generation is
/// `generation`, without checking that they fit.
///
/// # Safety
///
/// `position` must fit in the key's index type, and that index and
/// `generation` must fit the key config together. Both hold for the
/// position and generation of a slot that holds a value, as the comment on
/// the map's `slots` field explains.
#[inline]
unsafe fn key_from_parts_unchecked<C: MapConfig>(
    position: usize,
    generation: Odd<MapGen<C>>,
) -> Key<MapKeyConfig<C>> {
    debug_assert!(
        MapIdx::<C>::from_usize(position)
            .and_then(|idx| <MapKeyConfig<C> as KeyConfig>::pack(idx, generation))
            .is_some(),
        "the index and the generation fit the key"
    );
    // SAFETY: the caller promises that `position` fits in the index type,
    // and that the index and `generation` fit the key config.
    unsafe {
        let idx = MapIdx::<C>::from_usize_unchecked(position);
        Key::from_repr(<MapKeyConfig<C> as KeyConfig>::pack_unchecked(
            idx, generation,
        ))
    }
}

/// A map from the keys of a [`GenMap`](crate::GenMap) to values of type
/// `T`, configured by `C`.
///
/// A `SecondaryMap` does not hand out keys. It stores values under keys
/// that a `GenMap` handed out, to add data to the values of a `GenMap`
/// without changing its value type. To use a `GenMap`'s keys, `C` must have
/// the same [`KeyConfig`](crate::KeyConfig) as the `GenMap`'s config, so
/// that both maps use the same key type.
///
/// Each value sits in a slot at its key's index, together with the key's
/// generation, and only a key with that index and generation matches it.
/// Removing a key's value from the `GenMap` does not remove the key's value
/// from this map. The key keeps matching its value in this map until the value
/// is removed from this map, or until an insert under a key with the same index
/// and a different generation replaces it. `C`'s
/// [`ReplaceStrategy`](crate::SecondaryMapConfig::ReplaceStrategy) decides
/// whether such an insert replaces the value.
///
/// The map keeps a slot at every index up to the highest index that an insert
/// has used, in the [`SliceStorage`] its [`SecondaryMapConfig`] picks, the
/// same kind of storage a `GenMap` uses.
///
/// With the `alloc` feature, `C` defaults to [`DefaultMapConfig`]. A map
/// with that config keeps its slots in a `Vec` and uses
/// [`NewerWins`](crate::NewerWins) to decide whether an insert replaces a
/// value that was inserted under a different generation.
///
/// # Examples
///
/// ```
/// use gen_map::{GenMap, SecondaryMap};
///
/// let mut people = GenMap::new();
/// let mut ages = SecondaryMap::new();
///
/// let alice = people.insert("Alice");
/// let bob = people.insert("Bob");
/// ages.insert(alice, 30).unwrap();
///
/// assert_eq!(ages.get(alice), Some(&30));
/// assert_eq!(ages.get(bob), None);
/// ```
pub struct SecondaryMap<
    T,
    #[cfg(feature = "alloc")] C: SecondaryMapConfig = DefaultMapConfig,
    #[cfg(not(feature = "alloc"))] C: SecondaryMapConfig,
> {
    // The position and the generation of every slot that holds a value fit
    // the key config together. The map builds keys from its slots without
    // checks because of this.
    slots: Slots<T, C>,
    // `len` is the number of values. No slot has the largest value of the
    // index type as its index, so there are at most as many slots as that
    // largest value, and the count of values always fits in the index type.
    len: MapIdx<C>,
}

#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
impl<T> SecondaryMap<T> {
    /// Creates an empty map with the [`DefaultMapConfig`].
    ///
    /// This method only exists for the default config, so that
    /// `SecondaryMap::new()` compiles without a type annotation. Rust does not
    /// fall back to a default type parameter when it infers types, so a `new`
    /// for every config would leave the config unknown. Use
    /// [`new_with_config`](Self::new_with_config) for any other config.
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self::new_with_config()
    }

    /// Creates an empty map with the [`DefaultMapConfig`] and room for
    /// `capacity` slots.
    #[inline]
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self
    where
        Slots<T, DefaultMapConfig>: ReserveStorage,
    {
        Self {
            slots: Slots::<T, DefaultMapConfig>::with_capacity(capacity),
            len: MapIdx::<DefaultMapConfig>::ZERO,
        }
    }
}

impl<T, C: SecondaryMapConfig> SecondaryMap<T, C> {
    /// Creates an empty map with config `C`.
    #[inline]
    #[must_use]
    pub fn new_with_config() -> Self {
        Self {
            slots: Slots::<T, C>::empty(),
            len: MapIdx::<C>::ZERO,
        }
    }

    /// How many slots the storage can hold before it has to grow, or in total
    /// if it cannot grow.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.slots.capacity()
    }

    /// Returns the number of values in the map.
    #[inline]
    pub fn len(&self) -> usize {
        // SAFETY: every value has a slot of its own in the storage, so the
        // count is at most the number of slots, which is a `usize`.
        unsafe { self.len.into_usize_unchecked() }
    }

    /// Returns `true` if the map holds no values.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == MapIdx::<C>::ZERO
    }

    /// The number of slots, whether they hold a value or not.
    #[inline]
    pub fn slots_len(&self) -> usize {
        self.slots.len()
    }

    /// Returns `true` if a value is stored under `key`.
    #[inline]
    pub fn contains_key(&self, key: Key<MapKeyConfig<C>>) -> bool {
        self.get(key).is_some()
    }

    /// Returns a reference to the value stored under `key`, or `None` if
    /// there is none.
    #[inline]
    pub fn get(&self, key: Key<MapKeyConfig<C>>) -> Option<&T> {
        let position = key.idx().into_usize()?;
        self.slots
            .as_slice()
            .get(position)?
            .get_odd(key.generation())
    }

    /// Returns a mutable reference to the value stored under `key`, or
    /// `None` if there is none.
    #[inline]
    pub fn get_mut(&mut self, key: Key<MapKeyConfig<C>>) -> Option<&mut T> {
        let position = key.idx().into_usize()?;
        self.slots
            .as_mut_slice()
            .get_mut(position)?
            .get_odd_mut(key.generation())
    }

    /// Returns a reference to the value stored under `key`, like
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

    /// Returns a mutable reference to the value stored under `key`, like
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
            self.slots
                .as_mut_slice()
                .get_unchecked_mut(key.idx().into_usize_unchecked())
                .get_odd_unchecked_mut()
        }
    }

    /// Returns the key of the value in the slot at `idx`, or `None` if there
    /// is no slot at `idx` or the slot holds no value.
    #[inline]
    pub fn key_at(&self, idx: MapIdx<C>) -> Option<Key<MapKeyConfig<C>>> {
        let position = idx.into_usize()?;
        let (generation, _) = occupied(self.slots.as_slice().get(position)?)?;
        // SAFETY: the slot at `position` holds a value under `generation`, so
        // the two fit the key.
        Some(unsafe { key_from_parts_unchecked::<C>(position, generation) })
    }

    /// Returns the key of the value in the slot at `idx`, like
    /// [`key_at`](Self::key_at), but without checking that there is a slot at
    /// `idx` or that it holds a value.
    ///
    /// # Safety
    ///
    /// There must be a slot at `idx` and it must hold a value, meaning
    /// [`key_at`](Self::key_at) returns `Some` for it. A slot that holds no
    /// value has generation zero, and a key's generation is an
    /// [`Odd`](crate::Odd), so building a key from that generation is
    /// undefined behavior.
    #[inline]
    pub unsafe fn key_at_unchecked(&self, idx: MapIdx<C>) -> Key<MapKeyConfig<C>> {
        debug_assert!(self.key_at(idx).is_some());
        // SAFETY: the caller promises a slot at `idx` that holds a value. So
        // `idx` fits in `usize` and points at a slot in bounds, the slot's
        // generation is odd, and the position and generation fit the key.
        unsafe {
            let position = idx.into_usize_unchecked();
            let slot = self.slots.as_slice().get_unchecked(position);
            key_from_parts_unchecked::<C>(position, Odd::new_unchecked(slot.generation()))
        }
    }

    /// Returns the key and a reference to the value in the slot at `idx`, or
    /// `None` if there is no slot at `idx` or the slot holds no value.
    #[inline]
    pub fn get_at(&self, idx: MapIdx<C>) -> Option<(Key<MapKeyConfig<C>>, &T)> {
        let position = idx.into_usize()?;
        let (generation, value) = occupied(self.slots.as_slice().get(position)?)?;
        // SAFETY: the slot at `position` holds a value under `generation`, so
        // the two fit the key.
        let key = unsafe { key_from_parts_unchecked::<C>(position, generation) };
        Some((key, value))
    }

    /// Returns the key and a mutable reference to the value in the slot at
    /// `idx`, or `None` if there is no slot at `idx` or the slot holds no
    /// value. See [`get_at`](Self::get_at).
    #[inline]
    pub fn get_at_mut(&mut self, idx: MapIdx<C>) -> Option<(Key<MapKeyConfig<C>>, &mut T)> {
        let position = idx.into_usize()?;
        let (generation, value) = occupied_mut(self.slots.as_mut_slice().get_mut(position)?)?;
        // SAFETY: the slot at `position` holds a value under `generation`, so
        // the two fit the key.
        let key = unsafe { key_from_parts_unchecked::<C>(position, generation) };
        Some((key, value))
    }

    /// Returns the key and a reference to the value in the slot at `idx`, like
    /// [`get_at`](Self::get_at), but without checking that there is a slot at
    /// `idx` or that it holds a value.
    ///
    /// # Safety
    ///
    /// There must be a slot at `idx` and it must hold a value, meaning
    /// [`key_at`](Self::key_at) returns `Some` for it. Otherwise the slot is
    /// read as if it held a value and the key gets built using generation
    /// zero, both of which are undefined behavior.
    #[inline]
    pub unsafe fn get_at_unchecked(&self, idx: MapIdx<C>) -> (Key<MapKeyConfig<C>>, &T) {
        debug_assert!(self.key_at(idx).is_some());
        // SAFETY: the same as in `key_at_unchecked`. The slot holds a value,
        // so reading it as one is sound.
        unsafe {
            let position = idx.into_usize_unchecked();
            let slot = self.slots.as_slice().get_unchecked(position);
            let generation = Odd::new_unchecked(slot.generation());
            let key = key_from_parts_unchecked::<C>(position, generation);
            (key, slot.get_odd_unchecked())
        }
    }

    /// Returns the key and a mutable reference to the value in the slot at
    /// `idx`, like [`get_at_mut`](Self::get_at_mut), but without checking that
    /// there is a slot at `idx` or that it holds a value.
    ///
    /// # Safety
    ///
    /// The same as for [`get_at_unchecked`](Self::get_at_unchecked).
    #[inline]
    pub unsafe fn get_at_unchecked_mut(
        &mut self,
        idx: MapIdx<C>,
    ) -> (Key<MapKeyConfig<C>>, &mut T) {
        debug_assert!(self.key_at(idx).is_some());
        // SAFETY: the same as in `get_at_unchecked`.
        unsafe {
            let position = idx.into_usize_unchecked();
            let slot = self.slots.as_mut_slice().get_unchecked_mut(position);
            let generation = Odd::new_unchecked(slot.generation());
            let key = key_from_parts_unchecked::<C>(position, generation);
            (key, slot.get_odd_unchecked_mut())
        }
    }

    /// Returns the generation of the slot at `idx`, or `None` if there is no
    /// slot at `idx`.
    ///
    /// A slot that holds a value has the generation of the value's key, which
    /// is odd. A slot that holds no value has generation zero.
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
    /// [`generation_at`](Self::generation_at) returns `Some` for it. The
    /// slot does not have to hold a value.
    #[inline]
    pub unsafe fn generation_at_unchecked(&self, idx: MapIdx<C>) -> MapGen<C> {
        debug_assert!(self.generation_at(idx).is_some());
        // SAFETY: the caller promises a slot at `idx`, so `idx` fits in
        // `usize` and points at a slot in bounds.
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
        idxs: [MapIdx<C>; N],
    ) -> Result<[(Key<MapKeyConfig<C>>, &mut T); N], GetDisjointMutAtError> {
        for (i, idx) in idxs.iter().enumerate() {
            if self.key_at(*idx).is_none() {
                return Err(GetDisjointMutAtError::NoValue);
            }
            if idxs[..i].contains(idx) {
                return Err(GetDisjointMutAtError::OverlappingIndices);
            }
        }
        // SAFETY: every index was just found to point at a slot that holds
        // a value, and every index is distinct.
        Ok(unsafe { self.get_disjoint_mut_at_unchecked(idxs) })
    }

    /// Returns the key and a mutable reference to the value in the slot at each
    /// index, like [`get_disjoint_mut_at`](Self::get_disjoint_mut_at), but
    /// without any of its checks.
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
            // index points at the same slot, so the references do not alias.
            unsafe {
                let position = idx.into_usize_unchecked();
                let slot = &mut *slots.add(position);
                let generation = Odd::new_unchecked(slot.generation());
                let key = key_from_parts_unchecked::<C>(position, generation);
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
        keys: [Key<MapKeyConfig<C>>; N],
    ) -> Result<[&mut T; N], GetDisjointMutError> {
        for (i, key) in keys.iter().enumerate() {
            if !self.contains_key(*key) {
                return Err(GetDisjointMutError::InvalidKey);
            }
            // Keys point at the same slot exactly when their indices are
            // equal.
            if keys[..i].iter().any(|earlier| earlier.idx() == key.idx()) {
                return Err(GetDisjointMutError::OverlappingKeys);
            }
        }
        // SAFETY: every key was just found to have a value, and every index
        // is distinct.
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
    /// and no two keys may point at the same slot. Otherwise a slot is read
    /// as if it held a value, or two of the references point at the same
    /// value, either of which is undefined behavior.
    #[inline]
    pub unsafe fn get_disjoint_mut_unchecked<const N: usize>(
        &mut self,
        keys: [Key<MapKeyConfig<C>>; N],
    ) -> [&mut T; N] {
        debug_assert!(keys.iter().all(|key| self.contains_key(*key)));
        debug_assert!(keys
            .iter()
            .enumerate()
            .all(|(i, key)| keys[..i].iter().all(|earlier| earlier.idx() != key.idx())));
        let slots = self.slots.as_mut_slice().as_mut_ptr();
        keys.map(|key| {
            // SAFETY: the caller promises that the key has a value, so its
            // index fits in `usize` and points at a slot in bounds that holds
            // a value. The caller also promises that no other key points at
            // the same slot, so the references do not alias.
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
    /// kept the old value, the key's index is the largest value of the index
    /// type, or the storage could not make room for the slots. The
    /// [`SecondaryInsertError`] variant says which of the three happened.
    ///
    /// # Examples
    ///
    /// ```
    /// use gen_map::{GenMap, SecondaryInsertError, SecondaryMap};
    ///
    /// let mut people = GenMap::new();
    /// let mut ages = SecondaryMap::new();
    ///
    /// let alice = people.insert("Alice");
    /// assert_eq!(ages.insert(alice, 30).unwrap(), None);
    /// assert_eq!(ages.insert(alice, 31).unwrap(), Some(30));
    ///
    /// // Bob gets Alice's slot, with a newer generation, so his age replaces
    /// // hers, and Alice's old key can no longer replace his.
    /// people.remove(alice);
    /// let bob = people.insert("Bob");
    /// assert_eq!(bob.idx(), alice.idx());
    /// assert_eq!(ages.insert(bob, 25).unwrap(), Some(31));
    /// assert!(matches!(ages.insert(alice, 32), Err(SecondaryInsertError::Refused(32))));
    /// assert_eq!(ages[bob], 25);
    /// ```
    pub fn insert(&mut self, key: Key<MapKeyConfig<C>>, value: T) -> InsertResult<T, C> {
        // No `GenMap` gives a slot the largest value of the index type as its
        // index, so only a hand-built key can have that index. Rejecting it
        // caps the number of slots, and so the count of values, at that largest
        // value, so the count fits in the index type.
        if key.idx() == MapIdx::<C>::MAX {
            return Err(SecondaryInsertError::IndexReserved(value));
        }
        let generation = key.generation();
        let slot = match Self::get_or_grow_slot(&mut self.slots, key.idx()) {
            Ok(slot) => slot,
            Err(error) => return Err(SecondaryInsertError::StorageFull(value, error)),
        };
        if let Some(old) = slot.get_odd_mut(generation) {
            return Ok(Some(core::mem::replace(old, value)));
        }
        match occupied(slot).map(|(current, _)| current) {
            None => {
                replace(slot, generation, value);
                increment_len(&mut self.len);
                Ok(None)
            }
            Some(current)
                if <Strategy<C> as ReplaceStrategy<MapKeyConfig<C>>>::replaces(
                    current, generation,
                ) =>
            {
                Ok(replace(slot, generation, value))
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
        idx: MapIdx<C>,
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
                // `SliceStorage` promises that the pushes succeed after
                // that. A push that fails here means the storage broke that
                // promise.
                if slots.try_push(empty_slot()).is_err() {
                    panic!("SliceStorage::try_push failed although ensure_room returned Ok");
                }
            }
        }
        debug_assert!(position < slots.len());
        // SAFETY: the storage either had a slot at `position` already, or
        // the loop above pushed a slot for every position up to it.
        // `SliceStorage` promises that each push adds one slot at the end,
        // so `position` is in bounds.
        Ok(unsafe { slots.as_mut_slice().get_unchecked_mut(position) })
    }

    /// Removes the value stored under `key` and returns it, or returns
    /// `None` if there is none. The slot stays in the storage without a
    /// value.
    #[inline]
    pub fn remove(&mut self, key: Key<MapKeyConfig<C>>) -> Option<T> {
        let position = key.idx().into_usize()?;
        let slot = self.slots.as_mut_slice().get_mut(position)?;
        slot.get_odd(key.generation())?;
        let value = take(slot)?;
        decrement_len(&mut self.len);
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
    pub fn retain<F: FnMut(Key<MapKeyConfig<C>>, &mut T) -> bool>(&mut self, mut f: F) {
        for (position, slot) in self.slots.as_mut_slice().iter_mut().enumerate() {
            if let Some((generation, value)) = occupied_mut(slot) {
                // SAFETY: the slot at `position` holds a value under
                // `generation`, so the two fit the key.
                let key = unsafe { key_from_parts_unchecked::<C>(position, generation) };
                if !f(key, value) {
                    decrement_len(&mut self.len);
                    drop(take(slot));
                }
            }
        }
    }

    /// Removes every value. The slots stay, as they do after
    /// [`remove`](Self::remove).
    pub fn clear(&mut self) {
        for slot in self.slots.as_mut_slice() {
            if slot.is_odd() {
                decrement_len(&mut self.len);
                drop(take(slot));
            }
        }
    }

    /// Returns an iterator over the keys and references to the values, in
    /// index order.
    #[inline]
    pub fn iter(&self) -> SecondaryIter<'_, T, C> {
        let remaining = self.len();
        SecondaryIter {
            slots: self.slots.as_slice().iter().enumerate(),
            remaining,
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
        let remaining = self.len();
        SecondaryIterMut {
            slots: self.slots.as_mut_slice().iter_mut().enumerate(),
            remaining,
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
    /// keys, in index order. The slots stay, and dropping the iterator
    /// removes the values it has not reached.
    #[inline]
    pub fn drain(&mut self) -> SecondaryDrain<'_, T, C> {
        SecondaryDrain {
            map: self,
            position: 0,
        }
    }
}

/// These methods need a storage that can grow on request, so a map whose
/// storage has a fixed capacity does not have them.
impl<T, C: SecondaryMapConfig> SecondaryMap<T, C>
where
    Slots<T, C>: ReserveStorage,
{
    /// Creates an empty map with config `C` and room for `capacity` slots.
    #[inline]
    #[must_use]
    pub fn with_capacity_and_config(capacity: usize) -> Self {
        Self {
            slots: Slots::<T, C>::with_capacity(capacity),
            len: MapIdx::<C>::ZERO,
        }
    }

    /// Reserves room for at least `additional` more slots.
    ///
    /// # Panics
    ///
    /// Panics if the storage cannot make the room. Use
    /// [`try_reserve`](Self::try_reserve) to get an error instead.
    #[inline]
    pub fn reserve(&mut self, additional: usize) {
        if let Err(error) = self.try_reserve(additional) {
            panic!("SecondaryMap cannot make room for {additional} more slots: {error:?}");
        }
    }

    /// The fallible form of [`reserve`](Self::reserve). After it returns `Ok`,
    /// the map can add `additional` more slots without running out of storage.
    ///
    /// # Errors
    ///
    /// Returns the storage's error if the storage cannot make the room.
    #[inline]
    pub fn try_reserve(&mut self, additional: usize) -> Result<(), SecondaryStorageError<T, C>> {
        self.slots.ensure_room(additional)
    }
}

impl<T, C: SecondaryMapConfig> Default for SecondaryMap<T, C> {
    #[inline]
    fn default() -> Self {
        Self::new_with_config()
    }
}

impl<T: Clone, C: SecondaryMapConfig> Clone for SecondaryMap<T, C> {
    /// The clone has the same slots, so every key of the original works on
    /// it.
    fn clone(&self) -> Self {
        // This pushes a clone of each slot rather than calling the storage's
        // own `Clone`, which the `SliceStorage` contract does not cover.
        // The map builds keys from its slots without checks, which relies on
        // every slot being where `insert` put it.
        let mut slots = Slots::<T, C>::with_capacity(self.slots.len());
        for slot in self.slots.as_slice() {
            // A push only fails here for a storage that cannot hold as many
            // slots as another of its type.
            if slots.try_push(slot.clone()).is_err() {
                panic!("SliceStorage::try_push failed while cloning a storage of the same type");
            }
        }
        Self {
            slots,
            len: self.len,
        }
    }
}

impl<T: fmt::Debug, C: SecondaryMapConfig> fmt::Debug for SecondaryMap<T, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<T, C: SecondaryMapConfig> Index<Key<MapKeyConfig<C>>> for SecondaryMap<T, C> {
    type Output = T;

    /// Returns a reference to the value stored under `key`.
    ///
    /// # Panics
    ///
    /// Panics if no value is stored under `key`.
    #[inline]
    fn index(&self, key: Key<MapKeyConfig<C>>) -> &T {
        self.get(key).expect("invalid SecondaryMap key")
    }
}

impl<T, C: SecondaryMapConfig> IndexMut<Key<MapKeyConfig<C>>> for SecondaryMap<T, C> {
    /// Returns a mutable reference to the value stored under `key`.
    ///
    /// # Panics
    ///
    /// Panics if no value is stored under `key`.
    #[inline]
    fn index_mut(&mut self, key: Key<MapKeyConfig<C>>) -> &mut T {
        self.get_mut(key).expect("invalid SecondaryMap key")
    }
}

/// Iterator over `(key, &value)` pairs, in index order. It is created using
/// [`SecondaryMap::iter`].
pub struct SecondaryIter<'a, T, C: MapConfig> {
    slots: Enumerate<slice::Iter<'a, SecondaryMapSlot<T, C>>>,
    remaining: usize,
}

impl<'a, T, C: MapConfig> Iterator for SecondaryIter<'a, T, C> {
    type Item = (Key<MapKeyConfig<C>>, &'a T);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        for (position, slot) in self.slots.by_ref() {
            if let Some((generation, value)) = occupied(slot) {
                self.remaining -= 1;
                // SAFETY: the slot at `position` holds a value under
                // `generation`, so the two fit the key.
                let key = unsafe { key_from_parts_unchecked::<C>(position, generation) };
                return Some((key, value));
            }
        }
        None
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<T, C: MapConfig> DoubleEndedIterator for SecondaryIter<'_, T, C> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        while let Some((position, slot)) = self.slots.next_back() {
            if let Some((generation, value)) = occupied(slot) {
                self.remaining -= 1;
                // SAFETY: the slot at `position` holds a value under
                // `generation`, so the two fit the key.
                let key = unsafe { key_from_parts_unchecked::<C>(position, generation) };
                return Some((key, value));
            }
        }
        None
    }
}

impl<T, C: MapConfig> ExactSizeIterator for SecondaryIter<'_, T, C> {}
impl<T, C: MapConfig> FusedIterator for SecondaryIter<'_, T, C> {}

// This impl is written by hand, because a derive would require `T: Clone`
// and `C: Clone`.
impl<T, C: MapConfig> Clone for SecondaryIter<'_, T, C> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            slots: self.slots.clone(),
            remaining: self.remaining,
        }
    }
}

/// Iterator over `(key, &mut value)` pairs, in index order. It is created
/// using [`SecondaryMap::iter_mut`].
pub struct SecondaryIterMut<'a, T, C: MapConfig> {
    slots: Enumerate<slice::IterMut<'a, SecondaryMapSlot<T, C>>>,
    remaining: usize,
}

impl<'a, T, C: MapConfig> Iterator for SecondaryIterMut<'a, T, C> {
    type Item = (Key<MapKeyConfig<C>>, &'a mut T);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        for (position, slot) in self.slots.by_ref() {
            if let Some((generation, value)) = occupied_mut(slot) {
                self.remaining -= 1;
                // SAFETY: the slot at `position` holds a value under
                // `generation`, so the two fit the key.
                let key = unsafe { key_from_parts_unchecked::<C>(position, generation) };
                return Some((key, value));
            }
        }
        None
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<T, C: MapConfig> DoubleEndedIterator for SecondaryIterMut<'_, T, C> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        while let Some((position, slot)) = self.slots.next_back() {
            if let Some((generation, value)) = occupied_mut(slot) {
                self.remaining -= 1;
                // SAFETY: the slot at `position` holds a value under
                // `generation`, so the two fit the key.
                let key = unsafe { key_from_parts_unchecked::<C>(position, generation) };
                return Some((key, value));
            }
        }
        None
    }
}

impl<T, C: MapConfig> ExactSizeIterator for SecondaryIterMut<'_, T, C> {}
impl<T, C: MapConfig> FusedIterator for SecondaryIterMut<'_, T, C> {}

/// Iterator over keys, in index order. It is created using
/// [`SecondaryMap::keys`].
pub struct SecondaryKeys<'a, T, C: MapConfig> {
    inner: SecondaryIter<'a, T, C>,
}

impl<T, C: MapConfig> Iterator for SecondaryKeys<'_, T, C> {
    type Item = Key<MapKeyConfig<C>>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(key, _)| key)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<T, C: MapConfig> DoubleEndedIterator for SecondaryKeys<'_, T, C> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(|(key, _)| key)
    }
}

impl<T, C: MapConfig> ExactSizeIterator for SecondaryKeys<'_, T, C> {}
impl<T, C: MapConfig> FusedIterator for SecondaryKeys<'_, T, C> {}

impl<T, C: MapConfig> Clone for SecondaryKeys<'_, T, C> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

/// Iterator over references to the values, in index order. It is created
/// using [`SecondaryMap::values`].
pub struct SecondaryValues<'a, T, C: MapConfig> {
    inner: SecondaryIter<'a, T, C>,
}

impl<'a, T, C: MapConfig> Iterator for SecondaryValues<'a, T, C> {
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

impl<T, C: MapConfig> DoubleEndedIterator for SecondaryValues<'_, T, C> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(|(_, value)| value)
    }
}

impl<T, C: MapConfig> ExactSizeIterator for SecondaryValues<'_, T, C> {}
impl<T, C: MapConfig> FusedIterator for SecondaryValues<'_, T, C> {}

impl<T, C: MapConfig> Clone for SecondaryValues<'_, T, C> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

/// Iterator over mutable references to the values, in index order. It is
/// created using [`SecondaryMap::values_mut`].
pub struct SecondaryValuesMut<'a, T, C: MapConfig> {
    inner: SecondaryIterMut<'a, T, C>,
}

impl<'a, T, C: MapConfig> Iterator for SecondaryValuesMut<'a, T, C> {
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

impl<T, C: MapConfig> DoubleEndedIterator for SecondaryValuesMut<'_, T, C> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(|(_, value)| value)
    }
}

impl<T, C: MapConfig> ExactSizeIterator for SecondaryValuesMut<'_, T, C> {}
impl<T, C: MapConfig> FusedIterator for SecondaryValuesMut<'_, T, C> {}

/// Iterator that takes each value out, with its key, in index order. It is
/// created using [`SecondaryMap::drain`].
pub struct SecondaryDrain<'a, T, C: SecondaryMapConfig> {
    map: &'a mut SecondaryMap<T, C>,
    /// The position of the slot the iterator checks next.
    position: usize,
}

impl<T, C: SecondaryMapConfig> Iterator for SecondaryDrain<'_, T, C> {
    type Item = (Key<MapKeyConfig<C>>, T);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        while let Some(slot) = self.map.slots.as_mut_slice().get_mut(self.position) {
            let position = self.position;
            self.position += 1;
            if let Some((generation, value)) = into_parts(core::mem::replace(slot, empty_slot())) {
                decrement_len(&mut self.map.len);
                // SAFETY: the slot at `position` held this value under
                // `generation`, so the two fit the key.
                let key = unsafe { key_from_parts_unchecked::<C>(position, generation) };
                return Some((key, value));
            }
        }
        None
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.map.len();
        (len, Some(len))
    }
}

impl<T, C: SecondaryMapConfig> ExactSizeIterator for SecondaryDrain<'_, T, C> {}
impl<T, C: SecondaryMapConfig> FusedIterator for SecondaryDrain<'_, T, C> {}

impl<T, C: SecondaryMapConfig> Drop for SecondaryDrain<'_, T, C> {
    fn drop(&mut self) {
        for _ in self.by_ref() {}
    }
}

/// Owning iterator over `(key, value)` pairs, in index order. It is created
/// by consuming a map with `into_iter`, which a map only has when its storage
/// implements `IntoIterator`. It implements `DoubleEndedIterator`, which
/// gives it `next_back` and `rev`, only when the storage's iterator
/// implements both `DoubleEndedIterator` and `ExactSizeIterator`.
pub struct SecondaryIntoIter<T, C: SecondaryMapConfig>
where
    Slots<T, C>: IntoIterator<Item = SecondaryMapSlot<T, C>>,
{
    slots: Enumerate<<Slots<T, C> as IntoIterator>::IntoIter>,
    remaining: usize,
}

impl<T, C: SecondaryMapConfig> Iterator for SecondaryIntoIter<T, C>
where
    Slots<T, C>: IntoIterator<Item = SecondaryMapSlot<T, C>>,
{
    type Item = (Key<MapKeyConfig<C>>, T);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        for (position, slot) in self.slots.by_ref() {
            if let Some((generation, value)) = into_parts(slot) {
                self.remaining -= 1;
                // SAFETY: the slot at `position` holds a value under
                // `generation`, so the two fit the key.
                let key = unsafe { key_from_parts_unchecked::<C>(position, generation) };
                return Some((key, value));
            }
        }
        None
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

// `next_back` calls `Enumerate::next_back`, which works out the position of
// the last slot from the storage iterator's length, so that iterator has to
// implement `ExactSizeIterator` as well as `DoubleEndedIterator`.
impl<T, C: SecondaryMapConfig> DoubleEndedIterator for SecondaryIntoIter<T, C>
where
    Slots<T, C>: IntoIterator<Item = SecondaryMapSlot<T, C>>,
    <Slots<T, C> as IntoIterator>::IntoIter: DoubleEndedIterator + ExactSizeIterator,
{
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        while let Some((position, slot)) = self.slots.next_back() {
            if let Some((generation, value)) = into_parts(slot) {
                self.remaining -= 1;
                // SAFETY: the slot at `position` held this value under
                // `generation`, so the two fit the key.
                let key = unsafe { key_from_parts_unchecked::<C>(position, generation) };
                return Some((key, value));
            }
        }
        None
    }
}

impl<T, C: SecondaryMapConfig> ExactSizeIterator for SecondaryIntoIter<T, C> where
    Slots<T, C>: IntoIterator<Item = SecondaryMapSlot<T, C>>
{
}
impl<T, C: SecondaryMapConfig> FusedIterator for SecondaryIntoIter<T, C> where
    Slots<T, C>: IntoIterator<Item = SecondaryMapSlot<T, C>>
{
}

impl<T, C: SecondaryMapConfig> IntoIterator for SecondaryMap<T, C>
where
    Slots<T, C>: IntoIterator<Item = SecondaryMapSlot<T, C>>,
{
    type Item = (Key<MapKeyConfig<C>>, T);
    type IntoIter = SecondaryIntoIter<T, C>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        SecondaryIntoIter {
            remaining: self.len(),
            slots: self.slots.into_iter().enumerate(),
        }
    }
}

impl<'a, T, C: SecondaryMapConfig> IntoIterator for &'a SecondaryMap<T, C> {
    type Item = (Key<MapKeyConfig<C>>, &'a T);
    type IntoIter = SecondaryIter<'a, T, C>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T, C: SecondaryMapConfig> IntoIterator for &'a mut SecondaryMap<T, C> {
    type Item = (Key<MapKeyConfig<C>>, &'a mut T);
    type IntoIter = SecondaryIterMut<'a, T, C>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

/// `SecondaryMapRawParts` holds the fields of a [`SecondaryMap`].
/// [`into_raw_parts`](SecondaryMap::into_raw_parts) takes a map apart into
/// these fields, and [`from_raw_parts`](SecondaryMap::from_raw_parts) builds a
/// map from them.
///
/// The parts given to `from_raw_parts` must follow every rule below, and the
/// parts that `into_raw_parts` returns always do.
///
/// The values are the data that the map stores under its keys, and each one
/// sits in a slot. Editing or replacing a value in its slot never breaks a
/// rule, and neither does changing the capacity of `slots`.
///
/// # Rules
///
/// - No slot sits at a position above the largest index the map's keys can
///   hold, or at the largest value of the index type.
/// - A slot that holds a value has a generation no larger than the largest
///   one the map's keys can hold, and a slot that holds no value has
///   generation zero.
/// - `len` is the number of slots that hold a value.
///
/// # Examples
///
/// ```
/// use gen_map::{Even, GenMap, SecondaryMap, SecondaryMapRawParts, Slot};
///
/// let mut people = GenMap::new();
/// let alice = people.insert("Alice");
/// let bob = people.insert("Bob");
///
/// // Only Bob has an age, so the slot at Alice's index holds no value.
/// let parts = SecondaryMapRawParts {
///     slots: vec![Slot::new_even(Even::ZERO, ()), Slot::new_odd(bob.generation(), 25)],
///     len: 1,
/// };
/// // SAFETY: both slots sit at indices that a `GenMap` hands out, the slot
/// // without a value has generation zero, the slot that holds a value has the
/// // generation of a key, and `len` counts the one value.
/// let ages: SecondaryMap<u32> = unsafe { SecondaryMap::from_raw_parts(parts) };
/// assert_eq!(ages.get(alice), None);
/// assert_eq!(ages[bob], 25);
/// ```
pub struct SecondaryMapRawParts<
    T,
    #[cfg(feature = "alloc")] C: SecondaryMapConfig = DefaultMapConfig,
    #[cfg(not(feature = "alloc"))] C: SecondaryMapConfig,
> {
    // These fields are in the same order as the fields of `SecondaryMap` on
    // purpose, so that the two structs can be read side by side.
    // `into_raw_parts` and `from_raw_parts` list every field of both structs,
    // so the compiler catches a field that only one of them has, but nothing
    // catches a change in order. Think twice before removing this comment,
    // because it is the only thing that keeps the two orders the same.
    /// The map keeps its slots in this storage, which the map's config picks.
    /// A slot's index is its position in the storage.
    pub slots: <C as SecondaryMapConfig>::Storage<SecondaryMapSlot<T, C>>,
    /// `len` is the number of values in the map.
    pub len: MapIdx<C>,
}

impl<T, C: SecondaryMapConfig> SecondaryMap<T, C> {
    /// Takes the map apart into its fields.
    /// [`from_raw_parts`](Self::from_raw_parts) builds a map from them again,
    /// and [`SecondaryMapRawParts`] lists the rules they follow.
    #[inline]
    pub fn into_raw_parts(self) -> SecondaryMapRawParts<T, C> {
        let Self { slots, len } = self;
        SecondaryMapRawParts { slots, len }
    }

    /// Builds a map from the fields that
    /// [`into_raw_parts`](Self::into_raw_parts) takes a map apart into.
    ///
    /// # Safety
    ///
    /// `parts` must follow every rule listed on [`SecondaryMapRawParts`]. The
    /// map's methods rely on those rules, and parts that break one can make
    /// them cause undefined behavior.
    #[inline]
    #[must_use]
    pub unsafe fn from_raw_parts(parts: SecondaryMapRawParts<T, C>) -> Self {
        let SecondaryMapRawParts { slots, len } = parts;
        Self { slots, len }
    }
}
