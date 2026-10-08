#[cfg(feature = "alloc")]
use crate::config::DefaultMapConfig;
use crate::config::{GenMapConfig, KeyConfig, MapConfig};
use crate::error::{
    FullError, GetDisjointMutAtError, GetDisjointMutError, InsertError, InsertWithError,
};
use crate::key::Key;
use crate::key_piece::KeyPiece;
use crate::parity::{Even, Odd};
use crate::slot::{Parity, ParityMut, ParityRef, Slot};
use crate::storage::{ReserveStorage, SliceStorage};
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

/// A [`GenMap`] keeps each of its values in a [`Slot`] of this type. While the
/// slot holds a value, its generation is odd. While it holds no value, its
/// generation is even and it stores an index instead. A slot on the free list
/// stores the index of the next free slot, or the largest value of the index
/// type if it is the last free slot. No slot ever has that index, so it can't
/// be mistaken for the index of a real slot. A detached slot stores its own
/// index, which is how [`GenMap::reattach`] tells a detached slot apart from a
/// free or retired one. When the map retires a slot, it stores the largest
/// value of the index type there, so the slot never looks detached.
/// [`Slot::as_parity`] checks the parity of the generation and returns either
/// the value in [`ParityRef::Odd`] or the index in [`ParityRef::Even`].
pub type MapSlot<T, C> = Slot<MapGen<C>, T, MapIdx<C>>;

/// Returns the key of the value in the slot at `idx`, given the slot's current
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
type Slots<T, C> = <C as GenMapConfig>::Storage<MapSlot<T, C>>;

/// The largest index a key of `C` can hold.
#[inline]
fn max_idx<C: MapConfig>() -> Idx<C> {
    <MapKeyConfig<C> as KeyConfig>::max_idx()
}

/// The largest index a slot can have. It is the largest index a key of `C` can
/// hold, except that no slot ever gets the largest value of the index type.
/// That value is [`no_slot`]. Leaving it out also caps the number of slots, and
/// so the number of values, at the largest value of the index type, so the map
/// can count its values in that type.
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

/// Adds one to a map's `len` when the map adds a value. No slot has the largest
/// value of the index type as its index, so a map has at most as many slots as
/// that largest value. Every value has a slot of its own, and the new value
/// already has one of them, so `len` is below that largest value beforehand and
/// the add cannot overflow.
#[inline]
pub(crate) fn increment_len<I: KeyPiece>(len: &mut I) {
    debug_assert!(
        *len < I::MAX,
        "a map never holds as many values as the largest value of its index type"
    );
    *len = len.wrapping_add(I::ONE);
}

/// Takes one off a map's `len`. The map only calls this when it takes out a
/// value it holds, so `len` is above zero beforehand.
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

/// Returns `true` if `slot` is detached under `key`.
///
/// When a key's value is detached, the slot's generation is set to what
/// [`detached_generation`] returns for the key's generation, and the slot's
/// `U` is set to the slot's own index. A slot on the free list has another
/// slot's index or [`no_slot`] in its `U`, and a retired slot has `no_slot`.
/// So a slot is detached under a key only if it has that generation and its
/// own index in its `U`.
#[inline]
fn is_detached<T, C: GenMapConfig>(slot: &MapSlot<T, C>, key: Key<MapKeyConfig<C>>) -> bool {
    slot.get_even(detached_generation::<C>(key.generation()))
        .is_some_and(|link| *link == key.idx())
}

/// Returns the generation and the link to give the slot at `idx` once the
/// value it held under `generation` is gone. Unless the slot retires, the link
/// is the old head of the free list, and `next_free` becomes `idx`. A slot
/// whose generation has run out starts over at zero if `C` wraps, and
/// otherwise retires, which keeps it off the free list for good.
#[inline]
fn freed_parts<C: GenMapConfig>(
    next_free: &mut Idx<C>,
    idx: Idx<C>,
    generation: Odd<Gen<C>>,
) -> (Even<Gen<C>>, Idx<C>) {
    match next_generation::<C>(generation) {
        Some(next) => (next, core::mem::replace(next_free, idx)),
        None if C::WRAP_ON_OVERFLOW => (Even::ZERO, core::mem::replace(next_free, idx)),
        None => (Even::ZERO, no_slot::<C>()),
    }
}

