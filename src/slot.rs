use crate::config::KeyConfig;
use crate::key_piece::KeyPiece;
use crate::parity::{Even, Odd};
use core::fmt;
use core::mem::ManuallyDrop;
use core::ptr;

/// The generation of a [`Slot`] together with its value. The variant says
/// whether the generation is odd or even, and so whether the value is the
/// slot's `T` or its `U`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Parity<G: KeyPiece, T, U> {
    /// The generation is odd and the value is a `T`.
    Odd(Odd<G>, T),
    /// The generation is even and the value is a `U`.
    Even(Even<G>, U),
}

/// The value of a [`Slot`]. `odd` is live while the slot's generation is odd,
/// and `even` is live while it is even.
union Value<T, U> {
    odd: ManuallyDrop<T>,
    even: ManuallyDrop<U>,
}

/// A generation together with a `T` while the generation is odd, or a `U`
/// while it is even.
///
/// A [`GenMap`](crate::GenMap) keeps each of its values in a slot like this,
/// its [`MapSlot`](crate::MapSlot). A key's generation is odd, so a key that
/// matches a slot finds its `T`.
///
/// # Examples
///
/// ```
/// use gen_map::{Even, GenMap, Parity, Slot};
///
/// let mut map = GenMap::new();
/// let key = map.insert("a");
///
/// // The slot starts out at generation zero, so it holds a `U`.
/// let mut slot = Slot::<u32, u64, ()>::new(Parity::Even(Even::ZERO, ()));
/// assert!(slot.get_odd(key.generation()).is_none());
///
/// slot.replace(Parity::Odd(key.generation(), 10));
/// assert_eq!(slot.get_odd(key.generation()), Some(&10));
/// ```
pub struct Slot<G: KeyPiece, T, U> {
    generation: G,
    /// The parity of `generation` says which field is live.
    value: Value<T, U>,
}

impl<G: KeyPiece, T, U> Slot<G, T, U> {
    /// Creates a slot with the generation and the value in `parity`.
    #[inline]
    pub fn new(parity: Parity<G, T, U>) -> Self {
        match parity {
            Parity::Odd(generation, value) => Self::new_odd(generation, value),
            Parity::Even(generation, value) => Self::new_even(generation, value),
        }
    }

    /// Creates a slot with an odd `generation` and a `T`.
    #[inline]
    pub fn new_odd(generation: Odd<G>, value: T) -> Self {
        Self {
            generation: G::from_non_zero(generation.get()),
            value: Value {
                odd: ManuallyDrop::new(value),
            },
        }
    }

    /// Creates a slot with an even `generation` and a `U`.
    #[inline]
    pub fn new_even(generation: Even<G>, value: U) -> Self {
        Self {
            generation: generation.get(),
            value: Value {
                even: ManuallyDrop::new(value),
            },
        }
    }

    /// Returns the slot's generation.
    #[inline]
    pub fn generation(&self) -> G {
        self.generation
    }

    /// Returns `true` if the generation is odd, which means the slot holds a
    /// `T`.
    #[inline]
    pub fn is_odd(&self) -> bool {
        self.generation.is_odd()
    }

    /// Returns `true` if the generation is even, which means the slot holds a
    /// `U`.
    #[inline]
    pub fn is_even(&self) -> bool {
        !self.generation.is_odd()
    }

    /// Returns the generation and a reference to the value.
    #[inline]
    pub fn as_parity(&self) -> Parity<G, &T, &U> {
        // SAFETY: the parity of the generation says which field is live, and
        // the branch taken is the one whose wrapper the generation fits.
        unsafe {
            if self.is_odd() {
                Parity::Odd(Odd::new_unchecked(self.generation), &self.value.odd)
            } else {
                Parity::Even(Even::new_unchecked(self.generation), &self.value.even)
            }
        }
    }

    /// Returns the generation and a mutable reference to the value.
    #[inline]
    pub fn as_parity_mut(&mut self) -> Parity<G, &mut T, &mut U> {
        // SAFETY: the same as in `as_parity`.
        unsafe {
            if self.is_odd() {
                Parity::Odd(Odd::new_unchecked(self.generation), &mut self.value.odd)
            } else {
                Parity::Even(Even::new_unchecked(self.generation), &mut self.value.even)
            }
        }
    }

