use crate::config::Config;
#[cfg(feature = "alloc")]
use crate::config::DefaultConfig;
use crate::error::{
    FullError, GetDisjointMutAtError, GetDisjointMutError, InsertError, InsertWithError,
};
use crate::key::Key;
use crate::key_layout::KeyLayout;
use crate::key_piece::KeyPiece;
use crate::storage::{ReserveStorage, SlotStorage};
use core::fmt;
use core::iter::{Enumerate, FusedIterator};
use core::mem::ManuallyDrop;
use core::ops::{Index, IndexMut};

/// The payload of a slot. `occupied` is live while the slot holds a value, and
/// `vacant` is live the rest of the time. For a slot on the free list,
/// `vacant` names the next free slot. The owning [`Slot`] tells the two
/// fields apart by its generation.
union SlotData<T, Idx: Copy> {
    occupied: ManuallyDrop<T>,
    vacant: Option<Idx>,
}

/// One entry of a map's storage, the `S` of a config's `Storage<S>`. It has
/// no public API.
///
/// An odd generation means the slot holds a value, and an even one means it
/// does not. Insert and remove each increment the generation, so every key
/// carries an odd generation.
pub struct Slot<T, C: Config> {
    generation: C::Gen,
    data: SlotData<T, C::Idx>,
}

impl<T, C: Config> Slot<T, C> {
    #[inline]
    fn is_occupied(&self) -> bool {
        self.generation.is_odd()
    }

    /// # Safety
    ///
    /// The slot must be occupied and `idx` must be its position in the
    /// storage.
    #[inline]
    unsafe fn key(&self, idx: C::Idx) -> Key<C> {
        // SAFETY: an occupied slot has an odd generation, so it is not zero,
        // and the map never lets a generation or a position grow past what
        // the layout holds.
        unsafe { Key::<C>::from_raw_parts(idx, self.generation.into_non_zero_unchecked()) }
    }

    /// # Safety
    ///
    /// The slot must be occupied.
    #[inline]
    unsafe fn value(&self) -> &T {
        &self.data.occupied
    }

    /// # Safety
    ///
    /// The slot must be occupied.
    #[inline]
    unsafe fn value_mut(&mut self) -> &mut T {
        &mut self.data.occupied
    }

    /// Returns `true` if the slot was detached with [`GenMap::detach`] and
    /// not reattached since. `idx` must be the slot's position.
    #[inline]
    fn is_detached(&self, idx: C::Idx) -> bool {
        // SAFETY: an even generation means `vacant` is the live field.
        !self.is_occupied() && unsafe { self.data.vacant } == Some(idx)
    }

    fn clone_slot(&self) -> Self
    where
        T: Clone,
    {
        // SAFETY: the parity of the generation says which field is live, and
        // every vacant slot in the map has had its `vacant` field written.
        let data = unsafe {
            if self.is_occupied() {
                SlotData {
                    occupied: ManuallyDrop::new(T::clone(&self.data.occupied)),
                }
            } else {
                SlotData {
                    vacant: self.data.vacant,
                }
            }
        };
        Slot {
            generation: self.generation,
            data,
        }
    }
}

impl<T, C: Config> Drop for Slot<T, C> {
    #[inline]
    fn drop(&mut self) {
        if self.is_occupied() {
            // SAFETY: an odd generation means `occupied` is the live field.
            unsafe { ManuallyDrop::drop(&mut self.data.occupied) }
        }
    }
}

/// Converts a position in the backing storage to `C::Idx`.
///
/// # Safety
///
/// `position` must be the position of a slot in the map's storage. Every such
/// position fits, because the map refuses to push a slot whose position does
/// not.
#[inline]
unsafe fn index_of<C: Config>(position: usize) -> C::Idx {
    debug_assert!(
        C::Idx::from_usize(position).is_some(),
        "every slot position fits in the configured index type"
    );
    unsafe { C::Idx::from_usize_unchecked(position) }
}

/// Converts the index of a slot back to its position in the backing storage.
///
/// # Safety
///
/// `idx` must be the index of a slot, such as one on the free list or one
/// carried by a valid key. Every such index was a slot's position, so it
/// fits in `usize`.
#[inline]
unsafe fn position_of<C: Config>(idx: C::Idx) -> usize {
    debug_assert!(
        idx.into_usize().is_some(),
        "every index of a slot fits in usize"
    );
    unsafe { idx.into_usize_unchecked() }
}

/// The storage a config gives the map for its slots.
type Slots<T, C> = <C as Config>::Storage<Slot<T, C>>;

/// The largest index a key of `C` can hold.
#[inline]
fn max_idx<C: Config>() -> C::Idx {
    <C::Layout as KeyLayout<C::Idx, C::Gen>>::max_idx()
}

/// The generation after `generation`, or `None` if a key of `C` could not
/// hold it, which is when a slot retires or wraps.
#[inline]
fn next_generation<C: Config>(generation: C::Gen) -> Option<C::Gen> {
    if generation == <C::Layout as KeyLayout<C::Idx, C::Gen>>::max_generation() {
        None
    } else {
        // The largest generation is odd, so an even one is below it and
        // adding one never wraps.
        Some(generation.wrapping_add(C::Gen::ONE))
    }
}

/// The error the storage of a `GenMap<T, C>` gives when it can not make room
/// for another slot. It is `TryReserveError` for a `Vec`, `CapacityError` for
/// an `ArrayVec` and `CollectionAllocErr` for a `SmallVec`.
pub type StorageError<T, C> =
    <<C as Config>::Storage<Slot<T, C>> as SlotStorage<Slot<T, C>>>::Error;

/// Where the next inserted value will go, worked out before anything is
/// written.
struct Target<C: Config> {
    idx: C::Idx,
    position: usize,
    /// The odd generation the new key gets.
    generation: C::Gen,
    /// `true` if the slot came off the free list, and `false` if it has to be
    /// pushed onto the storage.
    from_free_list: bool,
}

