use crate::config::{Config, DefaultConfig};
use crate::key::KeyOf;
use crate::key_piece::KeyPiece;
use alloc::collections::TryReserveError;
use alloc::vec::Vec;
use core::any::type_name;
use core::fmt;
use core::iter::{Enumerate, FusedIterator};
use core::mem::ManuallyDrop;
use core::ops::{Index, IndexMut};

/// The payload of a slot. `occupied` is live while the slot holds a value and
/// `vacant` while it sits on the free list. The owning [`Slot`] tells the two
/// apart by its generation.
union SlotData<T, Idx: Copy> {
    occupied: ManuallyDrop<T>,
    vacant: Option<Idx>,
}

/// One entry of the backing `Vec`. An even generation means vacant and an odd
/// one means occupied. Insert and remove each add one, so every key carries an
/// odd generation.
struct Slot<T, C: Config> {
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
    /// The slot must be occupied and `idx` must be its position in the `Vec`.
    #[inline]
    unsafe fn key(&self, idx: C::Idx) -> KeyOf<C> {
        KeyOf::<C>::from_parts_unchecked(idx, self.generation)
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

/// Converts a position in the backing `Vec` to `C::Idx`.
///
/// # Safety
///
/// `position` must be the position of a slot in the map's `Vec`. Every such
/// position fits, because the map refuses to push a slot whose position does
/// not.
#[inline]
unsafe fn index_of<C: Config>(position: usize) -> C::Idx {
    let idx = C::Idx::from_usize(position);
    debug_assert!(idx.is_some(), "every slot position fits in the configured index type");
    unsafe { idx.unwrap_unchecked() }
}

/// Converts a free list index back to its position in the backing `Vec`.
///
/// # Safety
///
/// `idx` must be on the free list. Every free list index was a slot's
/// position, so it fits in `usize`.
#[inline]
unsafe fn position_of<C: Config>(idx: C::Idx) -> usize {
    let position = idx.into_usize();
    debug_assert!(position.is_some(), "every free list index fits in usize");
    unsafe { position.unwrap_unchecked() }
}

/// A generational map whose key is configured by `C`.
///
/// See the [crate documentation](crate) for examples.
pub struct GenMap<T, C: Config = DefaultConfig> {
    slots: Vec<Slot<T, C>>,
    next_free: Option<C::Idx>,
    len: usize,
}

impl<T> GenMap<T> {
    /// Creates an empty map with the [`DefaultConfig`].
    ///
    /// Use [`new_with_config`](Self::new_with_config) for any other config,
    /// since the default type parameter does not take part in inference.
    #[inline]
    pub const fn new() -> Self {
        Self::new_with_config()
    }

    /// Creates an empty map with the [`DefaultConfig`] and room for `capacity`
    /// slots.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self::with_capacity_and_config(capacity)
    }
}

impl<T, C: Config> GenMap<T, C> {
    /// Creates an empty map with config `C`.
    ///
    /// ```
    /// use gen_map::{Config, GenMap};
    ///
    /// struct Wide;
    ///
    /// impl Config for Wide {
    ///     type Idx = u64;
    ///     type Gen = u64;
    /// }
    ///
    /// let mut map = GenMap::<&str, Wide>::new_with_config();
    /// let key = map.insert("x");
    /// assert_eq!(core::mem::size_of_val(&key), 16);
    /// ```
    #[inline]
    pub const fn new_with_config() -> Self {
        Self {
            slots: Vec::new(),
            next_free: None,
            len: 0,
        }
    }

    /// Creates an empty map with config `C` and room for `capacity` slots.
    #[inline]
    pub fn with_capacity_and_config(capacity: usize) -> Self {
        Self {
            slots: Vec::with_capacity(capacity),
            next_free: None,
            len: 0,
        }
    }

    /// Reserves room for at least `additional` more slots.
    #[inline]
    pub fn reserve(&mut self, additional: usize) {
        self.slots.reserve(additional);
    }

    /// The fallible form of [`reserve`](Self::reserve).
    #[inline]
    pub fn try_reserve(&mut self, additional: usize) -> Result<(), TryReserveError> {
        self.slots.try_reserve(additional)
    }

    /// How many slots the backing `Vec` can hold before it reallocates.
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

    /// The number of slots, occupied and vacant together.
    #[inline]
    pub fn slots_len(&self) -> usize {
        self.slots.len()
    }

    /// Returns `true` if `key` is valid.
    #[inline]
    pub fn contains_key(&self, key: KeyOf<C>) -> bool {
        self.get(key).is_some()
    }

    /// Returns a reference to the value corresponding to `key`.
    #[inline]
    pub fn get(&self, key: KeyOf<C>) -> Option<&T> {
        let slot = self.slots.get(key.index().into_usize()?)?;
        if slot.generation != key.generation() {
            return None;
        }
        // SAFETY: a key's generation is always odd, so a matching slot is
        // occupied.
        Some(unsafe { slot.value() })
    }

