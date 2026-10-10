use crate::config::{DefaultMapConfig, KeyConfig, MapConfig, SparseSecondaryMapConfig};
use crate::error::{
    check_disjoint_idxs, check_disjoint_keys, GetDisjointMutAtError, GetDisjointMutError,
    SecondaryInsertError,
};
use crate::key::parity::Odd;
use crate::key::piece::KeyPiece;
use crate::key::Key;
use crate::map::{MapGen, MapIdx, MapKeyConfig};
use crate::secondary::replace_strategy::ReplaceStrategy;
use core::fmt;
use core::hash::BuildHasher;
use core::iter::FusedIterator;
use core::ops::{Index, IndexMut};
use std::collections::hash_map::{self, HashMap};
use std::collections::TryReserveError;
use std::hash::RandomState;

/// A [`SparseSecondaryMap`] keeps each value in a `SparseSlot`, together with
/// the generation of the key the value is stored under.
#[derive(Clone)]
pub struct SparseSlot<G: KeyPiece, T> {
    generation: Odd<G>,
    value: T,
}

/// A map with config `C` and hasher `S` keeps its values in this `HashMap`,
/// under the indices of their keys.
type Slots<T, C, S> = HashMap<MapIdx<C>, SparseSlot<MapGen<C>, T>, S>;

/// `Strategy<C>` is the replace strategy that config `C` gives the map.
type Strategy<C> = <C as SparseSecondaryMapConfig>::ReplaceStrategy;

/// [`SparseSecondaryMap::insert`] returns this type.
type InsertResult<T> = Result<Option<T>, SecondaryInsertError<T, TryReserveError>>;

/// Builds the key whose index is `idx` and whose generation is `generation`,
/// without checking that they fit.
///
/// # Safety
///
/// `idx` and `generation` must fit the key config together.
#[inline]
unsafe fn key_from_parts_unchecked<C: MapConfig>(
    idx: MapIdx<C>,
    generation: Odd<MapGen<C>>,
) -> Key<MapKeyConfig<C>> {
    debug_assert!(
        <MapKeyConfig<C> as KeyConfig>::pack(idx, generation).is_some(),
        "the index and the generation fit the key"
    );
    // SAFETY: the caller promises that `idx` and `generation` fit the key
    // config.
    Key::from_repr(unsafe { <MapKeyConfig<C> as KeyConfig>::pack_unchecked(idx, generation) })
}

/// A `SparseSecondaryMap` maps the keys of a [`GenMap`](crate::GenMap) to
/// values of type `T`, and keeps its values in a `HashMap`. `C` configures the
/// map, and `S` is the hasher of the `HashMap`.
///
/// It accepts the same keys as a [`SecondaryMap`](crate::SecondaryMap) with
/// the same key config, and the [`ReplaceStrategy`](crate::ReplaceStrategy)
/// of its config decides whether an insert replaces a value.
/// The difference is where the values live. A `SecondaryMap` keeps a
/// slot at every index up to the highest index that an insert has used, even
/// where no value is stored. A `SparseSecondaryMap` keeps each value in a
/// `HashMap` under the index of its key, together with the key's generation,
/// and keeps nothing for an index without a value. So it uses less memory than
/// a `SecondaryMap` when only a few of a `GenMap`'s keys have a value in it. A
/// lookup hashes the key's index, so it takes longer than a lookup in a
/// `SecondaryMap`.
///
/// `C` defaults to [`DefaultMapConfig`], whose
/// [`NewerWins`](crate::NewerWins) strategy lets an insert under a larger
/// generation replace a value stored under a smaller one. `S` defaults to
/// std's `RandomState`. The map needs the `std` feature.
///
/// # Examples
///
/// ```
/// use gen_map::{GenMap, SparseSecondaryMap};
///
/// let mut people = GenMap::new();
/// let mut ages = SparseSecondaryMap::new();
///
/// let alice = people.insert("Alice");
/// let bob = people.insert("Bob");
/// ages.insert(alice, 30).unwrap();
///
/// assert_eq!(ages.get(alice), Some(&30));
/// assert_eq!(ages.get(bob), None);
/// ```
#[cfg_attr(docsrs, doc(cfg(feature = "std")))]
pub struct SparseSecondaryMap<T, C: SparseSecondaryMapConfig = DefaultMapConfig, S = RandomState> {
    /// The `HashMap` holds each value under the index of its key, together with
    /// that key's generation, so the map builds keys from them without checks.
    slots: Slots<T, C, S>,
}

impl<T> SparseSecondaryMap<T> {
    /// Creates an empty map with the [`DefaultMapConfig`] and a `RandomState`
    /// hasher.
    ///
    /// This method only exists for the default config and hasher, so that
    /// `SparseSecondaryMap::new()` compiles without a type annotation. Use
    /// [`new_with_config`](Self::new_with_config) for any other config.
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self::new_with_config()
    }

    /// Creates an empty map with the [`DefaultMapConfig`], a `RandomState`
    /// hasher and room for `capacity` values.
    #[inline]
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self::with_capacity_and_config(capacity)
    }
}

