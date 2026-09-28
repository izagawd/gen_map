#[cfg(feature = "alloc")]
use crate::config::DefaultMapConfig;
use crate::config::{GenMapConfig, KeyConfig, MapConfig, MapConfigFor};
use crate::error::{
    FullError, GetDisjointMutAtError, GetDisjointMutError, InsertError, InsertWithError,
};
use crate::key::Key;
use crate::key_piece::KeyPiece;
use crate::parity::{Even, Odd};
use crate::slot::{Parity, Slot};
use crate::storage::{ReserveStorage, SlotStorage};
use core::fmt;
use core::iter::{Enumerate, FusedIterator};
use core::ops::{Index, IndexMut};

/// The key config that a map config `C` picks for its keys.
pub type MapKeyConfig<C> = <C as MapConfig>::KeyConfig;

/// The index type of the keys that a map with config `C` works with.
pub type MapIdx<C> = <MapKeyConfig<C> as KeyConfig>::Idx;

/// The generation type of the keys that a map with config `C` works with.
pub type MapGen<C> = <MapKeyConfig<C> as KeyConfig>::Gen;

/// Shorter aliases for [`MapIdx`] and [`MapGen`] inside the crate.
pub(crate) type Idx<C> = MapIdx<C>;
pub(crate) type Gen<C> = MapGen<C>;

/// The [`Slot`] a [`GenMap<T, C>`](GenMap) keeps each value in. While a
/// slot is on the free list, its `U` is the index of the next free slot, or
/// the largest value of the index type if it is the last free slot. No slot
/// ever has that index, so it can't be mistaken for the index of a real
/// slot. A retired slot's `U` is also the largest value of the index type,
/// and a detached slot's `U` is its own index.
pub type MapSlot<T, C> = Slot<MapGen<C>, T, MapIdx<C>>;

/// The key of the value in the slot at `idx` whose generation is
/// `generation`.
///
/// # Safety
///
/// `idx` must be the position of a slot in the storage, and `generation`
/// must be the generation of that slot while it holds a value.
#[inline]
unsafe fn slot_key<C: MapConfig>(idx: Idx<C>, generation: Odd<Gen<C>>) -> Key<MapKeyConfig<C>> {
    // SAFETY: the map never lets a slot's position, or the generation of a
    // slot that holds a value, grow past what the key config holds.
    Key::from_repr(unsafe { <MapKeyConfig<C> as KeyConfig>::pack_unchecked(idx, generation) })
}

/// Converts a position in the backing storage to `Idx<C>`.
///
/// # Safety
///
/// `position` must be the position of a slot in the map's storage. Every such
/// position fits, because the map refuses to push a slot whose position does
/// not.
#[inline]
unsafe fn index_of<C: MapConfig>(position: usize) -> Idx<C> {
    debug_assert!(
        Idx::<C>::from_usize(position).is_some(),
        "every slot position fits in the configured index type"
    );
    // SAFETY: the caller promises that `position` belongs to a slot, and
    // every slot's position fits in `Idx<C>`.
    unsafe { Idx::<C>::from_usize_unchecked(position) }
}

/// Converts the index of a slot back to its position in the backing storage.
///
/// # Safety
///
/// `idx` must be the index of a slot, such as one on the free list or one
/// carried by a key that has a value. Every such index was made from a slot's
/// position, so it fits in `usize`.
#[inline]
unsafe fn position_of<C: MapConfig>(idx: Idx<C>) -> usize {
    debug_assert!(
        idx.into_usize().is_some(),
        "every index of a slot fits in usize"
    );
    // SAFETY: the caller promises that `idx` is a slot's index, and every
    // slot's index was made from its position, which is a `usize`.
    unsafe { idx.into_usize_unchecked() }
}

/// The storage a config gives the map for its slots.
type Slots<T, C> = <C as GenMapConfig<MapSlot<T, C>>>::Storage;

/// The largest index a key of `C` can hold.
#[inline]
fn max_idx<C: MapConfig>() -> Idx<C> {
    <MapKeyConfig<C> as KeyConfig>::max_idx()
}

/// The largest index a slot can have. It is the largest index a key of `C`
/// can hold, except that no slot ever gets the largest value of the index
/// type. That value is [`no_slot`], and leaving it out also keeps the number
/// of slots, and so the number of values, within the index type.
#[inline]
fn max_slot_idx<C: MapConfig>() -> Idx<C> {
    let max = max_idx::<C>();
    if max == Idx::<C>::MAX {
        max.wrapping_sub(Idx::<C>::ONE)
    } else {
        max
    }
}

/// The largest value of the index type, which no slot ever has as its index.
/// The free list ends with it, `next_free` holds it while no slot is free,
/// and a retired slot keeps it as its link.
#[inline]
fn no_slot<C: MapConfig>() -> Idx<C> {
    Idx::<C>::MAX
}

/// Adds one to a map's `len`, which is below the largest value of the index
/// type beforehand, because every value has a slot of its own and no slot has
/// that index.
#[inline]
pub(crate) fn increment_len<I: KeyPiece>(len: &mut I) {
    debug_assert!(
        *len < I::MAX,
        "a map never holds as many values as the largest value of its index type"
    );
    *len = len.wrapping_add(I::ONE);
}

/// Takes one off a map's `len`. The map only calls this after taking out a
/// value it held, so `len` is above zero beforehand.
#[inline]
pub(crate) fn decrement_len<I: KeyPiece>(len: &mut I) {
    debug_assert!(*len > I::ZERO, "a map only removes a value it holds");
    *len = len.wrapping_sub(I::ONE);
}

/// The generation after `generation`, or `None` if `generation` is the
/// largest one a key of `C` can hold, which is when a slot retires or wraps.
#[inline]
fn next_generation<C: MapConfig>(generation: Odd<Gen<C>>) -> Option<Even<Gen<C>>> {
    if generation == <MapKeyConfig<C> as KeyConfig>::max_generation() {
        None
    } else {
        Some(generation.wrapping_next())
    }
}

/// The generation a slot has while it is detached, for a key whose generation
/// is `generation`. It is one more than `generation`, or zero if `generation`
/// is the largest value of the generation type. It is even, so no key matches
/// the slot. [`GenMap::detach`] gives the slot this generation, and
/// [`GenMap::reattach`] calls this function with the key's generation to
/// check that the slot is still detached under that key.
///
/// With a [`Packed`](crate::Packed) key config, this generation can be one
/// past the largest generation a key can hold. That does no harm, because a
/// detached slot is never on the free list, so its generation never goes into
/// a key.
#[inline]
fn detached_generation<C: MapConfig>(generation: Odd<Gen<C>>) -> Even<Gen<C>> {
    generation.wrapping_next()
}

/// The error the storage of a `GenMap<T, C>` gives when it cannot make room
/// for another slot. It is `TryReserveError` for a `Vec`, `CapacityError` for
/// an `ArrayVec` and `CollectionAllocErr` for a `SmallVec`.
pub type StorageError<T, C> = <<C as GenMapConfig<MapSlot<T, C>>>::Storage as SlotStorage>::Error;