impl<C: Config> Target<C> {
    /// The key that a value put at this target gets.
    #[inline]
    fn key(&self) -> Key<C> {
        // SAFETY: a `Target` only ever comes from `next_target`, which makes
        // the generation odd, so not zero, and checks that both parts fit
        // the layout.
        unsafe { Key::<C>::from_raw_parts(self.idx, self.generation.into_non_zero_unchecked()) }
    }
}

/// Panics with the message `insert` gives for a full map. It is kept out of
/// line so that the insert paths stay small.
#[cold]
#[inline(never)]
fn panic_full<T, C: Config>(full: FullError<StorageError<T, C>>, slots_len: usize) -> ! {
    match full {
        FullError::IndexExhausted => panic!(
            "GenMap is full, its keys can not address more than {} slots",
            slots_len
        ),
        FullError::StorageFull(error) => panic!(
            "GenMap is full, its storage can not make room for more than {} slots: {:?}",
            slots_len, error
        ),
    }
}

/// The slot the next insert would use, handed out by
/// [`GenMap::vacant_entry`](GenMap::vacant_entry). Nothing is written until
/// [`insert`](Self::insert) is called, so dropping the entry leaves the map
/// as it was.
pub struct VacantEntry<'a, T, C: Config> {
    map: &'a mut GenMap<T, C>,
    target: Target<C>,
}

impl<'a, T, C: Config> VacantEntry<'a, T, C> {
    /// The key the value will get. It is invalid until
    /// [`insert`](Self::insert) is called.
    #[inline]
    pub fn key(&self) -> Key<C> {
        self.target.key()
    }

    /// Puts `value` in the slot and returns its key, which is the one
    /// [`key`](Self::key) gave.
    #[inline]
    pub fn insert(self, value: T) -> Key<C> {
        // SAFETY: `target` came from `next_target`, and this entry has held
        // `&mut` on the map since, so nothing has touched it.
        unsafe { self.map.fill(self.target, value) }
    }
}

impl<T, C: Config> fmt::Debug for VacantEntry<'_, T, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VacantEntry")
            .field("key", &self.key())
            .finish()
    }
}

/// A generational map whose key is configured by `C`.
///
/// See the [crate documentation](crate) for examples.
pub struct GenMap<
    T,
    #[cfg(feature = "alloc")] C: Config = DefaultConfig,
    #[cfg(not(feature = "alloc"))] C: Config,
> {
    slots: Slots<T, C>,
    next_free: Option<C::Idx>,
    len: usize,
}

#[cfg(feature = "alloc")]
impl<T> GenMap<T> {
    /// Creates an empty map with the [`DefaultConfig`].
    ///
    /// Use [`new_with_config`](Self::new_with_config) for any other config,
    /// since Rust does not use a default type parameter when it infers
    /// types.
    #[inline]
    pub const fn new() -> Self {
        Self::new_with_config()
    }

    /// Creates an empty map with the [`DefaultConfig`] and room for `capacity`
    /// slots.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self
    where
        Slots<T, DefaultConfig>: ReserveStorage<Slot<T, DefaultConfig>>,
    {
        Self {
            slots: Slots::<T, DefaultConfig>::with_capacity(capacity),
            next_free: None,
            len: 0,
        }
    }
}

/// These methods need a storage that can grow on request, so a map whose
/// storage has a fixed capacity does not have them.
impl<T, C: Config> GenMap<T, C>
where
    Slots<T, C>: ReserveStorage<Slot<T, C>>,
{
    /// Creates an empty map with config `C` and room for `capacity` slots.
    #[inline]
    pub fn with_capacity_and_config(capacity: usize) -> Self {
        Self {
            slots: Slots::<T, C>::with_capacity(capacity),
            next_free: None,
            len: 0,
        }
    }

    /// Reserves room for at least `additional` more slots.
    ///
    /// # Panics
    ///
    /// Panics or aborts if the storage can not make the room, as
    /// `Vec::reserve` does. Use [`try_reserve`](Self::try_reserve) to get an
    /// error instead.
    #[inline]
    pub fn reserve(&mut self, additional: usize) {
        self.slots.reserve(additional);
    }

    /// The fallible form of [`reserve`](Self::reserve). After `Ok`, the next
    /// `additional` inserts can not fail for lack of storage.
    ///
    /// # Errors
    ///
    /// Returns what the storage says when it can not make the room.
    #[inline]
    pub fn try_reserve(&mut self, additional: usize) -> Result<(), StorageError<T, C>> {
        self.slots.try_reserve(additional)
    }
}

impl<T, C: Config> GenMap<T, C> {
    /// Creates an empty map with config `C`.
    ///
    /// ```
    /// use gen_map::{Config, GenMap, Split};
    ///
    /// struct Wide;
    ///
    /// impl Config for Wide {
    ///     type Idx = u64;
    ///     type Gen = u64;
    ///     type Layout = Split;
    ///     type Storage<S> = Vec<S>;
    /// }
    ///
    /// let mut map = GenMap::<&str, Wide>::new_with_config();
    /// let key = map.insert("x");
    /// assert_eq!(core::mem::size_of_val(&key), 16);
    /// ```
    #[inline]
    pub const fn new_with_config() -> Self {
        Self {
            slots: Slots::<T, C>::EMPTY,
            next_free: None,
            len: 0,
        }
    }