impl<T, S: BuildHasher> SparseSecondaryMap<T, DefaultMapConfig, S> {
    /// Creates an empty map with the [`DefaultMapConfig`]. The map hashes
    /// indices with `hasher`.
    ///
    /// This method only exists for the default config, so that
    /// `SparseSecondaryMap::with_hasher(hasher)` compiles without a type
    /// annotation. Use [`with_hasher_and_config`](Self::with_hasher_and_config)
    /// for any other config.
    ///
    /// # Examples
    ///
    /// ```
    /// use gen_map::{GenMap, SparseSecondaryMap};
    /// use std::hash::{BuildHasherDefault, DefaultHasher};
    ///
    /// let hasher = BuildHasherDefault::<DefaultHasher>::default();
    /// let mut keys = GenMap::new();
    /// let mut map = SparseSecondaryMap::with_hasher(hasher);
    /// let key = keys.insert(());
    /// map.insert(key, "a").unwrap();
    /// assert_eq!(map[key], "a");
    /// ```
    #[inline]
    #[must_use]
    pub fn with_hasher(hasher: S) -> Self {
        Self::with_hasher_and_config(hasher)
    }

    /// Creates an empty map with the [`DefaultMapConfig`] and room for
    /// `capacity` values. The map hashes indices with `hasher`.
    #[inline]
    #[must_use]
    pub fn with_capacity_and_hasher(capacity: usize, hasher: S) -> Self {
        Self::with_capacity_and_hasher_and_config(capacity, hasher)
    }
}

impl<T, C: SparseSecondaryMapConfig, S: BuildHasher + Default> SparseSecondaryMap<T, C, S> {
    /// Creates an empty map with config `C`. The map hashes indices with
    /// `S::default()`.
    #[inline]
    #[must_use]
    pub fn new_with_config() -> Self {
        Self::with_hasher_and_config(S::default())
    }

    /// Creates an empty map with config `C` and room for `capacity` values.
    /// The map hashes indices with `S::default()`.
    #[inline]
    #[must_use]
    pub fn with_capacity_and_config(capacity: usize) -> Self {
        Self::with_capacity_and_hasher_and_config(capacity, S::default())
    }
}

impl<T, C: SparseSecondaryMapConfig, S: BuildHasher> SparseSecondaryMap<T, C, S> {
    /// Creates an empty map with config `C`. The map hashes indices with
    /// `hasher`.
    #[inline]
    #[must_use]
    pub fn with_hasher_and_config(hasher: S) -> Self {
        Self {
            slots: HashMap::with_hasher(hasher),
        }
    }

    /// Creates an empty map with config `C` and room for `capacity` values.
    /// The map hashes indices with `hasher`.
    #[inline]
    #[must_use]
    pub fn with_capacity_and_hasher_and_config(capacity: usize, hasher: S) -> Self {
        Self {
            slots: HashMap::with_capacity_and_hasher(capacity, hasher),
        }
    }

    /// Returns the hasher the map hashes indices with.
    #[inline]
    pub fn hasher(&self) -> &S {
        self.slots.hasher()
    }