/// Where the next inserted value will go, worked out before anything is
/// written.
struct Target<C: MapConfig> {
    idx: Idx<C>,
    position: usize,
    /// The generation the new key gets.
    generation: Odd<Gen<C>>,
    /// `true` if the slot came off the free list, and `false` if it has to be
    /// pushed onto the storage.
    from_free_list: bool,
}

impl<C: MapConfig> Target<C> {
    /// The key that a value gets when it is put at this target.
    #[inline]
    fn key(&self) -> Key<MapKeyConfig<C>> {
        // SAFETY: a `Target` only ever comes from `next_target`, which checks
        // that both parts fit the key config.
        Key::from_repr(unsafe {
            <MapKeyConfig<C> as KeyConfig>::pack_unchecked(self.idx, self.generation)
        })
    }
}

/// Panics with the message `insert` gives for a full map. It is kept out of
/// line so that the insert paths stay small.
#[cold]
#[inline(never)]
fn panic_full<T, C: MapConfigFor<T>>(full: FullError<StorageError<T, C>>, slots_len: usize) -> ! {
    match full {
        FullError::IndexExhausted => panic!(
            "GenMap is full, its keys cannot address more than {} slots",
            slots_len
        ),
        FullError::StorageFull(error) => panic!(
            "GenMap is full, its storage cannot make room for more than {} slots: {:?}",
            slots_len, error
        ),
    }
}

/// The slot the next insert would use, handed out by
/// [`GenMap::vacant_entry`](GenMap::vacant_entry). No slot is written until
/// [`insert`](Self::insert) is called, so dropping the entry inserts
/// nothing.
pub struct VacantEntry<'a, T, C: MapConfigFor<T>> {
    map: &'a mut GenMap<T, C>,
    target: Target<C>,
}

impl<'a, T, C: MapConfigFor<T>> VacantEntry<'a, T, C> {
    /// The key the value will get. It matches nothing until
    /// [`insert`](Self::insert) is called.
    #[inline]
    pub fn key(&self) -> Key<MapKeyConfig<C>> {
        self.target.key()
    }

    /// Puts `value` in the slot and returns its key, the same one
    /// [`key`](Self::key) returns.
    #[inline]
    pub fn insert(self, value: T) -> Key<MapKeyConfig<C>> {
        // SAFETY: `target` came from `next_target`, and this entry has held
        // `&mut` on the map since, so nothing has touched it.
        unsafe { self.map.fill(self.target, value) }
    }
}

impl<T, C: MapConfigFor<T>> fmt::Debug for VacantEntry<'_, T, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VacantEntry")
            .field("key", &self.key())
            .finish()
    }
}

/// A generational map that holds values of type `T` and is configured by
/// `C`.
///
/// With the `alloc` feature, `C` defaults to [`DefaultMapConfig`]. To use
/// your own config, implement [`MapConfig`] and [`GenMapConfig`] for it,
/// usually as `impl MapConfig for YourConfig` and
/// `impl<S: GenSlotItem> GenMapConfig<S> for YourConfig`. The bound on `C`,
/// [`MapConfigFor<T>`](MapConfigFor), is implemented automatically for every
/// such config, so you never implement it yourself.
///
/// See the [crate documentation](crate) for examples.
pub struct GenMap<
    T,
    #[cfg(feature = "alloc")] C: MapConfigFor<T> = DefaultMapConfig,
    #[cfg(not(feature = "alloc"))] C: MapConfigFor<T>,
> {
    slots: Slots<T, C>,
    /// The index of the first slot on the free list, or `no_slot` if no slot
    /// is free.
    next_free: Idx<C>,
    /// The number of values. No slot has the largest value of the index
    /// type as its index, so there are at most that many slots, and the
    /// count always fits.
    len: Idx<C>,
}

#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
impl<T> GenMap<T> {
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

    /// Creates an empty map with the [`DefaultMapConfig`] and room for
    /// `capacity` slots.
    #[inline]
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self
    where
        Slots<T, DefaultMapConfig>: ReserveStorage,
    {
        Self::with_slots(Slots::<T, DefaultMapConfig>::with_capacity(capacity))
    }
}

/// These methods need a storage that can grow on request, so a map whose
/// storage has a fixed capacity does not have them.
impl<T, C: MapConfigFor<T>> GenMap<T, C>
where
    Slots<T, C>: ReserveStorage,
{
    /// Creates an empty map with config `C` and room for `capacity` slots.
    #[inline]
    #[must_use]
    pub fn with_capacity_and_config(capacity: usize) -> Self {
        Self::with_slots(Slots::<T, C>::with_capacity(capacity))
    }

    /// Reserves room for at least `additional` more slots.
    ///
    /// # Panics
    ///
    /// Panics or aborts if the storage cannot make the room, as
    /// `Vec::reserve` does. Use [`try_reserve`](Self::try_reserve) to get an
    /// error instead.
    #[inline]
    pub fn reserve(&mut self, additional: usize) {
        self.slots.reserve(additional);
    }

    /// The fallible form of [`reserve`](Self::reserve). After `Ok`, the next
    /// `additional` inserts cannot fail for lack of storage.
    ///
    /// # Errors
    ///
    /// Returns what the storage says when it cannot make the room.
    #[inline]
    pub fn try_reserve(&mut self, additional: usize) -> Result<(), StorageError<T, C>> {
        self.slots.try_reserve(additional)
    }
}

impl<T, C: MapConfigFor<T>> GenMap<T, C> {
    /// Creates an empty map with config `C`.
    ///
    /// ```
    /// use gen_map::{GenMap, GenMapConfig, GenSlotItem, MapConfig, Split};
    ///
    /// struct Wide;
    ///
    /// impl MapConfig for Wide {
    ///     type KeyConfig = Split<u64, u64>;
    /// }
    ///
    /// impl<S: GenSlotItem> GenMapConfig<S> for Wide {
    ///     type Storage = Vec<S>;
    /// }
    ///
    /// let mut map = GenMap::<&str, Wide>::new_with_config();
    /// let key = map.insert("x");
    /// assert_eq!(core::mem::size_of_val(&key), 16);
    /// ```
    #[inline]
    #[must_use]
    pub fn new_with_config() -> Self {
        Self::with_slots(Slots::<T, C>::empty())
    }

    /// Creates a map with no values that keeps its slots in `slots`. The
    /// storage must be empty.
    #[inline]
    fn with_slots(slots: Slots<T, C>) -> Self {
        debug_assert!(slots.as_slice().is_empty());
        Self {
            slots,
            next_free: no_slot::<C>(),
            len: Idx::<C>::ZERO,
        }
    }