/// The error the storage of a `GenMap<T, C>` gives when it cannot make room
/// for another slot. It is [`ReserveError`](crate::ReserveError) for a
/// [`SingleVec`](crate::SingleVec), `TryReserveError` for a `Vec`,
/// `CapacityError` for an `ArrayVec` and `SmallVecError` for a `SmallVec`.
pub type StorageError<T, C> = <<C as GenMapConfig>::Storage<MapSlot<T, C>> as SliceStorage>::Error;

/// Where the next inserted value will go, worked out before anything is
/// written.
struct Target<C: MapConfig> {
    idx: Idx<C>,
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
        // SAFETY: a `Target` only ever comes from `next_target`, which picks an
        // index and a generation that a key can hold.
        Key::from_repr(unsafe {
            <MapKeyConfig<C> as KeyConfig>::pack_unchecked(self.idx, self.generation)
        })
    }
}

/// Panics with the message `insert` gives for a full map. It is kept out of
/// line so that the insert paths stay small.
#[cold]
#[inline(never)]
fn panic_full<T, C: GenMapConfig>(full: FullError<StorageError<T, C>>, slots_len: usize) -> ! {
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
pub struct VacantEntry<'a, T, C: GenMapConfig> {
    map: &'a mut GenMap<T, C>,
    target: Target<C>,
}

impl<'a, T, C: GenMapConfig> VacantEntry<'a, T, C> {
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

impl<T, C: GenMapConfig> fmt::Debug for VacantEntry<'_, T, C> {
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
/// your own config, implement [`MapConfig`] and [`GenMapConfig`] for it.
///
/// See the [crate documentation](crate) for examples.
pub struct GenMap<
    T,
    #[cfg(feature = "alloc")] C: GenMapConfig = DefaultMapConfig,
    #[cfg(not(feature = "alloc"))] C: GenMapConfig,
> {
    slots: Slots<T, C>,
    /// The index of the first slot on the free list, or `no_slot` if no slot
    /// is free.
    next_free: Idx<C>,
    /// The number of values. No slot has the largest value of the index type as
    /// its index, so there are at most as many slots as that largest value, and
    /// the count of values always fits in the index type.
    len: Idx<C>,
}

#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
impl<T> GenMap<T> {
    /// Creates an empty map with the [`DefaultMapConfig`].
    ///
    /// This method only exists for the default config, so that `GenMap::new()`
    /// compiles without a type annotation. Rust does not fall back to a default
    /// type parameter when it infers types, so a `new` for every config would
    /// leave the config unknown. Use [`new_with_config`](Self::new_with_config)
    /// for any other config.
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
impl<T, C: GenMapConfig> GenMap<T, C>
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
    /// Panics if the storage cannot make the room. Use
    /// [`try_reserve`](Self::try_reserve) to get an error instead.
    #[inline]
    pub fn reserve(&mut self, additional: usize) {
        if let Err(error) = self.try_reserve(additional) {
            panic!("GenMap cannot make room for {additional} more slots: {error:?}");
        }
    }