    /// Returns how many values the map can hold before it has to allocate
    /// more room.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.slots.capacity()
    }

    /// Returns the number of values in the map.
    #[inline]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Returns `true` if the map holds no values.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
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
        let slot = self.slots.get(&key.idx())?;
        (slot.generation == key.generation()).then_some(&slot.value)
    }

    /// Returns a mutable reference to the value stored under `key`, or `None`
    /// if there is none.
    #[inline]
    pub fn get_mut(&mut self, key: Key<MapKeyConfig<C>>) -> Option<&mut T> {
        let slot = self.slots.get_mut(&key.idx())?;
        (slot.generation == key.generation()).then_some(&mut slot.value)
    }

    /// Returns the key of the value at index `idx`, or `None` if there is no
    /// value at that index.
    #[inline]
    pub fn key_at(&self, idx: MapIdx<C>) -> Option<Key<MapKeyConfig<C>>> {
        let slot = self.slots.get(&idx)?;
        // SAFETY: `idx` and the slot's generation are the index and the
        // generation of the key that the slot's value is stored under, so they
        // fit the key config.
        Some(unsafe { key_from_parts_unchecked::<C>(idx, slot.generation) })
    }

    /// Returns the key and a reference to the value at index `idx`, or `None`
    /// if there is no value at that index.
    #[inline]
    pub fn get_at(&self, idx: MapIdx<C>) -> Option<(Key<MapKeyConfig<C>>, &T)> {
        let slot = self.slots.get(&idx)?;
        // SAFETY: `idx` and the slot's generation are the index and the
        // generation of the key that the slot's value is stored under, so they
        // fit the key config.
        let key = unsafe { key_from_parts_unchecked::<C>(idx, slot.generation) };
        Some((key, &slot.value))
    }

    /// Returns the key and a mutable reference to the value at index `idx`,
    /// or `None` if there is no value at that index.
    #[inline]
    pub fn get_at_mut(&mut self, idx: MapIdx<C>) -> Option<(Key<MapKeyConfig<C>>, &mut T)> {
        let slot = self.slots.get_mut(&idx)?;
        // SAFETY: `idx` and the slot's generation are the index and the
        // generation of the key that the slot's value is stored under, so they
        // fit the key config.
        let key = unsafe { key_from_parts_unchecked::<C>(idx, slot.generation) };
        Some((key, &mut slot.value))
    }

    /// Returns the key and a mutable reference to the value at each index,
    /// all at once. It checks the indices the same way
    /// [`SecondaryMap::get_disjoint_mut_at`](crate::SecondaryMap::get_disjoint_mut_at)
    /// does.
    ///
    /// # Errors
    ///
    /// Returns [`GetDisjointMutAtError::NoValue`] if there is no value at one
    /// of the indices, and [`GetDisjointMutAtError::OverlappingIndices`] if
    /// two indices are the same.
    #[inline]
    #[allow(clippy::type_complexity)]
    pub fn get_disjoint_mut_at<const N: usize>(
        &mut self,
        idxs: [MapIdx<C>; N],
    ) -> Result<[(Key<MapKeyConfig<C>>, &mut T); N], GetDisjointMutAtError> {
        check_disjoint_idxs(&idxs, |idx| self.slots.contains_key(idx))?;
        // SAFETY: `check_disjoint_idxs` found that no two of the indices are
        // the same.
        let mut slots = unsafe { self.slots.get_disjoint_unchecked_mut(idxs.each_ref()) };
        // `check_disjoint_idxs` found a value at every index, so the `HashMap`
        // only misses one here if the hasher gave an index a different hash.
        if slots.iter().any(Option::is_none) {
            return Err(GetDisjointMutAtError::NoValue);
        }
        Ok(core::array::from_fn(|i| {
            // SAFETY: `from_fn` calls this closure once with each index below
            // `N`, so `i` is in bounds of both arrays and each slot is taken
            // once. Every slot was just found to be `Some`.
            let (idx, slot) = unsafe {
                (
                    *idxs.get_unchecked(i),
                    slots.get_unchecked_mut(i).take().unwrap_unchecked(),
                )
            };
            // SAFETY: `idx` and the slot's generation are the index and the
            // generation of the key that the slot's value is stored under, so
            // they fit the key config.
            let key = unsafe { key_from_parts_unchecked::<C>(idx, slot.generation) };
            (key, &mut slot.value)
        }))
    }

    /// Returns the key and a mutable reference to the value at each index,
    /// like [`get_disjoint_mut_at`](Self::get_disjoint_mut_at), but without
    /// any of its checks.
    ///
    /// # Safety
    ///
    /// There must be a value at every index, meaning [`key_at`](Self::key_at)
    /// returns `Some` for every one of them, and no two of the indices may be
    /// the same. The map's hasher must also give each index the same hash every
    /// time, which every correct `BuildHasher` does.
    #[inline]
    pub unsafe fn get_disjoint_mut_at_unchecked<const N: usize>(
        &mut self,
        idxs: [MapIdx<C>; N],
    ) -> [(Key<MapKeyConfig<C>>, &mut T); N] {
        debug_assert!(check_disjoint_idxs(&idxs, |idx| self.slots.contains_key(idx)).is_ok());
        // SAFETY: the caller promises that no two of the indices are the same.
        let mut slots = unsafe { self.slots.get_disjoint_unchecked_mut(idxs.each_ref()) };
        core::array::from_fn(|i| {
            // SAFETY: `from_fn` calls this closure once with each index below
            // `N`, so `i` is in bounds of both arrays and each slot is taken
            // once. The caller promises a value at every index and a hasher
            // that gives each index the same hash as before, so the `HashMap`
            // found every value.
            let (idx, slot) = unsafe {
                (
                    *idxs.get_unchecked(i),
                    slots.get_unchecked_mut(i).take().unwrap_unchecked(),
                )
            };
            // SAFETY: `idx` and the slot's generation are the index and the
            // generation of the key that the slot's value is stored under, so
            // they fit the key config.
            let key = unsafe { key_from_parts_unchecked::<C>(idx, slot.generation) };
            (key, &mut slot.value)
        })
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
    /// two keys have the same index.
    ///
    /// # Examples
    ///
    /// ```
    /// use gen_map::{GenMap, GetDisjointMutError, SparseSecondaryMap};
    ///
    /// let mut keys = GenMap::new();
    /// let a = keys.insert(());
    /// let b = keys.insert(());
    /// let mut map = SparseSecondaryMap::new();
    /// map.insert(a, 1).unwrap();
    /// map.insert(b, 2).unwrap();
    ///
    /// let [x, y] = map.get_disjoint_mut([a, b]).unwrap();
    /// core::mem::swap(x, y);
    /// assert_eq!((map[a], map[b]), (2, 1));
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
        check_disjoint_keys(&keys, |key| self.contains_key(key))?;
        let idxs = keys.map(|key| key.idx());
        // SAFETY: `check_disjoint_keys` found that no two keys have the same
        // index.
        let slots = unsafe { self.slots.get_disjoint_unchecked_mut(idxs.each_ref()) };
        // `check_disjoint_keys` found every key's value, so the `HashMap` only
        // misses one here if the hasher gave an index a different hash.
        if slots.iter().any(Option::is_none) {
            return Err(GetDisjointMutError::InvalidKey);
        }
        Ok(slots.map(|slot| {
            // SAFETY: every slot was just found to be `Some`.
            unsafe { &mut slot.unwrap_unchecked().value }
        }))
    }

    /// Returns a mutable reference to the value stored under each key, like
    /// [`get_disjoint_mut`](Self::get_disjoint_mut), but without any of its
    /// checks.
    ///
    /// # Safety
    ///
    /// A value must be stored under every key, meaning
    /// [`contains_key`](Self::contains_key) returns `true` for each of them,
    /// and no two keys may have the same index. The map's hasher must also give
    /// each index the same hash every time, which every correct `BuildHasher`
    /// does.
    #[inline]
    pub unsafe fn get_disjoint_mut_unchecked<const N: usize>(
        &mut self,
        keys: [Key<MapKeyConfig<C>>; N],
    ) -> [&mut T; N] {
        debug_assert!(check_disjoint_keys(&keys, |key| self.contains_key(key)).is_ok());
        let idxs = keys.map(|key| key.idx());
        // SAFETY: the caller promises that no two keys have the same index.
        let slots = unsafe { self.slots.get_disjoint_unchecked_mut(idxs.each_ref()) };
        slots.map(|slot| {
            // SAFETY: the caller promises that a value is stored under every
            // key and that the hasher gives each index the same hash as before,
            // so the `HashMap` found every value.
            unsafe { &mut slot.unwrap_unchecked().value }
        })
    }

    /// Stores `value` under `key`.
    ///
    /// - If no value is stored at the key's index, the value goes in and
    ///   `Ok(None)` is returned.
    /// - If a value is stored under the same key, that value is replaced and
    ///   returned as `Ok(Some(old))`.
    /// - If the value stored at the key's index was inserted under a different
    ///   generation, `C`'s
    ///   [`ReplaceStrategy`](SparseSecondaryMapConfig::ReplaceStrategy)
    ///   decides whether `value` replaces it. If it does, the old value is
    ///   returned as `Ok(Some(old))`, and the old value's key stops matching
    ///   anything in the map.
    ///
    /// # Errors
    ///
    /// Hands `value` back, and leaves the values in the map as they were, if
    /// the strategy kept the old value, the key's index is the largest value of
    /// the index type, or the `HashMap` could not make room for one more value.
    /// The map makes that room before it looks the index up, even when `value`
    /// ends up replacing a value. The [`SecondaryInsertError`] variant says
    /// which of the three happened.
    ///
    /// # Examples
    ///
    /// ```
    /// use gen_map::{GenMap, SecondaryInsertError, SparseSecondaryMap};
    ///
    /// let mut people = GenMap::new();
    /// let mut ages = SparseSecondaryMap::new();
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
    pub fn insert(&mut self, key: Key<MapKeyConfig<C>>, value: T) -> InsertResult<T> {
        // No `GenMap` gives a slot the largest value of the index type as its
        // index, so only a hand-built key can have that index. The map refuses
        // such a key, as the other secondary maps do.
        if key.idx() == MapIdx::<C>::MAX {
            return Err(SecondaryInsertError::IndexReserved(value));
        }
        // `HashMap::entry` makes room for one more value when it finds no value
        // at the index, and it panics or aborts if it cannot. The map makes
        // room first, so it can hand `value` back instead.
        if let Err(error) = self.slots.try_reserve(1) {
            return Err(SecondaryInsertError::StorageFull(value, error));
        }
        let generation = key.generation();
        match self.slots.entry(key.idx()) {
            hash_map::Entry::Occupied(mut entry) => {
                let slot = entry.get_mut();
                if slot.generation != generation
                    && !<Strategy<C> as ReplaceStrategy<MapKeyConfig<C>>>::replaces(
                        slot.generation,
                        generation,
                    )
                {
                    return Err(SecondaryInsertError::Refused(value));
                }
                slot.generation = generation;
                Ok(Some(core::mem::replace(&mut slot.value, value)))
            }
            hash_map::Entry::Vacant(entry) => {
                entry.insert(SparseSlot { generation, value });
                Ok(None)
            }
        }
    }

    /// Removes the value stored under `key` and returns it, or returns `None`
    /// if there is none.
    #[inline]
    pub fn remove(&mut self, key: Key<MapKeyConfig<C>>) -> Option<T> {
        let idx = key.idx();
        // The map checks the generation with `get` before it calls `remove`,
        // and neither call makes room in the `HashMap`, so removing a key
        // without a value never allocates.
        if self.slots.get(&idx)?.generation != key.generation() {
            return None;
        }
        self.slots.remove(&idx).map(|slot| slot.value)
    }

    /// Keeps only the values `f` returns `true` for, and removes the rest.
    /// It calls `f` once for each value, in arbitrary order.
    pub fn retain<F: FnMut(Key<MapKeyConfig<C>>, &mut T) -> bool>(&mut self, mut f: F) {
        self.slots.retain(|&idx, slot| {
            // SAFETY: `idx` and the slot's generation are the index and the
            // generation of the key that the slot's value is stored under, so
            // they fit the key config.
            let key = unsafe { key_from_parts_unchecked::<C>(idx, slot.generation) };
            f(key, &mut slot.value)
        });
    }

    /// Removes every value. The map keeps the memory it has allocated.
    #[inline]
    pub fn clear(&mut self) {
        self.slots.clear();
    }

    /// Returns an iterator over the keys and references to the values, in
    /// arbitrary order.
    #[inline]
    pub fn iter(&self) -> SparseSecondaryIter<'_, T, C> {
        SparseSecondaryIter {
            slots: self.slots.iter(),
        }
    }

    /// Returns an iterator over the keys, in arbitrary order.
    #[inline]
    pub fn keys(&self) -> SparseSecondaryKeys<'_, T, C> {
        SparseSecondaryKeys { inner: self.iter() }
    }

    /// Returns an iterator over references to the values, in arbitrary order.
    #[inline]
    pub fn values(&self) -> SparseSecondaryValues<'_, T, C> {
        SparseSecondaryValues {
            slots: self.slots.values(),
        }
    }

    /// Returns an iterator over the keys and mutable references to the
    /// values, in arbitrary order.
    #[inline]
    pub fn iter_mut(&mut self) -> SparseSecondaryIterMut<'_, T, C> {
        SparseSecondaryIterMut {
            slots: self.slots.iter_mut(),
        }
    }

    /// Returns an iterator over mutable references to the values, in
    /// arbitrary order.
    #[inline]
    pub fn values_mut(&mut self) -> SparseSecondaryValuesMut<'_, T, C> {
        SparseSecondaryValuesMut {
            slots: self.slots.values_mut(),
        }
    }

    /// Removes every value, and returns an iterator over them and their keys,
    /// in arbitrary order. The map keeps the memory it has allocated, and
    /// dropping the iterator removes the values it has not reached.
    #[inline]
    pub fn drain(&mut self) -> SparseSecondaryDrain<'_, T, C> {
        SparseSecondaryDrain {
            slots: self.slots.drain(),
        }
    }

    /// Reserves room for at least `additional` more values.
    ///
    /// # Panics
    ///
    /// Panics if the map cannot make the room. Use
    /// [`try_reserve`](Self::try_reserve) to get an error instead.
    #[inline]
    pub fn reserve(&mut self, additional: usize) {
        if let Err(error) = self.try_reserve(additional) {
            panic!("SparseSecondaryMap cannot make room for {additional} more values: {error:?}");
        }
    }

    /// Reserves room for at least `additional` more values, like
    /// [`reserve`](Self::reserve), but returns an error instead of panicking.
    /// After it returns `Ok`, the map can take `additional` more values
    /// without allocating.
    ///
    /// # Errors
    ///
    /// Returns the `HashMap`'s error if it cannot make the room.
    #[inline]
    pub fn try_reserve(&mut self, additional: usize) -> Result<(), TryReserveError> {
        self.slots.try_reserve(additional)
    }

    /// Shrinks the map's capacity as much as possible. The map keeps room for
    /// every value it holds, and may keep a little more.
    #[inline]
    pub fn shrink_to_fit(&mut self) {
        self.slots.shrink_to_fit();
    }

    /// Shrinks the map's capacity, but not below `min_capacity`. The map keeps
    /// room for every value it holds, and may keep a little more. If the
    /// capacity is already below `min_capacity`, nothing changes.
    #[inline]
    pub fn shrink_to(&mut self, min_capacity: usize) {
        self.slots.shrink_to(min_capacity);
    }
}