    /// Takes the generation and the value out of the slot.
    #[inline]
    pub fn into_parity(self) -> Parity<G, T, U> {
        let mut slot = ManuallyDrop::new(self);
        // SAFETY: the parity of the generation says which field is live. It
        // is taken out once, and the slot is never dropped, so it is not
        // dropped a second time.
        unsafe {
            if slot.is_odd() {
                Parity::Odd(
                    Odd::new_unchecked(slot.generation),
                    ManuallyDrop::take(&mut slot.value.odd),
                )
            } else {
                Parity::Even(
                    Even::new_unchecked(slot.generation),
                    ManuallyDrop::take(&mut slot.value.even),
                )
            }
        }
    }

    /// Returns a reference to the `T` if the slot's generation is
    /// `generation`.
    #[inline]
    pub fn get_odd(&self, generation: Odd<G>) -> Option<&T> {
        if self.generation == G::from_non_zero(generation.get()) {
            // SAFETY: the generation is odd, so `odd` is the live field.
            Some(unsafe { &self.value.odd })
        } else {
            None
        }
    }

    /// Returns a mutable reference to the `T` if the slot's generation is
    /// `generation`.
    #[inline]
    pub fn get_odd_mut(&mut self, generation: Odd<G>) -> Option<&mut T> {
        if self.generation == G::from_non_zero(generation.get()) {
            // SAFETY: the generation is odd, so `odd` is the live field.
            Some(unsafe { &mut self.value.odd })
        } else {
            None
        }
    }

    /// Takes the `T` out of the slot if the slot's generation is
    /// `generation`.
    ///
    /// # Errors
    ///
    /// Hands the slot back if its generation is not `generation`.
    #[inline]
    pub fn into_odd(self, generation: Odd<G>) -> Result<T, Self> {
        if self.generation == G::from_non_zero(generation.get()) {
            let mut slot = ManuallyDrop::new(self);
            // SAFETY: the generation is odd, so `odd` is the live field, and
            // the slot is never dropped, so the value is not dropped twice.
            Ok(unsafe { ManuallyDrop::take(&mut slot.value.odd) })
        } else {
            Err(self)
        }
    }

    /// Returns a reference to the `U` if the slot's generation is
    /// `generation`.
    #[inline]
    pub fn get_even(&self, generation: Even<G>) -> Option<&U> {
        if self.generation == generation.get() {
            // SAFETY: the generation is even, so `even` is the live field.
            Some(unsafe { &self.value.even })
        } else {
            None
        }
    }

    /// Returns a mutable reference to the `U` if the slot's generation is
    /// `generation`.
    #[inline]
    pub fn get_even_mut(&mut self, generation: Even<G>) -> Option<&mut U> {
        if self.generation == generation.get() {
            // SAFETY: the generation is even, so `even` is the live field.
            Some(unsafe { &mut self.value.even })
        } else {
            None
        }
    }

    /// Takes the `U` out of the slot if the slot's generation is
    /// `generation`.
    ///
    /// # Errors
    ///
    /// Hands the slot back if its generation is not `generation`.
    #[inline]
    pub fn into_even(self, generation: Even<G>) -> Result<U, Self> {
        if self.generation == generation.get() {
            let mut slot = ManuallyDrop::new(self);
            // SAFETY: the generation is even, so `even` is the live field,
            // and the slot is never dropped, so the value is not dropped
            // twice.
            Ok(unsafe { ManuallyDrop::take(&mut slot.value.even) })
        } else {
            Err(self)
        }
    }

    /// Puts the generation and the value in `parity` into the slot, and
    /// returns the ones it had before.
    #[inline]
    pub fn replace(&mut self, parity: Parity<G, T, U>) -> Parity<G, T, U> {
        match parity {
            Parity::Odd(generation, value) => self.set_odd(generation, value),
            Parity::Even(generation, value) => self.set_even(generation, value),
        }
    }

    /// Puts an odd `generation` and a `T` into the slot, and returns the
    /// generation and the value it had before.
    #[inline]
    pub fn set_odd(&mut self, generation: Odd<G>, value: T) -> Parity<G, T, U> {
        // SAFETY: the old value is moved out of a copy of the slot, and both
        // fields are overwritten right after, before anything can panic or
        // drop the slot, so the old value is neither dropped nor used twice.
        let old = unsafe { ptr::read(self).into_parity() };
        self.generation = G::from_non_zero(generation.get());
        self.value.odd = ManuallyDrop::new(value);
        old
    }

