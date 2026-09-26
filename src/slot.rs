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
/// its [`MapSlot`](crate::MapSlot). The same type can hold data for any
/// collection keyed by the map's keys, such as a secondary map. A key's
/// generation is odd, so a key that matches a slot finds its `T`.
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
    /// Keeps [`SlotItem`](super::SlotItem) from being implemented outside
    /// this crate.
    pub trait Sealed {}
}

/// A slot type a [`MapConfig`](crate::MapConfig) can be implemented for.
///
/// Only [`Slot`] implements it, and it cannot be implemented outside this
/// crate. A config is implemented for every slot type it supports, and
/// [`Item`](Self::Item) names the value the slot holds, so the config can
/// put bounds on it.
pub trait SlotItem: sealed::Sealed {
    /// The value the slot holds while its generation is odd. For a map's
    /// [`MapSlot`](crate::MapSlot), it is the map's value type.
    type Item;
}

impl<G: KeyPiece, T, U> sealed::Sealed for Slot<G, T, U> {}

impl<G: KeyPiece, T, U> SlotItem for Slot<G, T, U> {
    type Item = T;
}