    /// How many slots the storage can hold before it has to grow, or in total
    /// if it can not grow.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.slots.capacity()
    }

    /// The number of values in the map.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` if the map holds no values.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The number of slots, whether they hold a value or not.
    #[inline]
    pub fn slots_len(&self) -> usize {
        self.slots.len()
    }

    /// Returns `true` if `key` is valid.
    #[inline]
    pub fn contains_key(&self, key: Key<C>) -> bool {
        self.get(key).is_some()
    }

    /// Returns a reference to the value corresponding to `key`.
    #[inline]
    pub fn get(&self, key: Key<C>) -> Option<&T> {
        let slot = self.slots.as_slice().get(key.idx().into_usize()?)?;
        if slot.generation != key.generation() {
            return None;
        }
        // SAFETY: a key's generation is always odd, so a matching slot is
        // occupied.
        Some(unsafe { slot.value() })
    }

    /// Returns a mutable reference to the value corresponding to `key`.
    #[inline]
    pub fn get_mut(&mut self, key: Key<C>) -> Option<&mut T> {
        let slot = self.slots.as_mut_slice().get_mut(key.idx().into_usize()?)?;
        if slot.generation != key.generation() {
            return None;
        }
        // SAFETY: a key's generation is always odd, so a matching slot is
        // occupied.
        Some(unsafe { slot.value_mut() })
    }

    /// [`get`](Self::get) without the bounds and generation checks.
    ///
    /// # Safety
    ///
    /// `key` must be valid, meaning [`contains_key`](Self::contains_key)
    /// returns `true` for it.
    #[inline]
    pub unsafe fn get_unchecked(&self, key: Key<C>) -> &T {
        debug_assert!(self.contains_key(key));
        self.slots
            .as_slice()
            .get_unchecked(position_of::<C>(key.idx()))
            .value()
    }

    /// [`get_mut`](Self::get_mut) without the bounds and generation checks.
    ///
    /// # Safety
    ///
    /// `key` must be valid, meaning [`contains_key`](Self::contains_key)
    /// returns `true` for it.
    #[inline]
    pub unsafe fn get_unchecked_mut(&mut self, key: Key<C>) -> &mut T {
        debug_assert!(self.contains_key(key));
        self.slots
            .as_mut_slice()
            .get_unchecked_mut(position_of::<C>(key.idx()))
            .value_mut()
    }

    /// Returns the key of the value in the slot at `idx`, or `None` if there
    /// is no slot at `idx` or the slot holds no value. A slot holds no value
    /// while it is vacant, detached or retired.
    #[inline]
    pub fn key_at(&self, idx: C::Idx) -> Option<Key<C>> {
        let slot = self.slots.as_slice().get(idx.into_usize()?)?;
        if !slot.is_occupied() {
            return None;
        }
        // SAFETY: the slot is occupied and `idx` is its position.
        Some(unsafe { slot.key(idx) })
    }

    /// [`key_at`](Self::key_at) without the bounds and occupancy checks.
    ///
    /// # Safety
    ///
    /// There must be a slot at `idx` and it must hold a value, meaning
    /// [`key_at`](Self::key_at) returns `Some` for it. A vacant or detached
    /// slot has an even generation, so the key would carry that even
    /// generation, and any lookup with such a key, checked or not, is
    /// undefined behavior, because the checked lookups rely on every key
    /// being odd. A retired slot has a generation of zero, and building a
    /// key from it is undefined behavior on its own, because the key needs
    /// the generation as a `NonZero`.
    #[inline]
    pub unsafe fn key_at_unchecked(&self, idx: C::Idx) -> Key<C> {
        debug_assert!(self.key_at(idx).is_some());
        self.slots
            .as_slice()
            .get_unchecked(position_of::<C>(idx))
            .key(idx)
    }

    /// Returns the key and a reference to the value in the slot at `idx`, or
    /// `None` if there is no slot at `idx` or the slot holds no value. A slot
    /// holds no value while it is vacant, detached or retired.
    #[inline]
    pub fn get_at(&self, idx: C::Idx) -> Option<(Key<C>, &T)> {
        let slot = self.slots.as_slice().get(idx.into_usize()?)?;
        if !slot.is_occupied() {
            return None;
        }
        // SAFETY: the slot is occupied and `idx` is its position.
        Some(unsafe { (slot.key(idx), slot.value()) })
    }

    /// Returns the key and a mutable reference to the value in the slot at
    /// `idx`, or `None` if there is no slot at `idx` or the slot holds no
    /// value. See [`get_at`](Self::get_at).
    #[inline]
    pub fn get_at_mut(&mut self, idx: C::Idx) -> Option<(Key<C>, &mut T)> {
        let slot = self.slots.as_mut_slice().get_mut(idx.into_usize()?)?;
        if !slot.is_occupied() {
            return None;
        }
        // SAFETY: the slot is occupied and `idx` is its position.
        let key = unsafe { slot.key(idx) };
        // SAFETY: the slot is occupied.
        Some((key, unsafe { slot.value_mut() }))
    }

    /// [`get_at`](Self::get_at) without the bounds and occupancy checks.
    ///
    /// # Safety
    ///
    /// There must be a slot at `idx` and it must hold a value, meaning
    /// [`key_at`](Self::key_at) returns `Some` for it. Otherwise the slot is
    /// read as if it held a value, which is undefined behavior, and the key
    /// is built from an even or zero generation, so using it in any later
    /// lookup is undefined behavior as well, as described on
    /// [`key_at_unchecked`](Self::key_at_unchecked).
    #[inline]
    pub unsafe fn get_at_unchecked(&self, idx: C::Idx) -> (Key<C>, &T) {
        debug_assert!(self.key_at(idx).is_some());
        let slot = self.slots.as_slice().get_unchecked(position_of::<C>(idx));
        (slot.key(idx), slot.value())
    }

    /// [`get_at_mut`](Self::get_at_mut) without the bounds and occupancy
    /// checks.
    ///
    /// # Safety
    ///
    /// The same as for [`get_at_unchecked`](Self::get_at_unchecked).
    #[inline]
    pub unsafe fn get_at_unchecked_mut(&mut self, idx: C::Idx) -> (Key<C>, &mut T) {
        debug_assert!(self.key_at(idx).is_some());
        let slot = self
            .slots
            .as_mut_slice()
            .get_unchecked_mut(position_of::<C>(idx));
        let key = slot.key(idx);
        (key, slot.value_mut())
    }

    /// Returns the generation of the slot at `idx`, or `None` if there is no
    /// slot at `idx`.
    ///
    /// Every slot has a generation, whether it holds a value or not, so this
    /// works for vacant, detached and retired slots too. The generation is
    /// odd while the slot holds a value and even otherwise.
    #[inline]
    pub fn generation_at(&self, idx: C::Idx) -> Option<C::Gen> {
        Some(self.slots.as_slice().get(idx.into_usize()?)?.generation)
    }

    /// [`generation_at`](Self::generation_at) without the bounds check.
    ///
    /// # Safety
    ///
    /// There must be a slot at `idx`, meaning
    /// [`generation_at`](Self::generation_at) returns `Some` for it. The
    /// slot does not have to hold a value.
    #[inline]
    pub unsafe fn generation_at_unchecked(&self, idx: C::Idx) -> C::Gen {
        debug_assert!(self.generation_at(idx).is_some());
        self.slots
            .as_slice()
            .get_unchecked(position_of::<C>(idx))
            .generation
    }

    /// Returns the key and a mutable reference to the value in the slot at
    /// each index, all at once. This is [`get_disjoint_mut`] for indices
    /// instead of keys, and like [`get_at`](Self::get_at) it hands each key
    /// back with its value.
    ///
    /// Every index gets the same checks as [`get_at_mut`](Self::get_at_mut),
    /// and every pair of indices is checked to be different, so the
    /// references can not alias.
    ///
    /// # Errors
    ///
    /// Returns [`GetDisjointMutAtError::NoValue`] if there is no slot at one
    /// of the indices or the slot holds no value, and
    /// [`GetDisjointMutAtError::OverlappingIndices`] if two indices are the
    /// same. Nothing is borrowed on error.
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
    pub fn get_disjoint_mut_at<const N: usize>(
        &mut self,
        idxs: [C::Idx; N],
    ) -> Result<[(Key<C>, &mut T); N], GetDisjointMutAtError> {
        for (i, idx) in idxs.iter().enumerate() {
            if self.key_at(*idx).is_none() {
                return Err(GetDisjointMutAtError::NoValue);
            }
            if idxs[..i].contains(idx) {
                return Err(GetDisjointMutAtError::OverlappingIndices);
            }
        }
        // SAFETY: every index was just found to name an occupied slot, and
        // every index is distinct.
        Ok(unsafe { self.get_disjoint_mut_at_unchecked(idxs) })
    }

    /// [`get_disjoint_mut_at`](Self::get_disjoint_mut_at) without any of the
    /// checks.
    ///
    /// # Safety
    ///
    /// There must be a slot at every index and each must hold a value,
    /// meaning [`key_at`](Self::key_at) returns `Some` for every one of them,
    /// and no two indices of the keys provided may be the same.
    #[inline]
    pub unsafe fn get_disjoint_mut_at_unchecked<const N: usize>(
        &mut self,
        idxs: [C::Idx; N],
    ) -> [(Key<C>, &mut T); N] {
        debug_assert!(idxs.iter().all(|idx| self.key_at(*idx).is_some()));
        debug_assert!(idxs
            .iter()
            .enumerate()
            .all(|(i, idx)| !idxs[..i].contains(idx)));
        let slots = self.slots.as_mut_slice().as_mut_ptr();
        idxs.map(|idx| {
            // SAFETY: the caller promises the slot exists and is occupied,
            // so its position is in bounds, its generation is odd and
            // `occupied` is the live field, and that no other index names
            // the same slot, so the references do not alias. The generation
            // is read through the raw pointer so that no reference to the
            // slot exists beside the one handed out.
            unsafe {
                let slot = slots.add(position_of::<C>(idx));
                let key =
                    Key::<C>::from_raw_parts(idx, (*slot).generation.into_non_zero_unchecked());
                (key, &mut *(*slot).data.occupied)
            }
        })
    }

    /// Returns a mutable reference to the value of each key, all at once.
    ///
    /// Every key gets the same checks as it would had you put it in [`get_mut`](Self::get_mut), and
    /// every pair of keys is checked to point at different slots, so the
    /// references can not alias.
    ///
    /// # Errors
    ///
    /// Returns [`GetDisjointMutError::InvalidKey`] if a key is invalid and
    /// [`GetDisjointMutError::OverlappingKeys`] if two keys point at the same
    /// slot. Nothing is borrowed on error.
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
    pub fn get_disjoint_mut<const N: usize>(
        &mut self,
        keys: [Key<C>; N],
    ) -> Result<[&mut T; N], GetDisjointMutError> {
        for (i, key) in keys.iter().enumerate() {
            if !self.contains_key(*key) {
                return Err(GetDisjointMutError::InvalidKey);
            }
            // Two valid keys for one slot carry the slot's generation, so the
            // index alone says whether they overlap.
            if keys[..i].iter().any(|earlier| earlier.idx() == key.idx()) {
                return Err(GetDisjointMutError::OverlappingKeys);
            }
        }
        // SAFETY: every key was just found valid and every index distinct.
        Ok(unsafe { self.get_disjoint_mut_unchecked(keys) })
    }

    /// [`get_disjoint_mut`](Self::get_disjoint_mut) without any of the
    /// checks.
    ///
    /// # Safety
    ///
    /// Every key must be valid, meaning [`contains_key`](Self::contains_key)
    /// returns `true` for it, and no two keys may point at the same slot.
    /// Otherwise a slot is read as if it held a value, or two of the
    /// references point at the same value, either of which is undefined
    /// behavior.
    #[inline]
    pub unsafe fn get_disjoint_mut_unchecked<const N: usize>(
        &mut self,
        keys: [Key<C>; N],
    ) -> [&mut T; N] {
        debug_assert!(keys.iter().all(|key| self.contains_key(*key)));
        debug_assert!(keys
            .iter()
            .enumerate()
            .all(|(i, key)| keys[..i].iter().all(|earlier| earlier.idx() != key.idx())));
        let slots = self.slots.as_mut_slice().as_mut_ptr();
        keys.map(|key| {
            // SAFETY: the caller promises the key is valid, so its position
            // is in bounds and `occupied` is the live field, and that no
            // other key names the same slot, so the references do not alias.
            unsafe { &mut *(*slots.add(position_of::<C>(key.idx()))).data.occupied }
        })
    }

    /// Inserts a value and returns its key.
    ///
    /// # Panics
    ///
    /// Panics if the map is full, meaning none of the slots are free and
    /// either its keys have no index left for a new slot or the storage can
    /// not make room for one. Use [`try_insert`](Self::try_insert) to get
    /// the value back instead.
    #[inline]
    pub fn insert(&mut self, value: T) -> Key<C> {
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
    /// can not make room for another one.
    ///
    /// # Examples
    ///
    /// ```
    /// use gen_map::{Config, GenMap, InsertError, Split};
    ///
    /// struct Tiny;
    ///
    /// impl Config for Tiny {
    ///     type Idx = u8;
    ///     type Gen = u8;
    ///     type Layout = Split;
    ///     type Storage<S> = Vec<S>;
    /// }
    ///
    /// let mut map = GenMap::<u32, Tiny>::new_with_config();
    /// for i in 0..256 {
    ///     map.insert(i);
    /// }
    /// assert!(matches!(map.try_insert(256), Err(InsertError::IndexExhausted(256))));
    /// ```
    #[inline]
    pub fn try_insert(&mut self, value: T) -> Result<Key<C>, InsertError<T, StorageError<T, C>>> {
        match self.vacant_entry() {
            Ok(entry) => Ok(entry.insert(value)),
            Err(FullError::IndexExhausted) => Err(InsertError::IndexExhausted(value)),
            Err(FullError::StorageFull(error)) => Err(InsertError::StorageFull(value, error)),
        }
    }

    /// Inserts the value returned by `f`, which is given the value's key.
    ///
    /// # Panics
    ///
    /// Panics if the map is full, meaning none of the slots are free and
    /// either its keys have no index left for a new slot or the storage can
    /// not make room for one.
    /// Use [`try_insert_with_key`](Self::try_insert_with_key) or
    /// [`vacant_entry`](Self::vacant_entry) to get an error instead.
    #[inline]
    pub fn insert_with_key<F>(&mut self, f: F) -> Key<C>
    where
        F: FnOnce(Key<C>) -> T,
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
    /// a full map is an error rather than a panic. On either error nothing
    /// is inserted, and the key `f` was given stays invalid until a later
    /// insert hands it out again.
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
    pub fn try_insert_with_key<F, E>(
        &mut self,
        f: F,
    ) -> Result<Key<C>, InsertWithError<E, StorageError<T, C>>>
    where
        F: FnOnce(Key<C>) -> Result<T, E>,
    {
        let entry = self.vacant_entry()?;
        let value = f(entry.key()).map_err(InsertWithError::Rejected)?;
        Ok(entry.insert(value))
    }

    /// Hands out the slot the next insert would use, without writing to it.
    /// [`VacantEntry::key`] is the key the value will get and
    /// [`VacantEntry::insert`] puts it there. Dropping the entry leaves the
    /// map as it was.
    ///
    /// # Errors
    ///
    /// Returns [`FullError::IndexExhausted`] if none of the slots are free
    /// and the map's keys have no index left for a new one, and
    /// [`FullError::StorageFull`] if none of them are free and the storage
    /// can not make room for another one. When a `Vec` can not allocate,
    /// this returns `StorageFull` instead of panicking.
    #[inline]
    pub fn vacant_entry(&mut self) -> Result<VacantEntry<'_, T, C>, FullError<StorageError<T, C>>> {
        let target = self.next_target()?;
        Ok(VacantEntry { map: self, target })
    }

    /// Works out where the next value goes without writing any slot. When a
    /// slot has to be pushed, this makes sure there is room for it, so that
    /// the push in [`fill`](Self::fill) can not fail. The generation in the
    /// result is odd.
    ///
    /// When the keys' index and the storage both run out, the error is
    /// `IndexExhausted`.
    #[inline]
    fn next_target(&mut self) -> Result<Target<C>, FullError<StorageError<T, C>>> {
        let (idx, position, generation, from_free_list) = match self.next_free {
            Some(idx) => {
                // SAFETY: `idx` is on the free list, so it was the position
                // of a slot that still exists.
                let position = unsafe { position_of::<C>(idx) };
                let generation =
                    unsafe { self.slots.as_slice().get_unchecked(position).generation };
                (idx, position, generation, true)
            }
            None => {
                let position = self.slots.len();
                let idx = C::Idx::from_usize(position)
                    .filter(|idx| *idx <= max_idx::<C>())
                    .ok_or(FullError::IndexExhausted)?;
                self.slots.ensure_room().map_err(FullError::StorageFull)?;
                (idx, position, C::Gen::ZERO, false)
            }
        };

        // A vacant slot's generation is even and the largest generation a
        // key can hold is odd, so the next one fits and the wrapping add
        // never wraps.
        debug_assert!(!generation.is_odd());
        let generation = generation.wrapping_add(C::Gen::ONE);

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
    unsafe fn fill(&mut self, target: Target<C>, value: T) -> Key<C> {
        let key = target.key();
        if target.from_free_list {
            // SAFETY: `position` was in bounds when `next_target` looked, and
            // nothing has changed since.
            let slot = unsafe { self.slots.as_mut_slice().get_unchecked_mut(target.position) };
            // SAFETY: the slot came off the free list, so it is vacant and
            // its `vacant` field is the live one.
            self.next_free = unsafe { slot.data.vacant };
            slot.data.occupied = ManuallyDrop::new(value);
            slot.generation = target.generation;
        } else {
            let slot = Slot {
                generation: target.generation,
                data: SlotData {
                    occupied: ManuallyDrop::new(value),
                },
            };
            // `next_target` made room for this slot, and `SlotStorage`
            // promises `try_push` succeeds then, so a refusal is a broken
            // storage. The slot is dropped with its value in that case.
            if self.slots.try_push(slot).is_err() {
                panic!("SlotStorage::try_push failed although ensure_room returned Ok");
            }
        }
        self.len += 1;
        key
    }

    /// Removes and returns the value corresponding to `key`, or `None` if the
    /// key is invalid.
    #[inline]
    pub fn remove(&mut self, key: Key<C>) -> Option<T> {
        let position = key.idx().into_usize()?;
        let slot = self.slots.as_slice().get(position)?;
        if slot.generation != key.generation() {
            return None;
        }
        // SAFETY: a key's generation is always odd, so a matching slot is
        // occupied.
        Some(unsafe { self.take(key.idx(), position) })
    }

    /// Moves the value out of the slot at `position`. The slot then goes on
    /// the free list, unless its generation has run out and the config
    /// retires such slots.
    ///
    /// # Safety
    ///
    /// The slot at `position` must be occupied, and `idx` must be `position`
    /// as a `C::Idx`.
    unsafe fn take(&mut self, idx: C::Idx, position: usize) -> T {
        // SAFETY: the caller promises an occupied slot at `position`, so it
        // is in bounds and `occupied` is the live field.
        let (slot, value) = unsafe {
            let slot = self.slots.as_mut_slice().get_unchecked_mut(position);
            debug_assert!(slot.is_occupied());
            let value = ManuallyDrop::take(&mut slot.data.occupied);
            (slot, value)
        };
        self.len -= 1;

        match next_generation::<C>(slot.generation) {
            Some(next) => {
                slot.generation = next;
                slot.data.vacant = self.next_free;
                self.next_free = Some(idx);
            }
            None => {
                slot.generation = C::Gen::ZERO;
                if C::WRAP_ON_OVERFLOW {
                    slot.data.vacant = self.next_free;
                    self.next_free = Some(idx);
                } else {
                    // The slot is retired, so it stays off the free list for
                    // good.
                    slot.data.vacant = None;
                }
            }
        }
        value
    }

    /// Takes the value out of the map while keeping its slot reserved for
    /// `key`, so that [`reattach`](Self::reattach) can put a value back under
    /// the same key.
    ///
    /// Until then the key is invalid, meaning
    /// [`contains_key`](Self::contains_key) returns `false` and the value is
    /// not counted by [`len`](Self::len), but the slot is not on the free
    /// list, so no insert call uses it.
    ///
    /// Returns `None` if the key is invalid, and also if adding one to the
    /// slot's generation would overflow, because a slot that is about to
    /// retire or wrap can not promise to give the same key back. The value
    /// stays in the map in that case.
    ///
    /// [`clear`](Self::clear) and [`retain`](Self::retain) leave a detached
    /// slot as it is, since it holds no value. [`reset`](Self::reset) removes
    /// every slot, detached ones included, and a later `reattach` then
    /// panics.
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
    /// map.reattach(key, value + 10);
    /// assert_eq!(map[key], 11);
    /// ```
    #[inline]
    pub fn detach(&mut self, key: Key<C>) -> Option<T> {
        let slot = self.slots.as_mut_slice().get_mut(key.idx().into_usize()?)?;
        if slot.generation != key.generation() {
            return None;
        }
        let next = next_generation::<C>(slot.generation)?;
        // SAFETY: a key's generation is always odd, so the matching slot is
        // occupied and `occupied` is the live field.
        let value = unsafe { ManuallyDrop::take(&mut slot.data.occupied) };
        slot.generation = next;
        slot.data.vacant = Some(key.idx());
        self.len -= 1;
        Some(value)
    }

    /// Puts a value back under a key whose value [`detach`](Self::detach)
    /// took out. The key is valid again afterwards.
    ///
    /// # Panics
    ///
    /// Panics if the key was not detached, or was detached and its slot has
    /// since been removed by [`reset`](Self::reset).
    #[inline]
    pub fn reattach(&mut self, key: Key<C>, value: T) {
        let generation = key.generation();
        let slot = key
            .idx()
            .into_usize()
            .and_then(|position| self.slots.as_mut_slice().get_mut(position))
            .filter(|slot| {
                // Detaching added one to the generation, so a slot still
                // detached under this key has a generation one above the
                // key's.
                next_generation::<C>(generation) == Some(slot.generation)
                    && slot.is_detached(key.idx())
            })
            .expect("reattach on a key that is not detached");
        slot.data.occupied = ManuallyDrop::new(value);
        slot.generation = generation;
        self.len += 1;
    }

    /// Removes every value. Slots are kept and every old key stays invalid.
    pub fn clear(&mut self) {
        for position in 0..self.slots.len() {
            // SAFETY: `position` is below the slot count, which `take` does
            // not change.
            let occupied = unsafe { self.slots.as_slice().get_unchecked(position).is_occupied() };
            if occupied {
                // SAFETY: the slot was just checked to be occupied, and
                // `position` is its position.
                drop(unsafe { self.take(index_of::<C>(position), position) });
            }
        }
        debug_assert_eq!(self.len, 0);
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
        self.next_free = None;
        self.len = 0;
        self.slots.clear();
    }

    /// Keeps only the values for which `f` returns `true`. `f` may mutate
    /// them.
    pub fn retain<F>(&mut self, mut f: F)
    where
        F: FnMut(Key<C>, &mut T) -> bool,
    {
        for position in 0..self.slots.len() {
            // SAFETY: `position` is below the slot count, which neither
            // `take` nor `f` (which never sees the map) changes.
            let slot = unsafe { self.slots.as_mut_slice().get_unchecked_mut(position) };
            if !slot.is_occupied() {
                continue;
            }
            // SAFETY: the slot is occupied and `position` is its position.
            let (idx, keep) = unsafe {
                let idx = index_of::<C>(position);
                (idx, f(slot.key(idx), slot.value_mut()))
            };
            if !keep {
                // SAFETY: the slot is still occupied. `f` only had `&mut T`
                // and could not have removed it.
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
            remaining: self.len,
        }
    }

    /// Iterates over every value mutably together with its key, in slot order.
    #[inline]
    pub fn iter_mut(&mut self) -> IterMut<'_, T, C> {
        IterMut {
            slots: self.slots.as_mut_slice().iter_mut().enumerate(),
            remaining: self.len,
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

impl<T, C: Config> Default for GenMap<T, C> {
    #[inline]
    fn default() -> Self {
        Self::new_with_config()
    }
}

impl<T, C: Config> Index<Key<C>> for GenMap<T, C> {
    type Output = T;

    #[inline]
    fn index(&self, key: Key<C>) -> &T {
        self.get(key).expect("invalid GenMap key")
    }
}

impl<T, C: Config> IndexMut<Key<C>> for GenMap<T, C> {
    #[inline]
    fn index_mut(&mut self, key: Key<C>) -> &mut T {
        self.get_mut(key).expect("invalid GenMap key")
    }
}

impl<T: fmt::Debug, C: Config> fmt::Debug for GenMap<T, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

/// Pushes `slot` onto a storage that is being filled with clones of another
/// storage of the same type. The storage being cloned already holds every
/// slot pushed here, so a refusal breaks the [`SlotStorage`] contract, and
/// this panics.
/// The slot is dropped with its value in that case.
fn push_cloned<T, C: Config>(slots: &mut Slots<T, C>, slot: Slot<T, C>) {
    if slots.try_push(slot).is_err() {
        panic!("SlotStorage::try_push failed while cloning a storage of the same type");
    }
}

/// Empties the slot storage on drop. Only dropped while unwinding out of
/// `clone_from`, where a half cloned storage would disagree with `len` and
/// the free list.
struct ClearOnUnwind<'a, T, C: Config>(&'a mut Slots<T, C>);

impl<T, C: Config> Drop for ClearOnUnwind<'_, T, C> {
    fn drop(&mut self) {
        SlotStorage::clear(self.0);
    }
}

impl<T: Clone, C: Config> Clone for GenMap<T, C> {
    /// The clone has the same slots, free list and generations, so every key
    /// of the original works on it.
    fn clone(&self) -> Self {
        let mut slots = Slots::<T, C>::with_capacity(self.slots.len());
        for slot in self.slots.as_slice() {
            push_cloned(&mut slots, slot.clone_slot());
        }
        Self {
            slots,
            next_free: self.next_free,
            len: self.len,
        }
    }

    /// Reuses this map's allocation. If a value's `clone` panics, this map is
    /// left empty.
    fn clone_from(&mut self, source: &Self) {
        self.next_free = None;
        self.len = 0;
        let guard: ClearOnUnwind<'_, T, C> = ClearOnUnwind(&mut self.slots);
        SlotStorage::clear(guard.0);
        for slot in source.slots.as_slice() {
            push_cloned(guard.0, slot.clone_slot());
        }
        core::mem::forget(guard);
        self.next_free = source.next_free;
        self.len = source.len;
    }
}

/// Iterator over `(key, &value)` pairs. Created by [`GenMap::iter`].
pub struct Iter<'a, T, C: Config> {
    slots: Enumerate<core::slice::Iter<'a, Slot<T, C>>>,
    remaining: usize,
}