    /// How many slots the storage can hold before it has to grow, or in total
    /// if it cannot grow.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.slots.capacity()
    }

    /// The number of values in the map.
    #[inline]
    pub fn len(&self) -> usize {
        // SAFETY: every value has a slot of its own in the storage, so the
        // count is at most the number of slots, which is a `usize`.
        unsafe { self.len.into_usize_unchecked() }
    }

    /// Returns `true` if the map holds no values.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == Idx::<C>::ZERO
    }

    /// The number of slots, whether they hold a value or not.
    #[inline]
    pub fn slots_len(&self) -> usize {
        self.slots.len()
    }

    /// Returns `true` if the map has a value for `key`.
    #[inline]
    pub fn contains_key(&self, key: Key<MapKeyConfig<C>>) -> bool {
        self.get(key).is_some()
    }

    /// Returns a reference to the value corresponding to `key`.
    #[inline]
    pub fn get(&self, key: Key<MapKeyConfig<C>>) -> Option<&T> {
        self.slots
            .as_slice()
            .get(key.idx().into_usize()?)?
            .get_odd(key.generation())
    }

    /// Returns a mutable reference to the value corresponding to `key`.
    #[inline]
    pub fn get_mut(&mut self, key: Key<MapKeyConfig<C>>) -> Option<&mut T> {
        self.slots
            .as_mut_slice()
            .get_mut(key.idx().into_usize()?)?
            .get_odd_mut(key.generation())
    }

    /// [`get`](Self::get) without the bounds and generation checks.
    ///
    /// # Safety
    ///
    /// The map must have a value for `key`, meaning
    /// [`contains_key`](Self::contains_key) returns `true` for it.
    #[inline]
    pub unsafe fn get_unchecked(&self, key: Key<MapKeyConfig<C>>) -> &T {
        debug_assert!(self.contains_key(key));
        // SAFETY: the caller promises the map has a value for `key`, so its
        // index is the position of a slot in bounds, and that slot's
        // generation is odd.
        unsafe {
            self.slots
                .as_slice()
                .get_unchecked(position_of::<C>(key.idx()))
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
    pub unsafe fn get_unchecked_mut(&mut self, key: Key<MapKeyConfig<C>>) -> &mut T {
        debug_assert!(self.contains_key(key));
        // SAFETY: the same as in `get_unchecked`.
        unsafe {
            self.slots
                .as_mut_slice()
                .get_unchecked_mut(position_of::<C>(key.idx()))
                .get_odd_unchecked_mut()
        }
    }

    /// Returns the key of the value in the slot at `idx`, or `None` if there
    /// is no slot at `idx` or the slot holds no value. A slot holds no value
    /// while it is vacant, detached or retired.
    #[inline]
    pub fn key_at(&self, idx: MapIdx<C>) -> Option<Key<MapKeyConfig<C>>> {
        match self.slots.as_slice().get(idx.into_usize()?)?.as_parity() {
            // SAFETY: `idx` is the slot's position.
            Parity::Odd(generation, _) => Some(unsafe { slot_key::<C>(idx, generation) }),
            Parity::Even(..) => None,
        }
    }

    /// [`key_at`](Self::key_at) without the bounds and occupancy checks.
    ///
    /// # Safety
    ///
    /// There must be a slot at `idx` and it must hold a value, meaning
    /// [`key_at`](Self::key_at) returns `Some` for it. A slot that holds no
    /// value has an even generation, and a key's generation is an
    /// [`Odd`](crate::Odd), so building a key from that generation is
    /// undefined behavior.
    #[inline]
    pub unsafe fn key_at_unchecked(&self, idx: MapIdx<C>) -> Key<MapKeyConfig<C>> {
        debug_assert!(self.key_at(idx).is_some());
        // SAFETY: the caller promises a slot at `idx` that holds a value, so
        // `idx` is the position of a slot in bounds, and that slot's
        // generation is odd.
        unsafe {
            let slot = self.slots.as_slice().get_unchecked(position_of::<C>(idx));
            slot_key::<C>(idx, Odd::new_unchecked(slot.generation()))
        }
    }

    /// Returns the key and a reference to the value in the slot at `idx`, or
    /// `None` if there is no slot at `idx` or the slot holds no value. A slot
    /// holds no value while it is vacant, detached or retired.
    #[inline]
    pub fn get_at(&self, idx: MapIdx<C>) -> Option<(Key<MapKeyConfig<C>>, &T)> {
        match self.slots.as_slice().get(idx.into_usize()?)?.as_parity() {
            Parity::Odd(generation, value) => {
                // SAFETY: `idx` is the slot's position.
                Some((unsafe { slot_key::<C>(idx, generation) }, value))
            }
            Parity::Even(..) => None,
        }
    }

    /// Returns the key and a mutable reference to the value in the slot at
    /// `idx`, or `None` if there is no slot at `idx` or the slot holds no
    /// value. See [`get_at`](Self::get_at).
    #[inline]
    pub fn get_at_mut(&mut self, idx: MapIdx<C>) -> Option<(Key<MapKeyConfig<C>>, &mut T)> {
        match self
            .slots
            .as_mut_slice()
            .get_mut(idx.into_usize()?)?
            .as_parity_mut()
        {
            Parity::Odd(generation, value) => {
                // SAFETY: `idx` is the slot's position.
                Some((unsafe { slot_key::<C>(idx, generation) }, value))
            }
            Parity::Even(..) => None,
        }
    }

    /// [`get_at`](Self::get_at) without the bounds and occupancy checks.
    ///
    /// # Safety
    ///
    /// There must be a slot at `idx` and it must hold a value, meaning
    /// [`key_at`](Self::key_at) returns `Some` for it. Otherwise the slot is
    /// read as if it held a value and the key gets built using an even
    /// generation, both of which are undefined behavior.
    #[inline]
    pub unsafe fn get_at_unchecked(&self, idx: MapIdx<C>) -> (Key<MapKeyConfig<C>>, &T) {
        debug_assert!(self.key_at(idx).is_some());
        // SAFETY: the same as in `key_at_unchecked`.
        unsafe {
            let slot = self.slots.as_slice().get_unchecked(position_of::<C>(idx));
            (
                slot_key::<C>(idx, Odd::new_unchecked(slot.generation())),
                slot.get_odd_unchecked(),
            )
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
        idx: MapIdx<C>,
    ) -> (Key<MapKeyConfig<C>>, &mut T) {
        debug_assert!(self.key_at(idx).is_some());
        // SAFETY: the same as in `key_at_unchecked`.
        unsafe {
            let slot = self
                .slots
                .as_mut_slice()
                .get_unchecked_mut(position_of::<C>(idx));
            let key = slot_key::<C>(idx, Odd::new_unchecked(slot.generation()));
            (key, slot.get_odd_unchecked_mut())
        }
    }

    /// Returns the generation of the slot at `idx`, or `None` if there is no
    /// slot at `idx`.
    ///
    /// Every slot has a generation, whether it holds a value or not, so this
    /// works for vacant, detached and retired slots too. The generation is
    /// odd while the slot holds a value and even otherwise.
    #[inline]
    pub fn generation_at(&self, idx: MapIdx<C>) -> Option<MapGen<C>> {
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
    pub unsafe fn generation_at_unchecked(&self, idx: MapIdx<C>) -> MapGen<C> {
        debug_assert!(self.generation_at(idx).is_some());
        // SAFETY: the caller promises a slot at `idx`, so `idx` is the
        // position of a slot in bounds.
        unsafe {
            self.slots
                .as_slice()
                .get_unchecked(position_of::<C>(idx))
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
    /// use gen_map::{GenMap, GetDisjointMutAtError};
    ///
    /// let mut map = GenMap::new();
    /// let a = map.insert(1);
    /// let b = map.insert(2);
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
        // SAFETY: every index was just found to point at an occupied slot,
        // and every index is distinct.
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
            // value, so its position is in bounds and its generation is odd.
            // The caller also promises that no other index points at the
            // same slot, so the references do not alias.
            unsafe {
                let slot = &mut *slots.add(position_of::<C>(idx));
                let key = slot_key::<C>(idx, Odd::new_unchecked(slot.generation()));
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
    /// use gen_map::{GenMap, GetDisjointMutError};
    ///
    /// let mut map = GenMap::new();
    /// let a = map.insert(1);
    /// let b = map.insert(2);
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

    /// [`get_disjoint_mut`](Self::get_disjoint_mut) without any of the
    /// checks.
    ///
    /// # Safety
    ///
    /// The map must have a value for every key, meaning
    /// [`contains_key`](Self::contains_key) returns `true` for each of them,
    /// and no two keys may point at the same slot.
    /// Otherwise a slot is read as if it held a value, or two of the
    /// references point at the same value, either of which is undefined
    /// behavior.
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
            // position is in bounds and its slot's generation is odd. The
            // caller also promises that no other key refers to the same slot,
            // so the references do not alias.
            unsafe { (*slots.add(position_of::<C>(key.idx()))).get_odd_unchecked_mut() }
        })
    }

    /// Inserts a value and returns its key.
    ///
    /// # Panics
    ///
    /// Panics if the map is full, meaning none of the slots are free and
    /// either its keys have no index left for a new slot or the storage cannot
    /// make room for one. Use [`try_insert`](Self::try_insert) to get
    /// the value back instead.
    #[inline]
    pub fn insert(&mut self, value: T) -> Key<MapKeyConfig<C>> {
        self.insert_with_key(|_| value)
    }

    /// Inserts a value and returns its key, or hands the value back if the
    /// map is full.
    ///
    /// # Errors
    ///
    /// Returns [`InsertError::IndexExhausted`] if none of the slots are free
    /// and the map's keys have no index left for a new one, and
    /// [`InsertError::StorageFull`] if none of them are free and the storage
    /// cannot make room for another one.
    ///
    /// # Examples
    ///
    /// ```
    /// use gen_map::{GenMap, GenMapConfig, GenSlotItem, InsertError, MapConfig, Split};
    ///
    /// struct Tiny;
    ///
    /// impl MapConfig for Tiny {
    ///     type KeyConfig = Split<u8, u8>;
    /// }
    ///
    /// impl<S: GenSlotItem> GenMapConfig<S> for Tiny {
    ///     type Storage = Vec<S>;
    /// }
    ///
    /// // No slot gets the index `u8::MAX`, so the map holds 255 values.
    /// let mut map = GenMap::<u32, Tiny>::new_with_config();
    /// for i in 0..255 {
    ///     map.insert(i);
    /// }
    /// assert!(matches!(map.try_insert(255), Err(InsertError::IndexExhausted(255))));
    /// ```
    #[inline]
    #[allow(clippy::type_complexity)]
    pub fn try_insert(
        &mut self,
        value: T,
    ) -> Result<Key<MapKeyConfig<C>>, InsertError<T, StorageError<T, C>>> {
        match self.vacant_entry() {
            Ok(entry) => Ok(entry.insert(value)),
            Err(FullError::IndexExhausted) => Err(InsertError::IndexExhausted(value)),
            Err(FullError::StorageFull(error)) => Err(InsertError::StorageFull(value, error)),
        }
    }

    /// Calls `f` with the key that the new value will get, and inserts the
    /// value that `f` returns. If `f` panics, nothing is inserted.
    ///
    /// # Panics
    ///
    /// Panics if the map is full, meaning none of the slots are free and
    /// either its keys have no index left for a new slot or the storage
    /// cannot make room for one. Use
    /// [`try_insert_with_key`](Self::try_insert_with_key) or
    /// [`vacant_entry`](Self::vacant_entry) to get an error instead.
    #[inline]
    pub fn insert_with_key<F>(&mut self, f: F) -> Key<MapKeyConfig<C>>
    where
        F: FnOnce(Key<MapKeyConfig<C>>) -> T,
    {
        // Nothing is written until `f` has returned, so a panicking `f`
        // leaves the map untouched.
        let entry = match self.vacant_entry() {
            Ok(entry) => entry,
            Err(full) => panic_full::<T, C>(full, self.slots.len()),
        };
        let value = f(entry.key());
        entry.insert(value)
    }

    /// Like [`insert_with_key`](Self::insert_with_key), but `f` may fail and
    /// a full map is an error rather than a panic. On either error, nothing is
    /// inserted. The key that `f` was given does not match anything, but the
    /// next insert can hand out that same key for a different value.
    ///
    /// # Errors
    ///
    /// Returns [`InsertWithError::Full`] if the map is full. `f` is not
    /// called in that case. Returns [`InsertWithError::Rejected`] with
    /// whatever `f` returned if `f` fails.
    ///
    /// # Examples
    ///
    /// ```
    /// use gen_map::{GenMap, InsertWithError};
    ///
    /// let mut map = GenMap::new();
    /// let key = map.try_insert_with_key(|key| Ok::<_, ()>(key)).unwrap();
    /// assert_eq!(map[key], key);
    ///
    /// let result = map.try_insert_with_key(|_| Err::<_, &str>("no thanks"));
    /// assert_eq!(result, Err(InsertWithError::Rejected("no thanks")));
    /// assert_eq!(map.len(), 1);
    /// ```
    #[allow(clippy::type_complexity)]
    pub fn try_insert_with_key<F, E>(
        &mut self,
        f: F,
    ) -> Result<Key<MapKeyConfig<C>>, InsertWithError<E, StorageError<T, C>>>
    where
        F: FnOnce(Key<MapKeyConfig<C>>) -> Result<T, E>,
    {
        let entry = self.vacant_entry()?;
        let value = f(entry.key()).map_err(InsertWithError::Rejected)?;
        Ok(entry.insert(value))
    }

    /// Hands out the slot the next insert would use, without writing to it.
    /// [`VacantEntry::key`] is the key the value will get and
    /// [`VacantEntry::insert`] puts it there. Dropping the entry inserts
    /// nothing.
    ///
    /// # Errors
    ///
    /// Returns [`FullError::IndexExhausted`] if none of the slots are free
    /// and the map's keys have no index left for a new one, and
    /// [`FullError::StorageFull`] if none of them are free and the storage
    /// cannot make room for another one. When a `Vec` cannot allocate,
    /// this returns `StorageFull` instead of panicking.
    #[inline]
    pub fn vacant_entry(&mut self) -> Result<VacantEntry<'_, T, C>, FullError<StorageError<T, C>>> {
        let target = self.next_target()?;
        Ok(VacantEntry { map: self, target })
    }

    /// Works out where the next value goes without writing any slot. When a
    /// slot has to be pushed, this makes sure there is room for it, so that
    /// the push in [`fill`](Self::fill) cannot fail.
    ///
    /// When the keys have no index left for a new slot and the storage is
    /// also full, the error is `IndexExhausted`.
    #[inline]
    fn next_target(&mut self) -> Result<Target<C>, FullError<StorageError<T, C>>> {
        let (idx, position, generation, from_free_list) = if self.next_free != no_slot::<C>() {
            let idx = self.next_free;
            // SAFETY: `idx` is on the free list, so it was the position of a
            // slot that still exists, and that slot holds no value, so its
            // generation is even.
            let (position, generation) = unsafe {
                let position = position_of::<C>(idx);
                let slot = self.slots.as_slice().get_unchecked(position);
                (position, Even::new_unchecked(slot.generation()))
            };
            (idx, position, generation, true)
        } else {
            let position = self.slots.len();
            let idx = Idx::<C>::from_usize(position)
                .filter(|idx| *idx <= max_slot_idx::<C>())
                .ok_or(FullError::IndexExhausted)?;
            self.slots.ensure_room(1).map_err(FullError::StorageFull)?;
            (idx, position, Even::ZERO, false)
        };

        // The largest generation a key can hold is odd, so it is above every
        // even one, and the generation after an even one still fits.
        let generation = generation.next();

        Ok(Target {
            idx,
            position,
            generation,
            from_free_list,
        })
    }

    /// Writes `value` where `target` says and returns its key.
    ///
    /// # Safety
    ///
    /// `target` must have come from [`next_target`](Self::next_target) with
    /// nothing having touched the map since.
    #[inline]
    unsafe fn fill(&mut self, target: Target<C>, value: T) -> Key<MapKeyConfig<C>> {
        let key = target.key();
        if target.from_free_list {
            // SAFETY: `position` was in bounds when `next_target` looked, and
            // nothing has changed since.
            let slot = unsafe { self.slots.as_mut_slice().get_unchecked_mut(target.position) };
            // SAFETY: the slot came off the free list, so it holds no value
            // and its generation is even.
            self.next_free = unsafe { slot.replace_even_unchecked(target.generation, value) };
        } else {
            let slot = Slot::new_odd(target.generation, value);
            // `next_target` made room for this slot, and `SlotStorage`
            // promises that `try_push` succeeds after that, so only a broken
            // storage refuses the push. The slot is dropped with its value in
            // that case.
            if self.slots.try_push(slot).is_err() {
                panic!("SlotStorage::try_push failed although ensure_room returned Ok");
            }
        }
        increment_len(&mut self.len);
        key
    }

    /// Removes and returns the value corresponding to `key`, or `None` if
    /// there is none.
    #[inline]
    pub fn remove(&mut self, key: Key<MapKeyConfig<C>>) -> Option<T> {
        let position = key.idx().into_usize()?;
        self.slots
            .as_slice()
            .get(position)?
            .get_odd(key.generation())?;
        // SAFETY: the slot's generation matches the key's, which is odd, so
        // the slot holds a value.
        Some(unsafe { self.take(key.idx(), position) })
    }

    /// Removes and returns the value corresponding to `key` like
    /// [`remove`](Self::remove), but retires its slot instead of freeing it.
    /// Returns `None` if there is no value for `key`.
    ///
    /// A retired slot is never used again until [`reset`](Self::reset), no
    /// matter how the map is configured, so no key to it can ever match a
    /// new value. This is how a map that wraps generations can still retire
    /// the slots it chooses.
    ///
    /// # Examples
    ///
    /// ```
    /// use gen_map::{GenMap, GenMapConfig, GenSlotItem, MapConfig, Packed};
    ///
    /// /// Sixteen slots whose generations wrap after eight values.
    /// struct Wrapping;
    ///
    /// impl MapConfig for Wrapping {
    ///     type KeyConfig = Packed<u8, 4>;
    /// }
    ///
    /// impl<S: GenSlotItem> GenMapConfig<S> for Wrapping {
    ///     const WRAP_ON_OVERFLOW: bool = true;
    ///     type Storage = Vec<S>;
    /// }
    ///
    /// let mut map = GenMap::<&str, Wrapping>::new_with_config();
    /// let key = map.insert("a");
    /// assert_eq!(map.retire(key), Some("a"));
    /// assert!(map.get(key).is_none());
    /// assert_eq!(map.generation_at(key.idx()), Some(0));
    ///
    /// // The retired slot is skipped, so the next value gets a new slot.
    /// let other = map.insert("b");
    /// assert_ne!(other.idx(), key.idx());
    /// ```
    #[inline]
    pub fn retire(&mut self, key: Key<MapKeyConfig<C>>) -> Option<T> {
        let slot = self.slots.as_mut_slice().get_mut(key.idx().into_usize()?)?;
        slot.get_odd(key.generation())?;
        decrement_len(&mut self.len);
        // A generation of zero and a link to no slot is what a retired slot
        // looks like, and nothing puts it back on the free list.
        // SAFETY: the slot's generation matched the key's, which is odd.
        Some(unsafe { slot.replace_odd_unchecked(Even::ZERO, no_slot::<C>()) })
    }

    /// Moves the value out of the slot at `position`. The slot then goes on
    /// the free list, unless its generation has run out and the config
    /// retires such slots.
    ///
    /// # Safety
    ///
    /// The slot at `position` must be occupied, and `idx` must be `position`
    /// as an `Idx<C>`.
    unsafe fn take(&mut self, idx: Idx<C>, position: usize) -> T {
        // SAFETY: the caller promises a slot at `position` that holds a
        // value, so the position is in bounds and the generation is odd.
        let (slot, generation) = unsafe {
            let slot = self.slots.as_mut_slice().get_unchecked_mut(position);
            let generation = Odd::new_unchecked(slot.generation());
            (slot, generation)
        };
        decrement_len(&mut self.len);

        // A slot whose generation has run out starts over at zero if the
        // config wraps, and retires otherwise. A retired slot stays off the
        // free list for good.
        let (next, link) = match next_generation::<C>(generation) {
            Some(next) => (next, core::mem::replace(&mut self.next_free, idx)),
            None if <C as GenMapConfig<MapSlot<T, C>>>::WRAP_ON_OVERFLOW => {
                (Even::ZERO, core::mem::replace(&mut self.next_free, idx))
            }
            None => (Even::ZERO, no_slot::<C>()),
        };
        // SAFETY: the slot's generation is odd.
        unsafe { slot.replace_odd_unchecked(next, link) }
    }

    /// Takes the value out of the map while keeping its slot reserved for
    /// `key`, so that [`reattach`](Self::reattach) can put a value back under
    /// the same key. Returns `None` if there is no value for `key`.
    ///
    /// Until `reattach` puts a value back,
    /// [`contains_key`](Self::contains_key) returns `false` for the key and
    /// [`len`](Self::len) does not count the value, but the slot is not on
    /// the free list, so no insert uses it.
    ///
    /// While the slot is detached, its generation is one more than the key's
    /// generation, or zero if the key's generation is the largest value of
    /// the generation type. No key matches that generation, and `reattach`
    /// gives the slot the key's generation back.
    ///
    /// [`clear`](Self::clear) and [`retain`](Self::retain) leave a detached
    /// slot as it is, since it holds no value. [`reset`](Self::reset) removes
    /// every slot, detached ones included, so `reattach` fails for a key that
    /// was detached before the reset.
    ///
    /// # Examples
    ///
    /// ```
    /// use gen_map::GenMap;
    ///
    /// let mut map = GenMap::new();
    /// let key = map.insert(1);
    ///
    /// let value = map.detach(key).unwrap();
    /// assert!(map.get(key).is_none());
    /// let other = map.insert(2); // Does not take the detached slot.
    /// assert_ne!(other.idx(), key.idx());
    ///
    /// map.reattach(key, value + 10).unwrap();
    /// assert_eq!(map[key], 11);
    /// ```
    #[inline]
    pub fn detach(&mut self, key: Key<MapKeyConfig<C>>) -> Option<T> {
        let slot = self.slots.as_mut_slice().get_mut(key.idx().into_usize()?)?;
        slot.get_odd(key.generation())?;
        decrement_len(&mut self.len);
        let detached = detached_generation::<C>(key.generation());
        // A detached slot has its own index in its `U`. A slot on the free
        // list has another slot's index or `no_slot` in its `U`, and a
        // retired slot has `no_slot`, so neither looks detached.
        // SAFETY: the slot's generation matched the key's, which is odd.
        Some(unsafe { slot.replace_odd_unchecked(detached, key.idx()) })
    }

    /// Puts a value back under a key whose value [`detach`](Self::detach)
    /// took out. Afterwards, the map has a value for the key again.
    ///
    /// # Errors
    ///
    /// Hands `value` back, and leaves the map as it was, if the key was not
    /// detached, or was detached and its slot has since been removed by
    /// [`reset`](Self::reset).
    #[inline]
    pub fn reattach(&mut self, key: Key<MapKeyConfig<C>>, value: T) -> Result<(), T> {
        let generation = key.generation();
        let Some(slot) = key
            .idx()
            .into_usize()
            .and_then(|position| self.slots.as_mut_slice().get_mut(position))
            .filter(|slot| {
                // When the key's value was detached, the slot's generation
                // was set to what `detached_generation` returns for the key's
                // generation, and the slot's `U` was set to the slot's own
                // index. A slot on the free list has another slot's index or
                // `no_slot` in its `U`, and a retired slot has `no_slot`. So
                // a slot is still detached under this key only if it has that
                // generation and its own index in its `U`.
                slot.get_even(detached_generation::<C>(generation))
                    .is_some_and(|link| *link == key.idx())
            })
        else {
            return Err(value);
        };
        // SAFETY: the slot's generation was just found to be even.
        unsafe { slot.replace_even_unchecked(generation, value) };
        increment_len(&mut self.len);
        Ok(())
    }

    /// Removes every value. The slots stay, and old keys stop matching just
    /// as they do after [`remove`](Self::remove).
    pub fn clear(&mut self) {
        for position in 0..self.slots.len() {
            // SAFETY: `position` is below the slot count, which `take` does
            // not change.
            let holds_value = unsafe { self.slots.as_slice().get_unchecked(position).is_odd() };
            if holds_value {
                // SAFETY: the slot was just found to hold a value, and
                // `position` is its position.
                drop(unsafe { self.take(index_of::<C>(position), position) });
            }
        }
        debug_assert!(self.is_empty());
    }

    /// Removes every value and every slot, keeping the allocation.
    ///
    /// Generations start over, so unlike [`clear`](Self::clear) this lets a
    /// key from before the call match a value inserted after it.
    #[inline]
    pub fn reset(&mut self) {
        // Reset the bookkeeping first. If a value's `drop` panics inside
        // `clear`, the storage is already empty, and a free list or `len`
        // that still described the old slots would let the next insert read
        // past the end of the storage.
        self.next_free = no_slot::<C>();
        self.len = Idx::<C>::ZERO;
        self.slots.clear();
    }

    /// Keeps only the values for which `f` returns `true`. `f` may mutate
    /// them. If `f` panics, the values it already rejected stay removed and
    /// the rest stay in the map.
    pub fn retain<F>(&mut self, mut f: F)
    where
        F: FnMut(Key<MapKeyConfig<C>>, &mut T) -> bool,
    {
        for position in 0..self.slots.len() {
            // SAFETY: `position` is below the slot count, which neither
            // `take` nor `f` (which never sees the map) changes.
            let slot = unsafe { self.slots.as_mut_slice().get_unchecked_mut(position) };
            let Parity::Odd(generation, value) = slot.as_parity_mut() else {
                continue;
            };
            // SAFETY: `position` is the slot's position.
            let (idx, keep) = unsafe {
                let idx = index_of::<C>(position);
                (idx, f(slot_key::<C>(idx, generation), value))
            };
            if !keep {
                // SAFETY: the slot still holds its value. `f` only had
                // `&mut T` and could not have removed it.
                drop(unsafe { self.take(idx, position) });
            }
        }
    }

    /// Removes every value, yielding each with its key.
    #[inline]
    pub fn drain(&mut self) -> Drain<'_, T, C> {
        Drain {
            map: self,
            position: 0,
        }
    }

    /// Iterates over every value together with its key, in slot order.
    #[inline]
    pub fn iter(&self) -> Iter<'_, T, C> {
        Iter {
            slots: self.slots.as_slice().iter().enumerate(),
            remaining: self.len(),
        }
    }

    /// Iterates over every value mutably together with its key, in slot order.
    #[inline]
    pub fn iter_mut(&mut self) -> IterMut<'_, T, C> {
        let remaining = self.len();
        IterMut {
            slots: self.slots.as_mut_slice().iter_mut().enumerate(),
            remaining,
        }
    }

    /// Iterates over every key, in slot order.
    #[inline]
    pub fn keys(&self) -> Keys<'_, T, C> {
        Keys { inner: self.iter() }
    }

    /// Iterates over every value, in slot order.
    #[inline]
    pub fn values(&self) -> Values<'_, T, C> {
        Values { inner: self.iter() }
    }

    /// Iterates over every value mutably, in slot order.
    #[inline]
    pub fn values_mut(&mut self) -> ValuesMut<'_, T, C> {
        ValuesMut {
            inner: self.iter_mut(),
        }
    }
}