    /// Returns a mutable reference to the value corresponding to `key`.
    #[inline]
    pub fn get_mut(&mut self, key: KeyOf<C>) -> Option<&mut T> {
        let slot = self.slots.get_mut(key.index().into_usize()?)?;
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
    pub unsafe fn get_unchecked(&self, key: KeyOf<C>) -> &T {
        debug_assert!(self.contains_key(key));
        self.slots
            .get_unchecked(key.index().into_usize().unwrap_unchecked())
            .value()
    }

    /// [`get_mut`](Self::get_mut) without the bounds and generation checks.
    ///
    /// # Safety
    ///
    /// `key` must be valid, meaning [`contains_key`](Self::contains_key)
    /// returns `true` for it.
    #[inline]
    pub unsafe fn get_unchecked_mut(&mut self, key: KeyOf<C>) -> &mut T {
        debug_assert!(self.contains_key(key));
        self.slots
            .get_unchecked_mut(key.index().into_usize().unwrap_unchecked())
            .value_mut()
    }

    /// Inserts a value and returns its key.
    ///
    /// # Panics
    ///
    /// Panics if the map is full, meaning it has `C::Idx::MAX + 1` slots and
    /// none of them are free.
    #[inline]
    pub fn insert(&mut self, value: T) -> KeyOf<C> {
        self.insert_with_key(|_| value)
    }

    /// Inserts the value returned by `f`, which is given the value's key.
    ///
    /// # Panics
    ///
    /// Panics if the map is full, meaning it has `C::Idx::MAX + 1` slots and
    /// none of them are free.
    #[inline]
    pub fn insert_with_key<F>(&mut self, f: F) -> KeyOf<C>
    where
        F: FnOnce(KeyOf<C>) -> T,
    {
        unsafe{ self.try_insert_with_key(|key| Ok::<T, core::convert::Infallible>(f(key))).unwrap_unchecked() }
    }

    /// Like [`insert_with_key`](Self::insert_with_key), but `f` may fail. On
    /// `Err` nothing is inserted.
    ///
    /// # Panics
    ///
    /// Panics if the map is full, meaning it has `C::Idx::MAX + 1` slots and
    /// none of them are free.
    pub fn try_insert_with_key<F, E>(&mut self, f: F) -> Result<KeyOf<C>, E>
    where
        F: FnOnce(KeyOf<C>) -> Result<T, E>,
    {
        // Nothing is written until `f` has succeeded, so a failing or
        // panicking `f` leaves the map untouched.
        let (idx, position, generation) = match self.next_free {
            Some(idx) => {
                // SAFETY: `idx` is on the free list, so it was the position
                // of a slot that still exists.
                let position = unsafe { position_of::<C>(idx) };
                let generation = unsafe { self.slots.get_unchecked(position).generation };
                (idx, position, generation)
            }
            None => match C::Idx::from_usize(self.slots.len()) {
                Some(idx) => (idx, self.slots.len(), C::Gen::ZERO),
                None => panic!(
                    "GenMap is full, {} can not address more than {} slots",
                    type_name::<C::Idx>(),
                    self.slots.len()
                ),
            },
        };

        // A vacant slot's generation is even, and the largest value of an
        // unsigned integer is odd, so adding one can not overflow.
        debug_assert!(!generation.is_odd());
        // SAFETY: `KeyPiece` is unsafe to implement and promises `checked_add`
        // behaves like the standard integers', so the sum fits.
        let generation = unsafe { generation.checked_add(C::Gen::ONE).unwrap_unchecked() };

        // SAFETY: `generation` was even and had one added, so it is odd.
        let key = unsafe { KeyOf::<C>::from_parts_unchecked(idx, generation) };

        let value = f(key)?;

        match self.next_free {
            Some(_) => {
                // SAFETY: `position` was in bounds above, and `f` could not
                // have touched the map since it holds no reference to it.
                let slot = unsafe { self.slots.get_unchecked_mut(position) };
                // SAFETY: the slot came off the free list, so it is vacant and
                // its `vacant` field is the live one.
                self.next_free = unsafe { slot.data.vacant };
                slot.data.occupied = ManuallyDrop::new(value);
                slot.generation = generation;
            }
            None => self.slots.push(Slot {
                generation,
                data: SlotData {
                    occupied: ManuallyDrop::new(value),
                },
            }),
        }
        self.len += 1;
        Ok(key)
    }

    /// Removes and returns the value corresponding to `key`, or `None` if the
    /// key is invalid.
    #[inline]
    pub fn remove(&mut self, key: KeyOf<C>) -> Option<T> {
        let position = key.index().into_usize()?;
        let slot = self.slots.get(position)?;
        if slot.generation != key.generation() {
            return None;
        }
        // SAFETY: a key's generation is always odd, so a matching slot is
        // occupied.
        Some(unsafe { self.take(key.index(), position) })
    }

    /// Moves the value out of the slot at `position` and frees it, or retires
    /// it if its generation overflowed and the config says so.
    ///
    /// # Safety
    ///
    /// The slot at `position` must be occupied, and `idx` must be `position`
    /// as a `C::Idx`.
    unsafe fn take(&mut self, idx: C::Idx, position: usize) -> T {
        // SAFETY: the caller promises an occupied slot at `position`, so it
        // is in bounds and `occupied` is the live field.
        let (slot, value) = unsafe {
            let slot = self.slots.get_unchecked_mut(position);
            debug_assert!(slot.is_occupied());
            let value = ManuallyDrop::take(&mut slot.data.occupied);
            (slot, value)
        };
        self.len -= 1;

        match slot.generation.checked_add(C::Gen::ONE) {
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
                    // Retired. The slot stays off the free list for good.
                    slot.data.vacant = None;
                }
            }
        }
        value
    }