impl<T, C: SparseSecondaryMapConfig, S: BuildHasher + Default> Default
    for SparseSecondaryMap<T, C, S>
{
    #[inline]
    fn default() -> Self {
        Self::new_with_config()
    }
}

impl<T: Clone, C: SparseSecondaryMapConfig, S: Clone> Clone for SparseSecondaryMap<T, C, S> {
    /// The clone holds a clone of each value under the same key, so every key
    /// of the original works on it.
    #[inline]
    fn clone(&self) -> Self {
        Self {
            slots: self.slots.clone(),
        }
    }

    /// Clones the slots of `source` with the `clone_from` of this map's
    /// `HashMap`.
    #[inline]
    fn clone_from(&mut self, source: &Self) {
        self.slots.clone_from(&source.slots);
    }
}

impl<T: fmt::Debug, C: SparseSecondaryMapConfig, S: BuildHasher> fmt::Debug
    for SparseSecondaryMap<T, C, S>
{
    /// Lists every key with its value, in arbitrary order.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<T, C: SparseSecondaryMapConfig, S: BuildHasher> Index<Key<MapKeyConfig<C>>>
    for SparseSecondaryMap<T, C, S>
{
    type Output = T;

    /// Returns a reference to the value stored under `key`.
    ///
    /// # Panics
    ///
    /// Panics if no value is stored under `key`. Use [`get`](Self::get) to get
    /// `None` instead.
    #[inline]
    fn index(&self, key: Key<MapKeyConfig<C>>) -> &T {
        self.get(key).expect("invalid SparseSecondaryMap key")
    }
}

impl<T, C: SparseSecondaryMapConfig, S: BuildHasher> IndexMut<Key<MapKeyConfig<C>>>
    for SparseSecondaryMap<T, C, S>
{
    /// Returns a mutable reference to the value stored under `key`.
    ///
    /// # Panics
    ///
    /// Panics if no value is stored under `key`. Use
    /// [`get_mut`](Self::get_mut) to get `None` instead.
    #[inline]
    fn index_mut(&mut self, key: Key<MapKeyConfig<C>>) -> &mut T {
        self.get_mut(key).expect("invalid SparseSecondaryMap key")
    }
}

impl<T, C: SparseSecondaryMapConfig, S> IntoIterator for SparseSecondaryMap<T, C, S> {
    type Item = (Key<MapKeyConfig<C>>, T);
    type IntoIter = SparseSecondaryIntoIter<T, C>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        SparseSecondaryIntoIter {
            slots: self.slots.into_iter(),
        }
    }
}