impl<T, C: MapConfigFor<T>> Default for GenMap<T, C> {
    #[inline]
    fn default() -> Self {
        Self::new_with_config()
    }
}

impl<T, C: MapConfigFor<T>> Index<Key<MapKeyConfig<C>>> for GenMap<T, C> {
    type Output = T;

    /// Returns a reference to the value corresponding to `key`.
    ///
    /// # Panics
    ///
    /// Panics if the map has no value for `key`. Use [`get`](Self::get) to
    /// get `None` instead.
    #[inline]
    fn index(&self, key: Key<MapKeyConfig<C>>) -> &T {
        self.get(key).expect("invalid GenMap key")
    }
}

impl<T, C: MapConfigFor<T>> IndexMut<Key<MapKeyConfig<C>>> for GenMap<T, C> {
    /// Returns a mutable reference to the value corresponding to `key`.
    ///
    /// # Panics
    ///
    /// Panics if the map has no value for `key`. Use
    /// [`get_mut`](Self::get_mut) to get `None` instead.
    #[inline]
    fn index_mut(&mut self, key: Key<MapKeyConfig<C>>) -> &mut T {
        self.get_mut(key).expect("invalid GenMap key")
    }
}

impl<T: fmt::Debug, C: MapConfigFor<T>> fmt::Debug for GenMap<T, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

