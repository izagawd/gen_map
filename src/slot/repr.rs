use crate::key::parity::{Even, Odd};
use crate::key::piece::KeyPiece;
use crate::slot::{Parity, ParityMut, ParityRef};
use core::hint::unreachable_unchecked;
use core::mem::{self, ManuallyDrop};

/// A `Repr` keeps the generation and the value of a [`Slot`](super::Slot).
/// The generation type `G` picks the `Repr` of its slots through
/// [`KeyPiece::SlotRepr`]. This trait is in a private module, so no type
/// outside this crate can implement it.
// `Slot` relies on every method doing what its docs say, and the maps' unsafe
// code relies on `Slot`.
pub trait Repr<G: KeyPiece, T, U>: Sized {
    /// Returns a `Repr` that holds an odd `generation` and a `T`.
    fn new_odd(generation: Odd<G>, value: T) -> Self;

    /// Returns a `Repr` that holds an even `generation` and a `U`.
    fn new_even(generation: Even<G>, value: U) -> Self;

    /// Returns the generation.
    fn generation(&self) -> G;

    /// Returns `true` if the generation is odd, which means the `Repr` holds a
    /// `T`.
    fn is_odd(&self) -> bool;

    /// Returns references to the generation and the value.
    fn as_parity(&self) -> ParityRef<'_, G, T, U>;

    /// Returns mutable references to the generation and the value.
    fn as_parity_mut(&mut self) -> ParityMut<'_, G, T, U>;

    /// Takes the generation and the value out.
    fn into_parity(self) -> Parity<G, T, U>;

    /// Returns a reference to the `T`.
    ///
    /// # Safety
    ///
    /// The generation must be odd.
    unsafe fn get_odd_unchecked(&self) -> &T;

    /// Returns a mutable reference to the `T`.
    ///
    /// # Safety
    ///
    /// The generation must be odd.
    unsafe fn get_odd_unchecked_mut(&mut self) -> &mut T;

    /// Returns a reference to the `U`.
    ///
    /// # Safety
    ///
    /// The generation must be even.
    unsafe fn get_even_unchecked(&self) -> &U;

    /// Returns a mutable reference to the `U`.
    ///
    /// # Safety
    ///
    /// The generation must be even.
    unsafe fn get_even_unchecked_mut(&mut self) -> &mut U;

    /// Moves the `T` out and puts an even `generation` and a `U` in its place.
    ///
    /// # Safety
    ///
    /// The generation must be odd.
    unsafe fn replace_odd_unchecked(&mut self, generation: Even<G>, value: U) -> T;

    /// Moves the `U` out and puts an odd `generation` and a `T` in its place.
    ///
    /// # Safety
    ///
    /// The generation must be even.
    unsafe fn replace_even_unchecked(&mut self, generation: Odd<G>, value: T) -> U;
}

/// A `Value` holds the `T` or the `U` of a [`UnionSlot`]. `odd` is live while
/// the slot's generation is odd, and `even` is live while it is even.
union Value<T, U> {
    odd: ManuallyDrop<T>,
    even: ManuallyDrop<U>,
}

/// A `UnionSlot` keeps the generation next to a union of a `T` and a `U`. The
/// parity of the generation says which of the two the union holds.
pub struct UnionSlot<G: KeyPiece, T, U> {
    generation: G,
    /// The parity of `generation` says which field is live.
    value: Value<T, U>,
}

impl<G: KeyPiece, T, U> Repr<G, T, U> for UnionSlot<G, T, U> {
    #[inline]
    fn new_odd(generation: Odd<G>, value: T) -> Self {
        Self {
            generation: G::from_non_zero(generation.get()),
            value: Value {
                odd: ManuallyDrop::new(value),
            },
        }
    }

    #[inline]
    fn new_even(generation: Even<G>, value: U) -> Self {
        Self {
            generation: G::from_non_max(generation.get()),
            value: Value {
                even: ManuallyDrop::new(value),
            },
        }
    }

    #[inline]
    fn generation(&self) -> G {
        self.generation
    }