impl<'a, T, C: SparseSecondaryMapConfig, S: BuildHasher> IntoIterator
    for &'a SparseSecondaryMap<T, C, S>
{
    type Item = (Key<MapKeyConfig<C>>, &'a T);
    type IntoIter = SparseSecondaryIter<'a, T, C>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T, C: SparseSecondaryMapConfig, S: BuildHasher> IntoIterator
    for &'a mut SparseSecondaryMap<T, C, S>
{
    type Item = (Key<MapKeyConfig<C>>, &'a mut T);
    type IntoIter = SparseSecondaryIterMut<'a, T, C>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

/// This iterator yields each key of a [`SparseSecondaryMap`] together with a
/// reference to its value, in arbitrary order. It is created using
/// [`SparseSecondaryMap::iter`].
pub struct SparseSecondaryIter<'a, T, C: MapConfig> {
    slots: hash_map::Iter<'a, MapIdx<C>, SparseSlot<MapGen<C>, T>>,
}

impl<'a, T, C: MapConfig> Iterator for SparseSecondaryIter<'a, T, C> {
    type Item = (Key<MapKeyConfig<C>>, &'a T);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let (&idx, slot) = self.slots.next()?;
        // SAFETY: `idx` and the slot's generation are the index and the
        // generation of the key that the slot's value is stored under, so they
        // fit the key config.
        let key = unsafe { key_from_parts_unchecked::<C>(idx, slot.generation) };
        Some((key, &slot.value))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.slots.size_hint()
    }
}

impl<T, C: MapConfig> ExactSizeIterator for SparseSecondaryIter<'_, T, C> {}
impl<T, C: MapConfig> FusedIterator for SparseSecondaryIter<'_, T, C> {}

// This impl is written by hand, because a derive would require `T: Clone`
// and `C: Clone`.
impl<T, C: MapConfig> Clone for SparseSecondaryIter<'_, T, C> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            slots: self.slots.clone(),
        }
    }
}