/// Pushes `slot` onto a storage that is being filled with clones of another
/// storage of the same type, and panics if the push fails. That only
/// happens with a storage that cannot hold as many slots as another of its
/// type. The slot is dropped with its value in that case.
fn push_cloned<T, C: MapConfigFor<T>>(slots: &mut Slots<T, C>, slot: MapSlot<T, C>) {
    if slots.try_push(slot).is_err() {
        panic!("SlotStorage::try_push failed while cloning a storage of the same type");
    }
}

/// Empties the slot storage on drop. It is only dropped while unwinding out
/// of `clone_from`, where a half cloned storage would disagree with `len` and
/// the free list.
struct ClearOnUnwind<'a, T, C: MapConfigFor<T>>(&'a mut Slots<T, C>);

impl<T, C: MapConfigFor<T>> Drop for ClearOnUnwind<'_, T, C> {
    fn drop(&mut self) {
        SlotStorage::clear(self.0);
    }
}

impl<T: Clone, C: MapConfigFor<T>> Clone for GenMap<T, C> {
    /// The clone has the same slots, free list and generations, so every key
    /// of the original works on it.
    fn clone(&self) -> Self {
        let mut slots = Slots::<T, C>::with_capacity(self.slots.len());
        for slot in self.slots.as_slice() {
            push_cloned::<T, C>(&mut slots, slot.clone());
        }
        Self {
            slots,
            next_free: self.next_free,
            len: self.len,
        }
    }