impl<'a, T, C: Config> Iterator for Iter<'a, T, C> {
    type Item = (Key<C>, &'a T);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        for (position, slot) in self.slots.by_ref() {
            if slot.is_occupied() {
                self.remaining -= 1;
                // SAFETY: the slot is occupied and `position` is where it
                // sits in the backing storage.
                return unsafe { Some((slot.key(index_of::<C>(position)), slot.value())) };
            }
        }
        None
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<T, C: Config> DoubleEndedIterator for Iter<'_, T, C> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        while let Some((position, slot)) = self.slots.next_back() {
            if slot.is_occupied() {
                self.remaining -= 1;
                // SAFETY: the slot is occupied and `position` is where it
                // sits in the backing storage.
                return unsafe { Some((slot.key(index_of::<C>(position)), slot.value())) };
            }
        }
        None
    }
}

impl<T, C: Config> ExactSizeIterator for Iter<'_, T, C> {}
impl<T, C: Config> FusedIterator for Iter<'_, T, C> {}

impl<T, C: Config> Clone for Iter<'_, T, C> {
    fn clone(&self) -> Self {
        Self {
            slots: self.slots.clone(),
            remaining: self.remaining,
        }
    }
}

/// Iterator over `(key, &mut value)` pairs. Created by [`GenMap::iter_mut`].
pub struct IterMut<'a, T, C: Config> {
    slots: Enumerate<core::slice::IterMut<'a, Slot<T, C>>>,
    remaining: usize,
}