    #[inline]
    fn is_odd(&self) -> bool {
        self.generation.is_odd()
    }

    #[inline]
    fn as_parity(&self) -> ParityRef<'_, G, T, U> {
        let generation: *const G = &self.generation;
        // SAFETY: the parity of the generation says which field is live. The
        // first branch only runs for an odd generation, so it reads `odd` and
        // treats the generation as an `Odd`, and the second branch does the
        // same with `even` and `Even` for an even generation. A reference to
        // the generation can be cast to a reference to either, as the comments
        // on `Odd` and `Even` explain.
        unsafe {
            if self.is_odd() {
                ParityRef::Odd(&*generation.cast::<Odd<G>>(), &self.value.odd)
            } else {
                ParityRef::Even(&*generation.cast::<Even<G>>(), &self.value.even)
            }
        }
    }

    #[inline]
    fn as_parity_mut(&mut self) -> ParityMut<'_, G, T, U> {
        let odd = self.is_odd();
        let Self { generation, value } = self;
        let generation: *mut G = generation;
        // SAFETY: the same as in `as_parity`. Writing through a reference to an
        // `Odd` stores another `Odd`, which is odd, and writing through a
        // reference to an `Even` stores another `Even`, which is even. When `G`
        // is `u1`, an `Odd` and an `Even` take no space, and writing one leaves
        // the generation as it was. So the generation keeps its parity, and the
        // field it says is live stays live, for as long as the references
        // exist.
        unsafe {
            if odd {
                ParityMut::Odd(&mut *generation.cast::<Odd<G>>(), &mut value.odd)
            } else {
                ParityMut::Even(&mut *generation.cast::<Even<G>>(), &mut value.even)
            }
        }
    }

    #[inline]
    fn into_parity(self) -> Parity<G, T, U> {
        let mut slot = ManuallyDrop::new(self);
        // SAFETY: the parity of the generation says which field is live. That
        // field's value is taken out once, and the slot is never dropped, so
        // the value is not dropped a second time.
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

    #[inline]
    unsafe fn get_odd_unchecked(&self) -> &T {
        debug_assert!(self.is_odd());
        // SAFETY: the caller promises an odd generation, so `odd` is the
        // live field.
        unsafe { &self.value.odd }
    }

    #[inline]
    unsafe fn get_odd_unchecked_mut(&mut self) -> &mut T {
        debug_assert!(self.is_odd());
        // SAFETY: the caller promises an odd generation, so `odd` is the
        // live field.
        unsafe { &mut self.value.odd }
    }

    #[inline]
    unsafe fn get_even_unchecked(&self) -> &U {
        debug_assert!(!self.is_odd());
        // SAFETY: the caller promises an even generation, so `even` is the
        // live field.
        unsafe { &self.value.even }
    }

    #[inline]
    unsafe fn get_even_unchecked_mut(&mut self) -> &mut U {
        debug_assert!(!self.is_odd());
        // SAFETY: the caller promises an even generation, so `even` is the
        // live field.
        unsafe { &mut self.value.even }
    }

    #[inline]
    unsafe fn replace_odd_unchecked(&mut self, generation: Even<G>, value: U) -> T {
        debug_assert!(self.is_odd());
        // SAFETY: the caller promises an odd generation, so `odd` is the live
        // field. It is overwritten right after without being dropped.
        let old = unsafe { ManuallyDrop::take(&mut self.value.odd) };
        self.generation = G::from_non_max(generation.get());
        self.value.even = ManuallyDrop::new(value);
        old
    }

    #[inline]
    unsafe fn replace_even_unchecked(&mut self, generation: Odd<G>, value: T) -> U {
        debug_assert!(!self.is_odd());
        // SAFETY: the caller promises an even generation, so `even` is the
        // live field. It is overwritten right after without being dropped.
        let old = unsafe { ManuallyDrop::take(&mut self.value.even) };
        self.generation = G::from_non_zero(generation.get());
        self.value.odd = ManuallyDrop::new(value);
        old
    }
}