    /// Puts an even `generation` and a `U` into the slot, and returns the
    /// generation and the value it had before.
    #[inline]
    pub fn set_even(&mut self, generation: Even<G>, value: U) -> Parity<G, T, U> {
        // SAFETY: the same as in `set_odd`.
        let old = unsafe { ptr::read(self).into_parity() };
        self.generation = generation.get();
        self.value.even = ManuallyDrop::new(value);
        old
    }

    /// Moves the `T` out of the slot and puts an even `generation` and a `U`
    /// in its place.
    ///
    /// # Safety
    ///
    /// The generation must be odd.
    #[inline]
    pub unsafe fn replace_odd_unchecked(&mut self, generation: Even<G>, value: U) -> T {
        debug_assert!(self.is_odd());
        // SAFETY: the caller promises an odd generation, so `odd` is the live
        // field. It is overwritten right after without being dropped.
        let old = unsafe { ManuallyDrop::take(&mut self.value.odd) };
        self.generation = generation.get();
        self.value.even = ManuallyDrop::new(value);
        old
    }

    /// Moves the `U` out of the slot and puts an odd `generation` and a `T`
    /// in its place.
    ///
    /// # Safety
    ///
    /// The generation must be even.
    #[inline]
    pub unsafe fn replace_even_unchecked(&mut self, generation: Odd<G>, value: T) -> U {
        debug_assert!(self.is_even());
        // SAFETY: the caller promises an even generation, so `even` is the
        // live field. It is overwritten right after without being dropped.
        let old = unsafe { ManuallyDrop::take(&mut self.value.even) };
        self.generation = G::from_non_zero(generation.get());
        self.value.odd = ManuallyDrop::new(value);
        old
    }

    /// Returns a reference to the `T` without checking the generation.
    ///
    /// # Safety
    ///
    /// The generation must be odd.
    #[inline]
    pub unsafe fn get_odd_unchecked(&self) -> &T {
        debug_assert!(self.is_odd());
        // SAFETY: the caller promises an odd generation, so `odd` is the
        // live field.
        unsafe { &self.value.odd }
    }

    /// Returns a mutable reference to the `T` without checking the
    /// generation.
    ///
    /// # Safety
    ///
    /// The generation must be odd.
    #[inline]
    pub unsafe fn get_odd_unchecked_mut(&mut self) -> &mut T {
        debug_assert!(self.is_odd());
        // SAFETY: the caller promises an odd generation, so `odd` is the
        // live field.
        unsafe { &mut self.value.odd }
    }

    /// Returns a reference to the `U` without checking the generation.
    ///
    /// # Safety
    ///
    /// The generation must be even.
    #[inline]
    pub unsafe fn get_even_unchecked(&self) -> &U {
        debug_assert!(self.is_even());
        // SAFETY: the caller promises an even generation, so `even` is the
        // live field.
        unsafe { &self.value.even }
    }

    /// Returns a mutable reference to the `U` without checking the
    /// generation.
    ///
    /// # Safety
    ///
    /// The generation must be even.
    #[inline]
    pub unsafe fn get_even_unchecked_mut(&mut self) -> &mut U {
        debug_assert!(self.is_even());
        // SAFETY: the caller promises an even generation, so `even` is the
        // live field.
        unsafe { &mut self.value.even }
    }
}

impl<G: KeyPiece, T, U> Drop for Slot<G, T, U> {
    #[inline]
    fn drop(&mut self) {
        // SAFETY: the parity of the generation says which field is live, and
        // the slot is not used again after this.
        unsafe {
            if self.is_odd() {
                ManuallyDrop::drop(&mut self.value.odd);
            } else {
                ManuallyDrop::drop(&mut self.value.even);
            }
        }
    }
}

impl<G: KeyPiece, T: Clone, U: Clone> Clone for Slot<G, T, U> {
    #[inline]
    fn clone(&self) -> Self {
        match self.as_parity() {
            Parity::Odd(generation, value) => Self::new_odd(generation, value.clone()),
            Parity::Even(generation, value) => Self::new_even(generation, value.clone()),
        }
    }
}