    /// Reuses this map's allocation if it is large enough, and otherwise
    /// makes one of the right size before cloning any slot. If a value's
    /// `clone` panics, this map is left empty.
    fn clone_from(&mut self, source: &Self) {
        self.next_free = no_slot::<C>();
        self.len = Idx::<C>::ZERO;
        let guard: ClearOnUnwind<'_, T, C> = ClearOnUnwind(&mut self.slots);
        SlotStorage::clear(guard.0);
        // An allocation that is too small would grow several times while the
        // slots are pushed, so it is swapped for one of the right size.
        if guard.0.capacity() < source.slots.len() {
            *guard.0 = Slots::<T, C>::with_capacity(source.slots.len());
        }
        for slot in source.slots.as_slice() {
            push_cloned::<T, C>(guard.0, slot.clone());
        }
        core::mem::forget(guard);
        self.next_free = source.next_free;
        self.len = source.len;
    }
}

/// Iterator over `(key, &value)` pairs. It is created using [`GenMap::iter`].
pub struct Iter<'a, T, C: MapConfigFor<T>> {
    slots: Enumerate<core::slice::Iter<'a, MapSlot<T, C>>>,
    remaining: usize,
}

impl<'a, T, C: MapConfigFor<T>> Iterator for Iter<'a, T, C> {
    type Item = (Key<MapKeyConfig<C>>, &'a T);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        for (position, slot) in self.slots.by_ref() {
            if let Parity::Odd(generation, value) = slot.as_parity() {
                self.remaining -= 1;
                // SAFETY: `position` is where the slot sits in the backing
                // storage.
                return Some((
                    unsafe { slot_key::<C>(index_of::<C>(position), generation) },
                    value,
                ));
            }
        }
        None
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<T, C: MapConfigFor<T>> DoubleEndedIterator for Iter<'_, T, C> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        while let Some((position, slot)) = self.slots.next_back() {
            if let Parity::Odd(generation, value) = slot.as_parity() {
                self.remaining -= 1;
                // SAFETY: `position` is where the slot sits in the backing
                // storage.
                return Some((
                    unsafe { slot_key::<C>(index_of::<C>(position), generation) },
                    value,
                ));
            }
        }
        None
    }
}