impl<'a, T, C: Config> Iterator for IterMut<'a, T, C> {
    type Item = (Key<C>, &'a mut T);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        for (position, slot) in self.slots.by_ref() {
            if slot.is_occupied() {
                self.remaining -= 1;
                // SAFETY: the slot is occupied and `position` is where it
                // sits in the backing storage.
                return unsafe { Some((slot.key(index_of::<C>(position)), slot.value_mut())) };
            }
        }
        None
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<T, C: Config> DoubleEndedIterator for IterMut<'_, T, C> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        while let Some((position, slot)) = self.slots.next_back() {
            if slot.is_occupied() {
                self.remaining -= 1;
                // SAFETY: the slot is occupied and `position` is where it
                // sits in the backing storage.
                return unsafe { Some((slot.key(index_of::<C>(position)), slot.value_mut())) };
            }
        }
        None
    }
}

impl<T, C: Config> ExactSizeIterator for IterMut<'_, T, C> {}
impl<T, C: Config> FusedIterator for IterMut<'_, T, C> {}

/// Iterator over keys. Created by [`GenMap::keys`].
pub struct Keys<'a, T, C: Config> {
    inner: Iter<'a, T, C>,
}

impl<T, C: Config> Iterator for Keys<'_, T, C> {
    type Item = Key<C>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(key, _)| key)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<T, C: Config> DoubleEndedIterator for Keys<'_, T, C> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(|(key, _)| key)
    }
}