impl<G: KeyPiece, T, U> Drop for UnionSlot<G, T, U> {
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

/// An `EnumSlot` holds an odd generation and a `T` in `Occupied`, or an even
/// generation and a `U` in `Vacant`. For a [`u1`](crate::u1) generation, `Odd`
/// and `Even` take no space, so the enum only stores its variant.
pub enum EnumSlot<G: KeyPiece, T, U> {
    /// The generation is odd and the value is a `T`.
    Occupied(Odd<G>, T),
    /// The generation is even and the value is a `U`.
    Vacant(Even<G>, U),
}

impl<G: KeyPiece, T, U> Repr<G, T, U> for EnumSlot<G, T, U> {
    #[inline]
    fn new_odd(generation: Odd<G>, value: T) -> Self {
        Self::Occupied(generation, value)
    }

    #[inline]
    fn new_even(generation: Even<G>, value: U) -> Self {
        Self::Vacant(generation, value)
    }

    #[inline]
    fn generation(&self) -> G {
        match self {
            Self::Occupied(generation, _) => G::from_non_zero(generation.get()),
            Self::Vacant(generation, _) => G::from_non_max(generation.get()),
        }
    }

    #[inline]
    fn is_odd(&self) -> bool {
        matches!(self, Self::Occupied(..))
    }

    #[inline]
    fn as_parity(&self) -> ParityRef<'_, G, T, U> {
        match self {
            Self::Occupied(generation, value) => ParityRef::Odd(generation, value),
            Self::Vacant(generation, value) => ParityRef::Even(generation, value),
        }
    }

    #[inline]
    fn as_parity_mut(&mut self) -> ParityMut<'_, G, T, U> {
        match self {
            Self::Occupied(generation, value) => ParityMut::Odd(generation, value),
            Self::Vacant(generation, value) => ParityMut::Even(generation, value),
        }
    }

    #[inline]
    fn into_parity(self) -> Parity<G, T, U> {
        match self {
            Self::Occupied(generation, value) => Parity::Odd(generation, value),
            Self::Vacant(generation, value) => Parity::Even(generation, value),
        }
    }

    #[inline]
    unsafe fn get_odd_unchecked(&self) -> &T {
        match self {
            Self::Occupied(_, value) => value,
            // SAFETY: the caller promises an odd generation, which only
            // `Occupied` holds.
            Self::Vacant(..) => unsafe { unreachable_unchecked() },
        }
    }

    #[inline]
    unsafe fn get_odd_unchecked_mut(&mut self) -> &mut T {
        match self {
            Self::Occupied(_, value) => value,
            // SAFETY: the caller promises an odd generation, which only
            // `Occupied` holds.
            Self::Vacant(..) => unsafe { unreachable_unchecked() },
        }
    }

    #[inline]
    unsafe fn get_even_unchecked(&self) -> &U {
        match self {
            Self::Vacant(_, value) => value,
            // SAFETY: the caller promises an even generation, which only
            // `Vacant` holds.
            Self::Occupied(..) => unsafe { unreachable_unchecked() },
        }
    }

    #[inline]
    unsafe fn get_even_unchecked_mut(&mut self) -> &mut U {
        match self {
            Self::Vacant(_, value) => value,
            // SAFETY: the caller promises an even generation, which only
            // `Vacant` holds.
            Self::Occupied(..) => unsafe { unreachable_unchecked() },
        }
    }

    #[inline]
    unsafe fn replace_odd_unchecked(&mut self, generation: Even<G>, value: U) -> T {
        match mem::replace(self, Self::Vacant(generation, value)) {
            Self::Occupied(_, old) => old,
            // SAFETY: the caller promises an odd generation, which only
            // `Occupied` holds.
            Self::Vacant(..) => unsafe { unreachable_unchecked() },
        }
    }

    #[inline]
    unsafe fn replace_even_unchecked(&mut self, generation: Odd<G>, value: T) -> U {
        match mem::replace(self, Self::Occupied(generation, value)) {
            Self::Vacant(_, old) => old,
            // SAFETY: the caller promises an even generation, which only
            // `Vacant` holds.
            Self::Occupied(..) => unsafe { unreachable_unchecked() },
        }
    }
}