impl<T, C: MapConfigFor<T>> ExactSizeIterator for Iter<'_, T, C> {}
impl<T, C: MapConfigFor<T>> FusedIterator for Iter<'_, T, C> {}

impl<T, C: MapConfigFor<T>> Clone for Iter<'_, T, C> {
    fn clone(&self) -> Self {
        Self {
            slots: self.slots.clone(),
            remaining: self.remaining,
        }
    }
}

/// Iterator over `(key, &mut value)` pairs. It is created using
/// [`GenMap::iter_mut`].
pub struct IterMut<'a, T, C: MapConfigFor<T>> {
    slots: Enumerate<core::slice::IterMut<'a, MapSlot<T, C>>>,
    remaining: usize,
}

impl<'a, T, C: MapConfigFor<T>> Iterator for IterMut<'a, T, C> {
    type Item = (Key<MapKeyConfig<C>>, &'a mut T);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        for (position, slot) in self.slots.by_ref() {
            if let Parity::Odd(generation, value) = slot.as_parity_mut() {
                self.remaining -= 1;
                // SAFETY: `position` is where the slot sits in the backing
                // storage.
                return Some((
                    unsafe { slot_key::<C>(index_of::<C>(position), generation) },
                    value,
                ));
            }
        }
        None
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<T, C: MapConfigFor<T>> DoubleEndedIterator for IterMut<'_, T, C> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        while let Some((position, slot)) = self.slots.next_back() {
            if let Parity::Odd(generation, value) = slot.as_parity_mut() {
                self.remaining -= 1;
                // SAFETY: `position` is where the slot sits in the backing
                // storage.
                return Some((
                    unsafe { slot_key::<C>(index_of::<C>(position), generation) },
                    value,
                ));
            }
        }
        None
    }
}

impl<T, C: MapConfigFor<T>> ExactSizeIterator for IterMut<'_, T, C> {}
impl<T, C: MapConfigFor<T>> FusedIterator for IterMut<'_, T, C> {}