impl<T, C: Config> ExactSizeIterator for Keys<'_, T, C> {}
impl<T, C: Config> FusedIterator for Keys<'_, T, C> {}

impl<T, C: Config> Clone for Keys<'_, T, C> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

/// Iterator over shared references to values. Created by [`GenMap::values`].
pub struct Values<'a, T, C: Config> {
    inner: Iter<'a, T, C>,
}

impl<'a, T, C: Config> Iterator for Values<'a, T, C> {
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

impl<T, C: Config> DoubleEndedIterator for Values<'_, T, C> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(|(_, value)| value)
    }
}

impl<T, C: Config> ExactSizeIterator for Values<'_, T, C> {}
impl<T, C: Config> FusedIterator for Values<'_, T, C> {}

impl<T, C: Config> Clone for Values<'_, T, C> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

/// Iterator over mutable references to values. Created by
/// [`GenMap::values_mut`].
pub struct ValuesMut<'a, T, C: Config> {
    inner: IterMut<'a, T, C>,
}

impl<'a, T, C: Config> Iterator for ValuesMut<'a, T, C> {
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

impl<T, C: Config> DoubleEndedIterator for ValuesMut<'_, T, C> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(|(_, value)| value)
    }
}

impl<T, C: Config> ExactSizeIterator for ValuesMut<'_, T, C> {}
impl<T, C: Config> FusedIterator for ValuesMut<'_, T, C> {}