/// This iterator yields each key of a [`SparseSecondaryMap`] together with a
/// mutable reference to its value, in arbitrary order. It is created using
/// [`SparseSecondaryMap::iter_mut`].
pub struct SparseSecondaryIterMut<'a, T, C: MapConfig> {
    slots: hash_map::IterMut<'a, MapIdx<C>, SparseSlot<MapGen<C>, T>>,
}

impl<'a, T, C: MapConfig> Iterator for SparseSecondaryIterMut<'a, T, C> {
    type Item = (Key<MapKeyConfig<C>>, &'a mut T);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let (&idx, slot) = self.slots.next()?;
        // SAFETY: `idx` and the slot's generation are the index and the
        // generation of the key that the slot's value is stored under, so they
        // fit the key config.
        let key = unsafe { key_from_parts_unchecked::<C>(idx, slot.generation) };
        Some((key, &mut slot.value))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.slots.size_hint()
    }
}

impl<T, C: MapConfig> ExactSizeIterator for SparseSecondaryIterMut<'_, T, C> {}
impl<T, C: MapConfig> FusedIterator for SparseSecondaryIterMut<'_, T, C> {}

/// This iterator yields the keys of a [`SparseSecondaryMap`], in arbitrary
/// order. It is created using [`SparseSecondaryMap::keys`].
pub struct SparseSecondaryKeys<'a, T, C: MapConfig> {
    inner: SparseSecondaryIter<'a, T, C>,
}