impl<G: KeyPiece, T: fmt::Debug, U: fmt::Debug> fmt::Debug for Slot<G, T, U> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value: &dyn fmt::Debug = match self.as_parity() {
            Parity::Odd(_, value) => value,
            Parity::Even(_, value) => value,
        };
        f.debug_struct("Slot")
            .field("generation", &self.generation)
            .field("value", value)
            .finish()
    }
}

mod sealed {
    /// Keeps [`GenSlotItem`](super::GenSlotItem) and
    /// [`SecondarySlotItem`](super::SecondarySlotItem) from being implemented
    /// outside this crate.
    pub trait Sealed {}
}

/// The slot a [`GenMap`](crate::GenMap) keeps each of its values in, as a
/// [`GenMapConfig`](crate::GenMapConfig) sees it.
///
/// Every value in a map sits in a slot, together with the slot's
/// generation. The slots of a `GenMap<T, C>` are
/// [`MapSlot<T, C>`](crate::MapSlot). A config implements `GenMapConfig<S>`
/// for the slot types `S` it supports, usually for all of them at once with
/// `impl<S: GenSlotItem> GenMapConfig<S> for YourConfig`. Inside that impl, `S`
/// is the slot and `S::Value` is the type of the value in it, so for a
/// `GenMap<T, C>` it is `T`.
///
/// Only [`Slot`] implements `GenSlotItem`, and it cannot be implemented outside
/// this crate.
///
/// # Bounds on the value and the slot
///
/// A config can limit which maps can use it with bounds on the value type
/// or on the slot type. A map whose values or slots do not meet the bounds
/// fails to compile.
///
/// A bound on the value limits the value types:
///
/// ```
/// use gen_map::{DefaultKeyConfig, GenMap, GenMapConfig, GenSlotItem, MapConfig};
///
/// /// Only for values that are `Copy`.
/// struct CopyValues;
///
/// impl<T: Copy> MapConfig<T> for CopyValues {
///     type KeyConfig = DefaultKeyConfig;
/// }
///
/// impl<S: GenSlotItem> GenMapConfig<S> for CopyValues
/// where
///     S::Value: Copy,
/// {
///     type Storage = Vec<S>;
/// }
///
/// let mut map = GenMap::<u32, CopyValues>::new_with_config();
/// let key = map.insert(5);
/// assert_eq!(map[key], 5);
///
/// // `String` is not `Copy`, so this does not compile:
/// // let map = GenMap::<String, CopyValues>::new_with_config();
/// ```
///
/// To allow only `u32` values, implement `MapConfig<u32>` instead of
/// `MapConfig<T>`, and `GenMapConfig` only for slots whose `Value` is `u32`:
///
/// ```
/// use gen_map::{DefaultKeyConfig, GenMap, GenMapConfig, GenSlotItem, MapConfig};
///
/// /// Only for values that are `u32`.
/// struct U32Values;
///
/// impl MapConfig<u32> for U32Values {
///     type KeyConfig = DefaultKeyConfig;
/// }
///
/// impl<S: GenSlotItem<Value = u32>> GenMapConfig<S> for U32Values {
///     type Storage = Vec<S>;
/// }
///
/// let mut map = GenMap::<u32, U32Values>::new_with_config();
/// let key = map.insert(5);
/// assert_eq!(map[key], 5);
///
/// // The values are `u64`, not `u32`, so this does not compile:
/// // let map = GenMap::<u64, U32Values>::new_with_config();
/// ```
///
/// A bound on `S` limits the slots. It is what a config needs when its
/// storage type requires something of the items it holds, because those
/// items are slots, not bare values. A slot is `Clone`, `Debug`, `Send` or
/// `Sync` when its value is, and it is never `Copy`.
///
/// ```
/// use gen_map::{DefaultKeyConfig, GenMap, GenMapConfig, GenSlotItem, SlotStorage, MapConfig};
///
/// /// A storage that only holds items that can be cloned. Its `SlotStorage`
/// /// impl, which forwards every method to the `Vec`, is hidden here.
/// struct ClonePool<S: Clone>(Vec<S>);
/// # // SAFETY: every method forwards to the `Vec`.
/// # unsafe impl<S: Clone> SlotStorage for ClonePool<S> {
/// #     type Item = S;
/// #     type Error = ();
/// #     const EMPTY: Self = ClonePool(Vec::new());
/// #     fn with_capacity(capacity: usize) -> Self {
/// #         ClonePool(Vec::with_capacity(capacity))
/// #     }
/// #     fn capacity(&self) -> usize {
/// #         self.0.capacity()
/// #     }
/// #     fn as_slice(&self) -> &[S] {
/// #         &self.0
/// #     }
/// #     fn as_mut_slice(&mut self) -> &mut [S] {
/// #         &mut self.0
/// #     }
/// #     fn ensure_room(&mut self, _additional: usize) -> Result<(), ()> {
/// #         Ok(())
/// #     }
/// #     fn try_push(&mut self, item: S) -> Result<(), S> {
/// #         self.0.push(item);
/// #         Ok(())
/// #     }
/// #     fn clear(&mut self) {
/// #         self.0.clear();
/// #     }
/// # }
///
/// /// Only for slots that can be cloned, since `ClonePool<S>` needs
/// /// `S: Clone`.
/// struct Cloneable;
///
/// impl<T> MapConfig<T> for Cloneable {
///     type KeyConfig = DefaultKeyConfig;
/// }
///
/// impl<S: GenSlotItem + Clone> GenMapConfig<S> for Cloneable {
///     type Storage = ClonePool<S>;
/// }
///
/// // `String` is `Clone`, so a slot holding one is too.
/// let mut map = GenMap::<String, Cloneable>::new_with_config();
/// let key = map.insert("a".to_string());
/// assert_eq!(map[key], "a");
///
/// // `Mutex` is not `Clone`, so neither is a slot holding one, and this
/// // does not compile:
/// // let map = GenMap::<std::sync::Mutex<u32>, Cloneable>::new_with_config();
/// ```
pub trait GenSlotItem: sealed::Sealed {
    /// The type of the value in the slot. For the slots of a
    /// `GenMap<T, C>`, it is `T`.
    type Value;
}