/// Owning iterator over `(key, value)` pairs. Created by consuming a map with
/// `into_iter`.
pub struct IntoIter<T, C: Config> {
    slots: Enumerate<<Slots<T, C> as IntoIterator>::IntoIter>,
    remaining: usize,
}

impl<T, C: Config> Iterator for IntoIter<T, C> {
    type Item = (Key<C>, T);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        for (position, mut slot) in self.slots.by_ref() {
            if slot.is_occupied() {
                self.remaining -= 1;
                // SAFETY: the slot is occupied and `position` is where it sat
                // in the storage. Zeroing the generation afterwards stops the
                // slot's `Drop` from dropping the value again.
                unsafe {
                    let key = slot.key(index_of::<C>(position));
                    let value = ManuallyDrop::take(&mut slot.data.occupied);
                    slot.generation = C::Gen::ZERO;
                    return Some((key, value));
                }
            }
        }
        None
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<T, C: Config> DoubleEndedIterator for IntoIter<T, C> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        while let Some((position, mut slot)) = self.slots.next_back() {
            if slot.is_occupied() {
                self.remaining -= 1;
                // SAFETY: same reasoning as in `next`.
                unsafe {
                    let key = slot.key(index_of::<C>(position));
                    let value = ManuallyDrop::take(&mut slot.data.occupied);
                    slot.generation = C::Gen::ZERO;
                    return Some((key, value));
                }
            }
        }
        None
    }
}