impl<T, C: MapConfig> Iterator for SparseSecondaryKeys<'_, T, C> {
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

impl<T, C: MapConfig> ExactSizeIterator for SparseSecondaryKeys<'_, T, C> {}
impl<T, C: MapConfig> FusedIterator for SparseSecondaryKeys<'_, T, C> {}

impl<T, C: MapConfig> Clone for SparseSecondaryKeys<'_, T, C> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

/// This iterator yields references to the values of a
/// [`SparseSecondaryMap`], in arbitrary order. It is created using
/// [`SparseSecondaryMap::values`].
pub struct SparseSecondaryValues<'a, T, C: MapConfig> {
    slots: hash_map::Values<'a, MapIdx<C>, SparseSlot<MapGen<C>, T>>,
}

impl<'a, T, C: MapConfig> Iterator for SparseSecondaryValues<'a, T, C> {
    type Item = &'a T;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.slots.next().map(|slot| &slot.value)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.slots.size_hint()
    }
}

impl<T, C: MapConfig> ExactSizeIterator for SparseSecondaryValues<'_, T, C> {}
impl<T, C: MapConfig> FusedIterator for SparseSecondaryValues<'_, T, C> {}

impl<T, C: MapConfig> Clone for SparseSecondaryValues<'_, T, C> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            slots: self.slots.clone(),
        }
    }
}

/// This iterator yields mutable references to the values of a
/// [`SparseSecondaryMap`], in arbitrary order. It is created using
/// [`SparseSecondaryMap::values_mut`].
pub struct SparseSecondaryValuesMut<'a, T, C: MapConfig> {
    slots: hash_map::ValuesMut<'a, MapIdx<C>, SparseSlot<MapGen<C>, T>>,
}

impl<'a, T, C: MapConfig> Iterator for SparseSecondaryValuesMut<'a, T, C> {
    type Item = &'a mut T;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.slots.next().map(|slot| &mut slot.value)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.slots.size_hint()
    }
}

impl<T, C: MapConfig> ExactSizeIterator for SparseSecondaryValuesMut<'_, T, C> {}
impl<T, C: MapConfig> FusedIterator for SparseSecondaryValuesMut<'_, T, C> {}

/// This iterator takes each value out of a [`SparseSecondaryMap`] together
/// with its key, in arbitrary order. It is created using
/// [`SparseSecondaryMap::drain`].
pub struct SparseSecondaryDrain<'a, T, C: MapConfig> {
    slots: hash_map::Drain<'a, MapIdx<C>, SparseSlot<MapGen<C>, T>>,
}

impl<T, C: MapConfig> Iterator for SparseSecondaryDrain<'_, T, C> {
    type Item = (Key<MapKeyConfig<C>>, T);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let (idx, slot) = self.slots.next()?;
        // SAFETY: `idx` and the slot's generation are the index and the
        // generation of the key that the slot's value is stored under, so they
        // fit the key config.
        let key = unsafe { key_from_parts_unchecked::<C>(idx, slot.generation) };
        Some((key, slot.value))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.slots.size_hint()
    }
}

impl<T, C: MapConfig> ExactSizeIterator for SparseSecondaryDrain<'_, T, C> {}
impl<T, C: MapConfig> FusedIterator for SparseSecondaryDrain<'_, T, C> {}

/// This iterator yields each key of a [`SparseSecondaryMap`] together with
/// its value, in arbitrary order. It is created by consuming the map with
/// `into_iter`.
pub struct SparseSecondaryIntoIter<T, C: MapConfig> {
    slots: hash_map::IntoIter<MapIdx<C>, SparseSlot<MapGen<C>, T>>,
}

impl<T, C: MapConfig> Iterator for SparseSecondaryIntoIter<T, C> {
    type Item = (Key<MapKeyConfig<C>>, T);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let (idx, slot) = self.slots.next()?;
        // SAFETY: `idx` and the slot's generation are the index and the
        // generation of the key that the slot's value is stored under, so they
        // fit the key config.
        let key = unsafe { key_from_parts_unchecked::<C>(idx, slot.generation) };
        Some((key, slot.value))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.slots.size_hint()
    }
}

impl<T, C: MapConfig> ExactSizeIterator for SparseSecondaryIntoIter<T, C> {}
impl<T, C: MapConfig> FusedIterator for SparseSecondaryIntoIter<T, C> {}