impl<G: KeyPiece, T, U> sealed::Sealed for Slot<G, T, U> {}

impl<G: KeyPiece, T, U> GenSlotItem for Slot<G, T, U> {
    type Value = T;
}

/// The slot a [`SecondaryMap`](crate::SecondaryMap) keeps each of its values
/// in, as a [`SecondaryMapConfig`](crate::SecondaryMapConfig) sees it.
///
/// It is to a `SecondaryMapConfig` what [`GenSlotItem`] is to a
/// [`GenMapConfig`](crate::GenMapConfig). A config implements
/// `SecondaryMapConfig<S>` for the slot types `S` it supports, usually for
/// all of them at once with
/// `impl<S: SecondarySlotItem> SecondaryMapConfig<S> for YourConfig`. Inside
/// that impl, `S` is the slot and `S::Value` is the type of the value in it.
/// For a `SecondaryMap<T, C>`, `S::Value` is `T`. Bounds on `S` and on
/// `S::Value` limit which maps can use the config, in the same way as in the
/// examples on [`GenSlotItem`].
///
/// Only [`SecondarySlot`] implements it, and it cannot be implemented
/// outside this crate.
pub trait SecondarySlotItem: sealed::Sealed {
    /// The type of the value in the slot. For the slots of a
    /// `SecondaryMap<T, C>`, it is `T`.
    type Value;
}

/// The slot a [`SecondaryMap`](crate::SecondaryMap) keeps each of its values
/// in. It has a generation, and it holds a `T` while the generation is odd
/// and no value while it is even.
///
/// `K` is the key config of the map's keys, and its `Gen` is the slot's
/// generation type. The slots of a `SecondaryMap<T, C>` are
/// [`SecondaryMapSlot<T, C>`](crate::SecondaryMapSlot).
///
/// # Examples
///
/// ```
/// use gen_map::{DefaultKeyConfig, GenMap, SecondarySlot};
///
/// let mut map = GenMap::new();
/// let key = map.insert("a");
///
/// let mut slot = SecondarySlot::<DefaultKeyConfig, u64>::empty();
/// assert_eq!(slot.get(), None);
///
/// assert_eq!(slot.replace(key.generation(), 10), None);
/// assert_eq!(slot.get(), Some((key.generation(), &10)));
/// assert_eq!(slot.take(), Some(10));
/// assert_eq!(slot.get(), None);
/// ```
pub struct SecondarySlot<K: KeyConfig, T>(Slot<K::Gen, T, ()>);