    /// Removes every value. Slots are kept and every old key stays invalid.
    pub fn clear(&mut self) {
        for position in 0..self.slots.len() {
            // SAFETY: `position` is below the slot count, which `take` does
            // not change.
            let occupied = unsafe { self.slots.get_unchecked(position).is_occupied() };
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
        self.slots.clear();
        self.next_free = None;
        self.len = 0;
    }

    /// Keeps only the values for which `f` returns `true`. `f` may mutate
    /// them.
    pub fn retain<F>(&mut self, mut f: F)
    where
        F: FnMut(KeyOf<C>, &mut T) -> bool,
    {
        for position in 0..self.slots.len() {
            // SAFETY: `position` is below the slot count, which neither
            // `take` nor `f` (which never sees the map) changes.
            let slot = unsafe { self.slots.get_unchecked_mut(position) };
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
            slots: self.slots.iter().enumerate(),
            remaining: self.len,
        }
    }

    /// Iterates over every value mutably together with its key, in slot order.
    #[inline]
    pub fn iter_mut(&mut self) -> IterMut<'_, T, C> {
        IterMut {
            slots: self.slots.iter_mut().enumerate(),
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

impl<T, C: Config> Index<KeyOf<C>> for GenMap<T, C> {
    type Output = T;

    #[inline]
    fn index(&self, key: KeyOf<C>) -> &T {
        self.get(key).expect("invalid GenMap key")
    }
}

impl<T, C: Config> IndexMut<KeyOf<C>> for GenMap<T, C> {
    #[inline]
    fn index_mut(&mut self, key: KeyOf<C>) -> &mut T {
        self.get_mut(key).expect("invalid GenMap key")
    }
}

impl<T: fmt::Debug, C: Config> fmt::Debug for GenMap<T, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

/// Empties the slot `Vec` on drop. Only dropped while unwinding out of
/// `clone_from`, where a half cloned `Vec` would disagree with `len` and the
/// free list.
struct ClearOnUnwind<'a, T, C: Config>(&'a mut Vec<Slot<T, C>>);

impl<T, C: Config> Drop for ClearOnUnwind<'_, T, C> {
    fn drop(&mut self) {
        self.0.clear();
    }
}

impl<T: Clone, C: Config> Clone for GenMap<T, C> {
    /// The clone has the same slots, free list and generations, so every key
    /// of the original works on it.
    fn clone(&self) -> Self { 
        Self {
            slots: self.slots.iter().map(Slot::clone_slot).collect(),
            next_free: self.next_free,
            len: self.len,
        }
    }

    /// Reuses this map's allocation. If a value's `clone` panics, this map is
    /// left empty.
    fn clone_from(&mut self, source: &Self) {
        self.next_free = None;
        self.len = 0;
        let guard = ClearOnUnwind(&mut self.slots);
        guard.0.clear();
        guard.0.extend(source.slots.iter().map(Slot::clone_slot));
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
    type Item = (KeyOf<C>, &'a T);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        for (position, slot) in self.slots.by_ref() {
            if slot.is_occupied() {
                self.remaining -= 1;
                // SAFETY: the slot is occupied and `position` is where it
                // sits in the backing `Vec`.
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
                // sits in the backing `Vec`.
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
    type Item = (KeyOf<C>, &'a mut T);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        for (position, slot) in self.slots.by_ref() {
            if slot.is_occupied() {
                self.remaining -= 1;
                // SAFETY: the slot is occupied and `position` is where it
                // sits in the backing `Vec`.
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
                // sits in the backing `Vec`.
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
    type Item = KeyOf<C>;

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
    slots: Enumerate<alloc::vec::IntoIter<Slot<T, C>>>,
    remaining: usize,
}

impl<T, C: Config> Iterator for IntoIter<T, C> {
    type Item = (KeyOf<C>, T);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        for (position, mut slot) in self.slots.by_ref() {
            if slot.is_occupied() {
                self.remaining -= 1;
                // SAFETY: the slot is occupied and `position` is where it sat
                // in the `Vec`. Zeroing the generation afterwards stops the
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
    type Item = (KeyOf<C>, T);
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
    type Item = (KeyOf<C>, &'a T);
    type IntoIter = Iter<'a, T, C>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T, C: Config> IntoIterator for &'a mut GenMap<T, C> {
    type Item = (KeyOf<C>, &'a mut T);
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
    type Item = (KeyOf<C>, T);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        while self.position < self.map.slots.len() {
            let position = self.position;
            self.position += 1;
            // SAFETY: `position` is below the slot count, which `take` does
            // not change.
            let slot = unsafe { self.map.slots.get_unchecked(position) };
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