    /// The fallible form of [`reserve`](Self::reserve). After it returns `Ok`,
    /// the next `additional` inserts cannot fail for lack of storage.
    ///
    /// # Errors
    ///
    /// Returns the storage's error if the storage cannot make the room.
    #[inline]
    pub fn try_reserve(&mut self, additional: usize) -> Result<(), StorageError<T, C>> {
        self.slots.ensure_room(additional)
    }
}

impl<T, C: GenMapConfig> GenMap<T, C> {
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
    /// impl GenMapConfig for Wide {
    ///     type Storage<S: GenSlotItem> = Vec<S>;
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
            ParityRef::Odd(&generation, value) => {
                // SAFETY: `idx` is the slot's position.
                Some((unsafe { slot_key::<C>(idx, generation) }, value))
            }
            ParityRef::Even(..) => None,
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
            ParityMut::Odd(&mut generation, value) => {
                // SAFETY: `idx` is the slot's position.
                Some((unsafe { slot_key::<C>(idx, generation) }, value))
            }
            ParityMut::Even(..) => None,
        }
    }

    /// Returns the key and a reference to the value in the slot at `idx`, like
    /// [`get_at`](Self::get_at), but without checking that there is a slot at
    /// `idx` or that it holds a value.
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

    /// Returns a mutable reference to the value of each key, like
    /// [`get_disjoint_mut`](Self::get_disjoint_mut), but without any of its
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
    /// impl GenMapConfig for Tiny {
    ///     type Storage<S: GenSlotItem> = Vec<S>;
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
    /// [`VacantEntry::key`] is the key the value will get, and
    /// [`VacantEntry::insert`] puts the value in the slot. Dropping the entry
    /// inserts nothing.
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
        let (idx, generation, from_free_list) = if self.next_free != no_slot::<C>() {
            let idx = self.next_free;
            // SAFETY: `idx` is on the free list, so it was the position of a
            // slot that still exists, and that slot holds no value, so its
            // generation is even.
            let generation = unsafe {
                let slot = self.slots.as_slice().get_unchecked(position_of::<C>(idx));
                Even::new_unchecked(slot.generation())
            };
            (idx, generation, true)
        } else {
            let idx = Idx::<C>::from_usize(self.slots.len())
                .filter(|idx| *idx <= max_slot_idx::<C>())
                .ok_or(FullError::IndexExhausted)?;
            self.slots.ensure_room(1).map_err(FullError::StorageFull)?;
            (idx, Even::ZERO, false)
        };

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
            // SAFETY: `target.idx` is the index of the slot that was first on
            // the free list when `next_target` looked, and nothing has changed
            // since, so that slot still exists.
            let slot = unsafe {
                self.slots
                    .as_mut_slice()
                    .get_unchecked_mut(position_of::<C>(target.idx))
            };
            // SAFETY: the slot came off the free list, so it holds no value
            // and its generation is even.
            self.next_free = unsafe { slot.replace_even_unchecked(target.generation, value) };
        } else {
            let slot = Slot::new_odd(target.generation, value);
            // `next_target` made room for this slot, and `SliceStorage`
            // promises that `try_push` succeeds after that, so only a broken
            // storage refuses the push. The slot is dropped with its value in
            // that case.
            if self.slots.try_push(slot).is_err() {
                panic!("SliceStorage::try_push failed although ensure_room returned Ok");
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
    /// /// Maps with this config have at most sixteen slots, and a slot's
    /// /// generation wraps after the slot has held eight values.
    /// struct Wrapping;
    ///
    /// impl MapConfig for Wrapping {
    ///     type KeyConfig = Packed<u8, 4>;
    /// }
    ///
    /// impl GenMapConfig for Wrapping {
    ///     const WRAP_ON_OVERFLOW: bool = true;
    ///     type Storage<S: GenSlotItem> = Vec<S>;
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
        // A retired slot has generation zero and `no_slot` as its link. This
        // method does not put the slot on the free list, so no insert can use
        // it again.
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
        let (next, link) = freed_parts::<C>(&mut self.next_free, idx, generation);
        // SAFETY: the slot's generation is odd.
        unsafe { slot.replace_odd_unchecked(next, link) }
    }

    /// Takes the value out of the map while keeping its slot reserved for
    /// `key`, so that [`reattach`](Self::reattach) can put a value back under
    /// the same key. Returns `None` if there is no value for `key`.
    ///
    /// Until `reattach` puts a value back or [`release`](Self::release) frees
    /// the slot, [`contains_key`](Self::contains_key) returns `false` for the
    /// key and [`len`](Self::len) does not count the value, but the slot is not
    /// on the free list, so no insert uses it.
    ///
    /// While the slot is detached, its generation is one more than the key's
    /// generation, or zero if the key's generation is the largest value of the
    /// generation type. That generation is even, so no key matches it, and
    /// `reattach` gives the slot the key's generation back.
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
    /// // This insert does not take the detached slot.
    /// let other = map.insert(2);
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
    /// detached, or was detached and its slot has since been freed by
    /// [`release`](Self::release) or removed by [`reset`](Self::reset).
    #[inline]
    pub fn reattach(&mut self, key: Key<MapKeyConfig<C>>, value: T) -> Result<(), T> {
        let Some(slot) = key
            .idx()
            .into_usize()
            .and_then(|position| self.slots.as_mut_slice().get_mut(position))
            .filter(|slot| is_detached::<T, C>(slot, key))
        else {
            return Err(value);
        };
        // SAFETY: a detached slot's generation is even.
        unsafe { slot.replace_even_unchecked(key.generation(), value) };
        increment_len(&mut self.len);
        Ok(())
    }

    /// Frees the slot of a key whose value [`detach`](Self::detach) took out,
    /// without putting a value back. The slot goes on the free list or retires,
    /// just as it would if the value had been removed, so
    /// [`reattach`](Self::reattach) fails for the key. Returns `false`, and
    /// leaves the map as it was, if the key is not detached.
    ///
    /// # Examples
    ///
    /// ```
    /// use gen_map::GenMap;
    ///
    /// let mut map = GenMap::new();
    /// let key = map.insert(1);
    /// map.detach(key).unwrap();
    ///
    /// assert!(map.release(key));
    /// assert_eq!(map.reattach(key, 2), Err(2));
    ///
    /// // The next insert reuses the freed slot under a newer key.
    /// let other = map.insert(3);
    /// assert_eq!(other.idx(), key.idx());
    /// assert_ne!(other, key);
    /// ```
    #[inline]
    pub fn release(&mut self, key: Key<MapKeyConfig<C>>) -> bool {
        let Some(slot) = key
            .idx()
            .into_usize()
            .and_then(|position| self.slots.as_mut_slice().get_mut(position))
            .filter(|slot| is_detached::<T, C>(slot, key))
        else {
            return false;
        };
        let (next, link) = freed_parts::<C>(&mut self.next_free, key.idx(), key.generation());
        slot.set_even(next, link);
        true
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
    /// The slots that the map adds afterwards start again at generation zero,
    /// so unlike [`clear`](Self::clear), this method lets a key from before the
    /// call match a value inserted after it. Once this method is called, the
    /// map behaves like a new map created with the same
    /// [`capacity`](Self::capacity).
    #[inline]
    pub fn reset(&mut self) {
        // The free list and `len` are reset before the storage drops the slots.
        // If a value's `drop` panics while the storage clears, the storage can
        // already be empty, and a free list that still described the
        // old slots would let the next insert read past the end of the storage.
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
            let ParityMut::Odd(&mut generation, value) = slot.as_parity_mut() else {
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

impl<T, C: GenMapConfig> Default for GenMap<T, C> {
    #[inline]
    fn default() -> Self {
        Self::new_with_config()
    }
}

impl<T, C: GenMapConfig> Index<Key<MapKeyConfig<C>>> for GenMap<T, C> {
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

impl<T, C: GenMapConfig> IndexMut<Key<MapKeyConfig<C>>> for GenMap<T, C> {
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

impl<T: fmt::Debug, C: GenMapConfig> fmt::Debug for GenMap<T, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

/// Pushes `slot` onto a storage that is being filled with clones of another
/// storage of the same type, and panics if the push fails. That only
/// happens with a storage that cannot hold as many slots as another of its
/// type. The slot is dropped with its value in that case.
fn push_cloned<T, C: GenMapConfig>(slots: &mut Slots<T, C>, slot: MapSlot<T, C>) {
    if slots.try_push(slot).is_err() {
        panic!("SliceStorage::try_push failed while cloning a storage of the same type");
    }
}

/// Empties the slot storage on drop. It is only dropped while unwinding out
/// of `clone_from`, where a half cloned storage would disagree with `len` and
/// the free list.
struct ClearOnUnwind<'a, T, C: GenMapConfig>(&'a mut Slots<T, C>);

impl<T, C: GenMapConfig> Drop for ClearOnUnwind<'_, T, C> {
    fn drop(&mut self) {
        SliceStorage::clear(self.0);
    }
}

impl<T: Clone, C: GenMapConfig> Clone for GenMap<T, C> {
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
        SliceStorage::clear(guard.0);
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
pub struct Iter<'a, T, C: MapConfig> {
    slots: Enumerate<core::slice::Iter<'a, MapSlot<T, C>>>,
    remaining: usize,
}

impl<'a, T, C: MapConfig> Iterator for Iter<'a, T, C> {
    type Item = (Key<MapKeyConfig<C>>, &'a T);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        for (position, slot) in self.slots.by_ref() {
            if let ParityRef::Odd(&generation, value) = slot.as_parity() {
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

impl<T, C: MapConfig> DoubleEndedIterator for Iter<'_, T, C> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        while let Some((position, slot)) = self.slots.next_back() {
            if let ParityRef::Odd(&generation, value) = slot.as_parity() {
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

impl<T, C: MapConfig> ExactSizeIterator for Iter<'_, T, C> {}
impl<T, C: MapConfig> FusedIterator for Iter<'_, T, C> {}

impl<T, C: MapConfig> Clone for Iter<'_, T, C> {
    fn clone(&self) -> Self {
        Self {
            slots: self.slots.clone(),
            remaining: self.remaining,
        }
    }
}

/// Iterator over `(key, &mut value)` pairs. It is created using
/// [`GenMap::iter_mut`].
pub struct IterMut<'a, T, C: MapConfig> {
    slots: Enumerate<core::slice::IterMut<'a, MapSlot<T, C>>>,
    remaining: usize,
}

impl<'a, T, C: MapConfig> Iterator for IterMut<'a, T, C> {
    type Item = (Key<MapKeyConfig<C>>, &'a mut T);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        for (position, slot) in self.slots.by_ref() {
            if let ParityMut::Odd(&mut generation, value) = slot.as_parity_mut() {
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

impl<T, C: MapConfig> DoubleEndedIterator for IterMut<'_, T, C> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        while let Some((position, slot)) = self.slots.next_back() {
            if let ParityMut::Odd(&mut generation, value) = slot.as_parity_mut() {
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

impl<T, C: MapConfig> ExactSizeIterator for IterMut<'_, T, C> {}
impl<T, C: MapConfig> FusedIterator for IterMut<'_, T, C> {}

/// Iterator over keys. It is created using [`GenMap::keys`].
pub struct Keys<'a, T, C: MapConfig> {
    inner: Iter<'a, T, C>,
}

impl<T, C: MapConfig> Iterator for Keys<'_, T, C> {
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

impl<T, C: MapConfig> DoubleEndedIterator for Keys<'_, T, C> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(|(key, _)| key)
    }
}

impl<T, C: MapConfig> ExactSizeIterator for Keys<'_, T, C> {}
impl<T, C: MapConfig> FusedIterator for Keys<'_, T, C> {}

impl<T, C: MapConfig> Clone for Keys<'_, T, C> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

/// Iterator over shared references to values. It is created using
/// [`GenMap::values`].
pub struct Values<'a, T, C: MapConfig> {
    inner: Iter<'a, T, C>,
}

impl<'a, T, C: MapConfig> Iterator for Values<'a, T, C> {
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

impl<T, C: MapConfig> DoubleEndedIterator for Values<'_, T, C> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(|(_, value)| value)
    }
}

impl<T, C: MapConfig> ExactSizeIterator for Values<'_, T, C> {}
impl<T, C: MapConfig> FusedIterator for Values<'_, T, C> {}

impl<T, C: MapConfig> Clone for Values<'_, T, C> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

/// Iterator over mutable references to values. It is created using
/// [`GenMap::values_mut`].
pub struct ValuesMut<'a, T, C: MapConfig> {
    inner: IterMut<'a, T, C>,
}

impl<'a, T, C: MapConfig> Iterator for ValuesMut<'a, T, C> {
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

impl<T, C: MapConfig> DoubleEndedIterator for ValuesMut<'_, T, C> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(|(_, value)| value)
    }
}

impl<T, C: MapConfig> ExactSizeIterator for ValuesMut<'_, T, C> {}
impl<T, C: MapConfig> FusedIterator for ValuesMut<'_, T, C> {}

/// Owning iterator over `(key, value)` pairs. It is created by consuming a map
/// with `into_iter`, which a map only has when its storage implements
/// `IntoIterator`. It implements `DoubleEndedIterator`, which gives it
/// `next_back` and `rev`, only when the storage's iterator implements both
/// `DoubleEndedIterator` and `ExactSizeIterator`, because it needs the length
/// of the storage's iterator to work out the position of a slot taken from the
/// back.
pub struct IntoIter<T, C: GenMapConfig>
where
    Slots<T, C>: IntoIterator<Item = MapSlot<T, C>>,
{
    slots: Enumerate<<Slots<T, C> as IntoIterator>::IntoIter>,
    remaining: usize,
}

impl<T, C: GenMapConfig> Iterator for IntoIter<T, C>
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

// `next_back` calls `Enumerate::next_back`, which needs the storage's iterator
// to implement `ExactSizeIterator` as well as `DoubleEndedIterator`.
// `Enumerate::next_back` takes the last slot and works out its position as the
// number of slots already taken from the front plus `len()`, which is the
// number of slots still left once that slot is taken.
impl<T, C: GenMapConfig> DoubleEndedIterator for IntoIter<T, C>
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

impl<T, C: GenMapConfig> ExactSizeIterator for IntoIter<T, C> where
    Slots<T, C>: IntoIterator<Item = MapSlot<T, C>>
{
}

impl<T, C: GenMapConfig> FusedIterator for IntoIter<T, C> where
    Slots<T, C>: IntoIterator<Item = MapSlot<T, C>>
{
}

impl<T, C: GenMapConfig> IntoIterator for GenMap<T, C>
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

impl<'a, T, C: GenMapConfig> IntoIterator for &'a GenMap<T, C> {
    type Item = (Key<MapKeyConfig<C>>, &'a T);
    type IntoIter = Iter<'a, T, C>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T, C: GenMapConfig> IntoIterator for &'a mut GenMap<T, C> {
    type Item = (Key<MapKeyConfig<C>>, &'a mut T);
    type IntoIter = IterMut<'a, T, C>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

/// Draining iterator over `(key, value)` pairs. It is created using
/// [`GenMap::drain`]. Dropping it removes the values it has not yielded yet.
pub struct Drain<'a, T, C: GenMapConfig> {
    map: &'a mut GenMap<T, C>,
    position: usize,
}

impl<T, C: GenMapConfig> Iterator for Drain<'_, T, C> {
    type Item = (Key<MapKeyConfig<C>>, T);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        while self.position < self.map.slots.len() {
            let position = self.position;
            self.position += 1;
            // SAFETY: `position` is below the slot count, which `take` does
            // not change.
            let slot = unsafe { self.map.slots.as_slice().get_unchecked(position) };
            if let ParityRef::Odd(&generation, _) = slot.as_parity() {
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

impl<T, C: GenMapConfig> ExactSizeIterator for Drain<'_, T, C> {}
impl<T, C: GenMapConfig> FusedIterator for Drain<'_, T, C> {}

impl<T, C: GenMapConfig> Drop for Drain<'_, T, C> {
    fn drop(&mut self) {
        for _ in self.by_ref() {}
    }
}

/// `GenMapRawParts` holds the fields of a [`GenMap`].
/// [`into_raw_parts`](GenMap::into_raw_parts) takes a map apart into these
/// fields, and [`from_raw_parts`](GenMap::from_raw_parts) builds a map from
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
/// - `len` is the number of slots that hold a value.
///
/// [`MapSlot`] explains what a free, detached or retired slot stores in place
/// of a value.
///
/// # Examples
///
/// ```
/// use gen_map::{GenMap, GenMapRawParts, Odd, Slot};
///
/// // The only slot holds "a" under generation one, so no slot is free.
/// let parts = GenMapRawParts {
///     slots: [Slot::new_odd(Odd::new(1).unwrap(), "a")].into_iter().collect(),
///     next_free: u32::MAX,
///     len: 1,
/// };
/// // SAFETY: the slot's position and generation fit the default key config,
/// // the free list is empty, and `len` counts the one value.
/// let map: GenMap<&str> = unsafe { GenMap::from_raw_parts(parts) };
/// let key = map.key_at(0).unwrap();
/// assert_eq!(map[key], "a");
/// ```
pub struct GenMapRawParts<
    T,
    #[cfg(feature = "alloc")] C: GenMapConfig = DefaultMapConfig,
    #[cfg(not(feature = "alloc"))] C: GenMapConfig,
> {
    // These fields are in the same order as the fields of `GenMap` on
    // purpose, so that the two structs can be read side by side.
    // `into_raw_parts` and `from_raw_parts` list every field of both structs,
    // so the compiler catches a field that only one of them has, but nothing
    // catches a change in order. Think twice before removing this comment,
    // because it is the only thing that keeps the two orders the same.
    /// The map keeps its slots in this storage, which the map's config picks.
    /// A slot's index is its position in the storage.
    pub slots: <C as GenMapConfig>::Storage<MapSlot<T, C>>,
    /// `next_free` holds the index of the first slot on the free list, or the
    /// largest value of the index type if no slot is free.
    pub next_free: MapIdx<C>,
    /// `len` is the number of values in the map.
    pub len: MapIdx<C>,
}

impl<T, C: GenMapConfig> GenMap<T, C> {
    /// Takes the map apart into its fields.
    /// [`from_raw_parts`](Self::from_raw_parts) builds a map from them again,
    /// and [`GenMapRawParts`] lists the rules they follow.
    ///
    /// # Examples
    ///
    /// ```
    /// use gen_map::GenMap;
    ///
    /// let mut map = GenMap::new();
    /// let a = map.insert(1);
    /// let b = map.insert(2);
    /// map.remove(a);
    ///
    /// let mut parts = map.into_raw_parts();
    /// assert_eq!(parts.slots.len(), 2);
    /// assert_eq!(parts.len, 1);
    /// // The slot that held 1 is the only free slot.
    /// assert_eq!(parts.next_free, a.idx());
    ///
    /// // Changing the value stored under `b` keeps the parts following the
    /// // rules.
    /// *parts.slots[1].get_odd_mut(b.generation()).unwrap() += 10;
    /// // SAFETY: the parts came from `into_raw_parts`, and only the value
    /// // stored under `b` changed.
    /// let map = unsafe { GenMap::from_raw_parts(parts) };
    /// assert_eq!(map[b], 12);
    /// assert!(map.get(a).is_none());
    /// ```
    #[inline]
    pub fn into_raw_parts(self) -> GenMapRawParts<T, C> {
        let Self {
            slots,
            next_free,
            len,
        } = self;
        GenMapRawParts {
            slots,
            next_free,
            len,
        }
    }

    /// Builds a map from the fields that
    /// [`into_raw_parts`](Self::into_raw_parts) takes a map apart into.
    ///
    /// # Safety
    ///
    /// `parts` must follow every rule listed on [`GenMapRawParts`]. The map's
    /// methods rely on those rules, and parts that break one can make them
    /// cause undefined behavior.
    #[inline]
    #[must_use]
    pub unsafe fn from_raw_parts(parts: GenMapRawParts<T, C>) -> Self {
        let GenMapRawParts {
            slots,
            next_free,
            len,
        } = parts;
        Self {
            slots,
            next_free,
            len,
        }
    }
}