impl<K: KeyConfig, T> SecondarySlot<K, T> {
    /// Creates a slot with generation zero and no value.
    #[inline]
    pub fn empty() -> Self {
        Self(Slot::new_even(Even::ZERO, ()))
    }

    /// Creates a slot that holds `value` under `generation`.
    #[inline]
    pub fn new(generation: Odd<K::Gen>, value: T) -> Self {
        Self(Slot::new_odd(generation, value))
    }

    /// Returns the generation and a reference to the value, or `None` if
    /// the slot is empty.
    #[inline]
    pub fn get(&self) -> Option<(Odd<K::Gen>, &T)> {
        match self.0.as_parity() {
            Parity::Odd(generation, value) => Some((generation, value)),
            Parity::Even(..) => None,
        }
    }

    /// Returns the generation and a mutable reference to the value, or
    /// `None` if the slot is empty.
    #[inline]
    pub fn get_mut(&mut self) -> Option<(Odd<K::Gen>, &mut T)> {
        match self.0.as_parity_mut() {
            Parity::Odd(generation, value) => Some((generation, value)),
            Parity::Even(..) => None,
        }
    }

    /// Returns a reference to the value if the slot's generation is
    /// `generation`.
    #[inline]
    pub fn get_odd(&self, generation: Odd<K::Gen>) -> Option<&T> {
        self.0.get_odd(generation)
    }

    /// Returns a mutable reference to the value if the slot's generation is
    /// `generation`.
    #[inline]
    pub fn get_odd_mut(&mut self, generation: Odd<K::Gen>) -> Option<&mut T> {
        self.0.get_odd_mut(generation)
    }

    /// Returns the slot's generation. It is odd while the slot holds a value,
    /// and zero while it holds none, since only [`empty`](Self::empty) and
    /// [`take`](Self::take) leave a slot without a value.
    #[inline]
    pub(crate) fn generation(&self) -> K::Gen {
        self.0.generation()
    }

    /// Returns a reference to the value without checking that there is one.
    ///
    /// # Safety
    ///
    /// The slot must hold a value.
    #[inline]
    pub(crate) unsafe fn get_odd_unchecked(&self) -> &T {
        // SAFETY: the caller promises that the slot holds a value, so its
        // generation is odd.
        unsafe { self.0.get_odd_unchecked() }
    }

    /// Returns a mutable reference to the value without checking that
    /// there is one.
    ///
    /// # Safety
    ///
    /// The slot must hold a value.
    #[inline]
    pub(crate) unsafe fn get_odd_unchecked_mut(&mut self) -> &mut T {
        // SAFETY: the caller promises that the slot holds a value, so its
        // generation is odd.
        unsafe { self.0.get_odd_unchecked_mut() }
    }

    /// Takes the generation and the value out of the slot, or returns
    /// `None` if the slot is empty.
    #[inline]
    pub fn into_inner(self) -> Option<(Odd<K::Gen>, T)> {
        match self.0.into_parity() {
            Parity::Odd(generation, value) => Some((generation, value)),
            Parity::Even(..) => None,
        }
    }

    /// Puts `value` in the slot under `generation`, and returns the value
    /// the slot held before, if it held one.
    #[inline]
    pub fn replace(&mut self, generation: Odd<K::Gen>, value: T) -> Option<T> {
        match self.0.set_odd(generation, value) {
            Parity::Odd(_, old) => Some(old),
            Parity::Even(..) => None,
        }
    }

    /// Takes the value out and leaves the slot empty, with generation zero.
    /// Returns `None` if the slot was already empty.
    #[inline]
    pub fn take(&mut self) -> Option<T> {
        match self.0.set_even(Even::ZERO, ()) {
            Parity::Odd(_, value) => Some(value),
            Parity::Even(..) => None,
        }
    }
}

impl<K: KeyConfig, T: Clone> Clone for SecondarySlot<K, T> {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<K: KeyConfig, T: fmt::Debug> fmt::Debug for SecondarySlot<K, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecondarySlot")
            .field("generation", &self.0.generation())
            .field("value", &self.get().map(|(_, value)| value))
            .finish()
    }
}

impl<K: KeyConfig, T> sealed::Sealed for SecondarySlot<K, T> {}

impl<K: KeyConfig, T> SecondarySlotItem for SecondarySlot<K, T> {
    type Value = T;
}