impl<T, C: Config> ExactSizeIterator for IntoIter<T, C> {}
impl<T, C: Config> FusedIterator for IntoIter<T, C> {}

impl<T, C: Config> IntoIterator for GenMap<T, C> {
    type Item = (Key<C>, T);
    type IntoIter = IntoIter<T, C>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        IntoIter {
            remaining: self.len,
            slots: self.slots.into_iter().enumerate(),
        }
    }
}

impl<'a, T, C: Config> IntoIterator for &'a GenMap<T, C> {
    type Item = (Key<C>, &'a T);
    type IntoIter = Iter<'a, T, C>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T, C: Config> IntoIterator for &'a mut GenMap<T, C> {
    type Item = (Key<C>, &'a mut T);
    type IntoIter = IterMut<'a, T, C>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

/// Draining iterator over `(key, value)` pairs. Created by [`GenMap::drain`].
/// Dropping it removes whatever has not been yielded yet.
pub struct Drain<'a, T, C: Config> {
    map: &'a mut GenMap<T, C>,
    position: usize,
}

impl<T, C: Config> Iterator for Drain<'_, T, C> {
    type Item = (Key<C>, T);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        while self.position < self.map.slots.len() {
            let position = self.position;
            self.position += 1;
            // SAFETY: `position` is below the slot count, which `take` does
            // not change.
            let slot = unsafe { self.map.slots.as_slice().get_unchecked(position) };
            if slot.is_occupied() {
                // SAFETY: the slot is occupied and `position` is its position.
                unsafe {
                    let idx = index_of::<C>(position);
                    let key = slot.key(idx);
                    return Some((key, self.map.take(idx, position)));
                }
            }
        }
        None
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.map.len, Some(self.map.len))
    }
}

impl<T, C: Config> ExactSizeIterator for Drain<'_, T, C> {}
impl<T, C: Config> FusedIterator for Drain<'_, T, C> {}

impl<T, C: Config> Drop for Drain<'_, T, C> {
    fn drop(&mut self) {
        for _ in self.by_ref() {}
    }
}