impl<G: KeyPiece, T> SparseSlot<G, T> {
    /// Creates a slot that holds `value` under `generation`.
    ///
    /// # Examples
    ///
    /// ```
    /// use gen_map::{GenMap, SparseSlot};
    ///
    /// let mut map = GenMap::new();
    /// let key = map.insert("a");
    ///
    /// let mut slot = SparseSlot::<u32, u64>::new(key.generation(), 10);
    /// *slot.value_mut() += 1;
    /// assert_eq!(slot.generation(), key.generation());
    /// assert_eq!(slot.value(), &11);
    /// assert_eq!(slot.into_inner(), (key.generation(), 11));
    /// ```
    #[inline]
    pub fn new(generation: Odd<G>, value: T) -> Self {
        Self { generation, value }
    }

    /// Returns the slot's generation.
    #[inline]
    pub fn generation(&self) -> Odd<G> {
        self.generation
    }

    /// Returns a reference to the value.
    #[inline]
    pub fn value(&self) -> &T {
        &self.value
    }

    /// Returns a mutable reference to the value.
    #[inline]
    pub fn value_mut(&mut self) -> &mut T {
        &mut self.value
    }

    /// Takes the generation and the value out of the slot.
    #[inline]
    pub fn into_inner(self) -> (Odd<G>, T) {
        (self.generation, self.value)
    }
}

impl<G: KeyPiece, T: fmt::Debug> fmt::Debug for SparseSlot<G, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SparseSlot")
            .field("generation", &self.generation.get())
            .field("value", &self.value)
            .finish()
    }
}

/// A [`SparseSecondaryMap`] keeps each of its values in a [`SparseSlot`] of
/// this type. The map's `HashMap` holds each slot under the index of its
/// value's key, and the slot stores the value together with the generation of
/// that key. The map removes a slot when it removes the slot's value, so every
/// slot holds a value. A key's generation is always odd, so the slot's
/// generation is too.
pub type SparseSecondaryMapSlot<T, C> = SparseSlot<MapGen<C>, T>;

/// `SparseSecondaryMapRawParts` holds the fields of a [`SparseSecondaryMap`].
/// [`into_raw_parts`](SparseSecondaryMap::into_raw_parts) takes a map apart
/// into these fields, and
/// [`from_raw_parts`](SparseSecondaryMap::from_raw_parts) builds a map from
/// them.
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
/// - No index in `slots` is above the largest index the map's keys can hold,
///   or is the largest value of the index type.
/// - No slot has a generation above the largest one the map's keys can hold.
///
/// # Examples
///
/// ```
/// use gen_map::{GenMap, SparseSecondaryMap, SparseSecondaryMapRawParts, SparseSlot};
/// use std::collections::HashMap;
///
/// let mut people = GenMap::new();
/// let alice = people.insert("Alice");
///
/// let mut slots = HashMap::new();
/// slots.insert(alice.idx(), SparseSlot::new(alice.generation(), 30));
/// // SAFETY: the index and the generation come from a key that a `GenMap`
/// // handed out, so they fit the key config, and the index is not the
/// // largest value of the index type.
/// let ages: SparseSecondaryMap<u32> =
///     unsafe { SparseSecondaryMap::from_raw_parts(SparseSecondaryMapRawParts { slots }) };
/// assert_eq!(ages[alice], 30);
/// ```
pub struct SparseSecondaryMapRawParts<
    T,
    C: SparseSecondaryMapConfig = DefaultMapConfig,
    S = RandomState,
> {
    // These fields are in the same order as the fields of `SparseSecondaryMap`
    // on purpose, so that the two structs can be read side by side.
    // `into_raw_parts` and `from_raw_parts` list every field of both structs,
    // so the compiler catches a field that only one of them has, but nothing
    // catches a change in order. Think twice before removing this comment,
    // because it is the only thing that keeps the two orders the same.
    /// The map keeps its values in this `HashMap`. It holds each value in a
    /// slot under the index of the value's key, and the slot holds the
    /// generation of that key.
    pub slots: HashMap<MapIdx<C>, SparseSecondaryMapSlot<T, C>, S>,
}

impl<T, C: SparseSecondaryMapConfig, S> SparseSecondaryMap<T, C, S> {
    /// Takes the map apart into its fields.
    /// [`from_raw_parts`](Self::from_raw_parts) builds a map from them again,
    /// and [`SparseSecondaryMapRawParts`] lists the rules they follow.
    #[inline]
    pub fn into_raw_parts(self) -> SparseSecondaryMapRawParts<T, C, S> {
        let Self { slots } = self;
        SparseSecondaryMapRawParts { slots }
    }

    /// Builds a map from the fields that
    /// [`into_raw_parts`](Self::into_raw_parts) takes a map apart into.
    ///
    /// # Safety
    ///
    /// `parts` must follow every rule listed on [`SparseSecondaryMapRawParts`].
    /// The map's methods rely on those rules, and parts that break one can
    /// make them cause undefined behavior.
    #[inline]
    #[must_use]
    pub unsafe fn from_raw_parts(parts: SparseSecondaryMapRawParts<T, C, S>) -> Self {
        let SparseSecondaryMapRawParts { slots } = parts;
        Self { slots }
    }
}