/// Iterator over keys. It is created using [`GenMap::keys`].
pub struct Keys<'a, T, C: MapConfigFor<T>> {
    inner: Iter<'a, T, C>,
}

impl<T, C: MapConfigFor<T>> Iterator for Keys<'_, T, C> {
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

impl<T, C: MapConfigFor<T>> DoubleEndedIterator for Keys<'_, T, C> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(|(key, _)| key)
    }
}

impl<T, C: MapConfigFor<T>> ExactSizeIterator for Keys<'_, T, C> {}
impl<T, C: MapConfigFor<T>> FusedIterator for Keys<'_, T, C> {}

impl<T, C: MapConfigFor<T>> Clone for Keys<'_, T, C> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

/// Iterator over shared references to values. It is created using
/// [`GenMap::values`].
pub struct Values<'a, T, C: MapConfigFor<T>> {
    inner: Iter<'a, T, C>,
}

impl<'a, T, C: MapConfigFor<T>> Iterator for Values<'a, T, C> {
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

impl<T, C: MapConfigFor<T>> DoubleEndedIterator for Values<'_, T, C> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(|(_, value)| value)
    }
}

impl<T, C: MapConfigFor<T>> ExactSizeIterator for Values<'_, T, C> {}
impl<T, C: MapConfigFor<T>> FusedIterator for Values<'_, T, C> {}

impl<T, C: MapConfigFor<T>> Clone for Values<'_, T, C> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

/// Iterator over mutable references to values. It is created using
/// [`GenMap::values_mut`].
pub struct ValuesMut<'a, T, C: MapConfigFor<T>> {
    inner: IterMut<'a, T, C>,
}

impl<'a, T, C: MapConfigFor<T>> Iterator for ValuesMut<'a, T, C> {
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

impl<T, C: MapConfigFor<T>> DoubleEndedIterator for ValuesMut<'_, T, C> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(|(_, value)| value)
    }
}

impl<T, C: MapConfigFor<T>> ExactSizeIterator for ValuesMut<'_, T, C> {}
impl<T, C: MapConfigFor<T>> FusedIterator for ValuesMut<'_, T, C> {}

/// Owning iterator over `(key, value)` pairs. It is created by consuming a
/// map with `into_iter`, which a map only has when its storage implements
/// `IntoIterator`. It implements `DoubleEndedIterator`, which gives it
/// `next_back` and `rev`, only when the storage's iterator implements both
/// `DoubleEndedIterator` and `ExactSizeIterator`.
pub struct IntoIter<T, C: MapConfigFor<T>>
where
    Slots<T, C>: IntoIterator<Item = MapSlot<T, C>>,
{
    slots: Enumerate<<Slots<T, C> as IntoIterator>::IntoIter>,
    remaining: usize,
}

impl<T, C: MapConfigFor<T>> Iterator for IntoIter<T, C>
where
    Slots<T, C>: IntoIterator<Item = MapSlot<T, C>>,
{
    type Item = (Key<MapKeyConfig<C>>, T);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        for (position, slot) in self.slots.by_ref() {
            if let Parity::Odd(generation, value) = slot.into_parity() {
                self.remaining -= 1;
                // SAFETY: `position` is where the slot sat in the storage.
                return Some((
                    unsafe { slot_key::<C>(index_of::<C>(position), generation) },
                    value,
                ));
            }
        }
        None
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

// `next_back` calls `Enumerate::next_back`, which needs the storage's
// iterator to implement `ExactSizeIterator` as well as
// `DoubleEndedIterator`. When it takes the last slot, it works out that
// slot's position as the number of slots already taken from the front plus
// `len()`, the number of slots still left.
impl<T, C: MapConfigFor<T>> DoubleEndedIterator for IntoIter<T, C>
where
    Slots<T, C>: IntoIterator<Item = MapSlot<T, C>>,
    <Slots<T, C> as IntoIterator>::IntoIter: DoubleEndedIterator + ExactSizeIterator,
{
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        while let Some((position, slot)) = self.slots.next_back() {
            if let Parity::Odd(generation, value) = slot.into_parity() {
                self.remaining -= 1;
                // SAFETY: `position` is where the slot sat in the storage.
                return Some((
                    unsafe { slot_key::<C>(index_of::<C>(position), generation) },
                    value,
                ));
            }
        }
        None
    }
}

impl<T, C: MapConfigFor<T>> ExactSizeIterator for IntoIter<T, C> where
    Slots<T, C>: IntoIterator<Item = MapSlot<T, C>>
{
}

impl<T, C: MapConfigFor<T>> FusedIterator for IntoIter<T, C> where
    Slots<T, C>: IntoIterator<Item = MapSlot<T, C>>
{
}

impl<T, C: MapConfigFor<T>> IntoIterator for GenMap<T, C>
where
    Slots<T, C>: IntoIterator<Item = MapSlot<T, C>>,
{
    type Item = (Key<MapKeyConfig<C>>, T);
    type IntoIter = IntoIter<T, C>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        IntoIter {
            remaining: self.len(),
            slots: self.slots.into_iter().enumerate(),
        }
    }
}

impl<'a, T, C: MapConfigFor<T>> IntoIterator for &'a GenMap<T, C> {
    type Item = (Key<MapKeyConfig<C>>, &'a T);
    type IntoIter = Iter<'a, T, C>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T, C: MapConfigFor<T>> IntoIterator for &'a mut GenMap<T, C> {
    type Item = (Key<MapKeyConfig<C>>, &'a mut T);
    type IntoIter = IterMut<'a, T, C>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

/// Draining iterator over `(key, value)` pairs. It is created using
/// [`GenMap::drain`]. Dropping it removes whatever has not been yielded yet.
pub struct Drain<'a, T, C: MapConfigFor<T>> {
    map: &'a mut GenMap<T, C>,
    position: usize,
}

impl<T, C: MapConfigFor<T>> Iterator for Drain<'_, T, C> {
    type Item = (Key<MapKeyConfig<C>>, T);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        while self.position < self.map.slots.len() {
            let position = self.position;
            self.position += 1;
            // SAFETY: `position` is below the slot count, which `take` does
            // not change.
            let slot = unsafe { self.map.slots.as_slice().get_unchecked(position) };
            if let Parity::Odd(generation, _) = slot.as_parity() {
                // SAFETY: the slot holds a value and `position` is its
                // position.
                unsafe {
                    let idx = index_of::<C>(position);
                    let key = slot_key::<C>(idx, generation);
                    return Some((key, self.map.take(idx, position)));
                }
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

impl<T, C: MapConfigFor<T>> ExactSizeIterator for Drain<'_, T, C> {}
impl<T, C: MapConfigFor<T>> FusedIterator for Drain<'_, T, C> {}

impl<T, C: MapConfigFor<T>> Drop for Drain<'_, T, C> {
    fn drop(&mut self) {
        for _ in self.by_ref() {}
    }
}
