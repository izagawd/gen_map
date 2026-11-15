pub(crate) mod repr;

use crate::key::parity::{Even, Odd};
use crate::key::piece::KeyPiece;
use crate::parity_union::repr::Repr;
use core::fmt;
use core::hint::unreachable_unchecked;
use core::mem;
use core::panic::{RefUnwindSafe, UnwindSafe};

/// The generation of a [`ParityUnion`] together with its value. The variant
/// says whether the generation is odd or even, and so whether the value is the
/// `T` or the `U` of the `ParityUnion`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Parity<G: KeyPiece, T, U> {
    /// The generation is odd and the value is a `T`.
    Odd(Odd<G>, T),
    /// The generation is even and the value is a `U`.
    Even(Even<G>, U),
}

/// A `ParityUnion` holds a generation together with a `T` while the
/// generation is odd, or a `U` while it is even.
///
/// A `ParityUnion` with a generation from `u8` to `u128`, or `usize`, keeps
/// the generation next to a union of a `T` and a `U`. A `ParityUnion` with a
/// [`u1`](crate::u1) generation is an enum of a `T` and a `U` instead, so it
/// stores no generation, and it is often no larger than the larger of the two.
///
/// # Examples
///
/// ```
/// use gen_map::{Even, Odd, Parity, ParityUnion};
///
/// let one = Odd::new(1).unwrap();
///
/// // The `ParityUnion` starts out at generation zero, so it holds a `U`.
/// let mut parity_union = ParityUnion::<u32, u64, ()>::new(Parity::Even(Even::ZERO, ()));
/// assert!(parity_union.get_odd(one).is_none());
///
/// parity_union.replace(Parity::Odd(one, 10));
/// assert_eq!(parity_union.get_odd(one), Some(&10));
/// ```
// `repr(transparent)` gives a `ParityUnion` the layout of
// `G::ParityUnionRepr<T, U>`, so a `ParityUnion` takes no more space than its
// representation.
#[repr(transparent)]
pub struct ParityUnion<G: KeyPiece, T, U> {
    repr: G::ParityUnionRepr<T, U>,
}

impl<G: KeyPiece, T, U> ParityUnion<G, T, U> {
    /// Creates a `ParityUnion` with the generation and the value in `parity`.
    #[inline]
    pub fn new(parity: Parity<G, T, U>) -> Self {
        match parity {
            Parity::Odd(generation, value) => Self::new_odd(generation, value),
            Parity::Even(generation, value) => Self::new_even(generation, value),
        }
    }

    /// Creates a `ParityUnion` with an odd `generation` and a `T`.
    #[inline]
    pub fn new_odd(generation: Odd<G>, value: T) -> Self {
        Self {
            repr: <G::ParityUnionRepr<T, U> as Repr<G, T, U>>::new_odd(generation, value),
        }
    }

    /// Creates a `ParityUnion` with an even `generation` and a `U`.
    #[inline]
    pub fn new_even(generation: Even<G>, value: U) -> Self {
        Self {
            repr: <G::ParityUnionRepr<T, U> as Repr<G, T, U>>::new_even(generation, value),
        }
    }

    /// Returns the generation.
    #[inline]
    pub fn generation(&self) -> G {
        self.repr.generation()
    }

    /// Returns `true` if the generation is odd, which means the `ParityUnion`
    /// holds a `T`.
    #[inline]
    pub fn is_odd(&self) -> bool {
        self.repr.is_odd()
    }

    /// Returns `true` if the generation is even, which means the `ParityUnion`
    /// holds a `U`.
    #[inline]
    pub fn is_even(&self) -> bool {
        !self.repr.is_odd()
    }

    /// Returns references to the generation and the value.
    #[inline]
    pub fn as_parity(&self) -> ParityRef<'_, G, T, U> {
        self.repr.as_parity()
    }

    /// Returns mutable references to the generation and the value.
    #[inline]
    pub fn as_parity_mut(&mut self) -> ParityMut<'_, G, T, U> {
        self.repr.as_parity_mut()
    }

    /// Takes the generation and the value out of the `ParityUnion`.
    #[inline]
    pub fn into_parity(self) -> Parity<G, T, U> {
        self.repr.into_parity()
    }

    /// Returns a reference to the `T` if the generation of the `ParityUnion`
    /// is `generation`.
    #[inline]
    pub fn get_odd(&self, generation: Odd<G>) -> Option<&T> {
        if self.generation() == G::from_non_zero(generation.get()) {
            // SAFETY: the generation is odd, so the `ParityUnion` holds a `T`.
            Some(unsafe { self.repr.get_odd_unchecked() })
        } else {
            None
        }
    }

    /// Returns a mutable reference to the `T` if the generation of the
    /// `ParityUnion` is `generation`.
    #[inline]
    pub fn get_odd_mut(&mut self, generation: Odd<G>) -> Option<&mut T> {
        if self.generation() == G::from_non_zero(generation.get()) {
            // SAFETY: the generation is odd, so the `ParityUnion` holds a `T`.
            Some(unsafe { self.repr.get_odd_unchecked_mut() })
        } else {
            None
        }
    }

    /// Takes the `T` out of the `ParityUnion` if its generation is
    /// `generation`.
    ///
    /// # Errors
    ///
    /// Hands the `ParityUnion` back if its generation is not `generation`.
    #[inline]
    pub fn into_odd(self, generation: Odd<G>) -> Result<T, Self> {
        if self.generation() == G::from_non_zero(generation.get()) {
            match self.into_parity() {
                Parity::Odd(_, value) => Ok(value),
                // SAFETY: the generation is odd, so `into_parity` returns
                // `Parity::Odd`.
                Parity::Even(..) => unsafe { unreachable_unchecked() },
            }
        } else {
            Err(self)
        }
    }

    /// Returns a reference to the `U` if the generation of the `ParityUnion`
    /// is `generation`.
    #[inline]
    pub fn get_even(&self, generation: Even<G>) -> Option<&U> {
        if self.generation() == G::from_non_max(generation.get()) {
            // SAFETY: the generation is even, so the `ParityUnion` holds a `U`.
            Some(unsafe { self.repr.get_even_unchecked() })
        } else {
            None
        }
    }

    /// Returns a mutable reference to the `U` if the generation of the
    /// `ParityUnion` is `generation`.
    #[inline]
    pub fn get_even_mut(&mut self, generation: Even<G>) -> Option<&mut U> {
        if self.generation() == G::from_non_max(generation.get()) {
            // SAFETY: the generation is even, so the `ParityUnion` holds a `U`.
            Some(unsafe { self.repr.get_even_unchecked_mut() })
        } else {
            None
        }
    }

    /// Takes the `U` out of the `ParityUnion` if its generation is
    /// `generation`.
    ///
    /// # Errors
    ///
    /// Hands the `ParityUnion` back if its generation is not `generation`.
    #[inline]
    pub fn into_even(self, generation: Even<G>) -> Result<U, Self> {
        if self.generation() == G::from_non_max(generation.get()) {
            match self.into_parity() {
                Parity::Even(_, value) => Ok(value),
                // SAFETY: the generation is even, so `into_parity` returns
                // `Parity::Even`.
                Parity::Odd(..) => unsafe { unreachable_unchecked() },
            }
        } else {
            Err(self)
        }
    }

    /// Puts the generation and the value in `parity` into the `ParityUnion`,
    /// and returns the ones it had before.
    #[inline]
    pub fn replace(&mut self, parity: Parity<G, T, U>) -> Parity<G, T, U> {
        mem::replace(self, Self::new(parity)).into_parity()
    }

    /// Puts an odd `generation` and a `T` into the `ParityUnion`, and returns
    /// the generation and the value it had before.
    #[inline]
    pub fn set_odd(&mut self, generation: Odd<G>, value: T) -> Parity<G, T, U> {
        mem::replace(self, Self::new_odd(generation, value)).into_parity()
    }

    /// Puts an even `generation` and a `U` into the `ParityUnion`, and returns
    /// the generation and the value it had before.
    #[inline]
    pub fn set_even(&mut self, generation: Even<G>, value: U) -> Parity<G, T, U> {
        mem::replace(self, Self::new_even(generation, value)).into_parity()
    }

    /// Moves the `T` out of the `ParityUnion` and puts an even `generation`
    /// and a `U` in its place.
    ///
    /// # Safety
    ///
    /// The current generation must be odd.
    #[inline]
    pub unsafe fn replace_odd_unchecked(&mut self, generation: Even<G>, value: U) -> T {
        debug_assert!(self.is_odd());
        // SAFETY: the caller promises an odd generation.
        unsafe { self.repr.replace_odd_unchecked(generation, value) }
    }

    /// Moves the `U` out of the `ParityUnion` and puts an odd `generation` and
    /// a `T` in its place.
    ///
    /// # Safety
    ///
    /// The current generation must be even.
    #[inline]
    pub unsafe fn replace_even_unchecked(&mut self, generation: Odd<G>, value: T) -> U {
        debug_assert!(self.is_even());
        // SAFETY: the caller promises an even generation.
        unsafe { self.repr.replace_even_unchecked(generation, value) }
    }

    /// Returns a reference to the `T` without checking the generation.
    ///
    /// # Safety
    ///
    /// The generation must be odd.
    #[inline]
    pub unsafe fn get_odd_unchecked(&self) -> &T {
        debug_assert!(self.is_odd());
        // SAFETY: the caller promises an odd generation.
        unsafe { self.repr.get_odd_unchecked() }
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
        // SAFETY: the caller promises an odd generation.
        unsafe { self.repr.get_odd_unchecked_mut() }
    }

    /// Returns a reference to the `U` without checking the generation.
    ///
    /// # Safety
    ///
    /// The generation must be even.
    #[inline]
    pub unsafe fn get_even_unchecked(&self) -> &U {
        debug_assert!(self.is_even());
        // SAFETY: the caller promises an even generation.
        unsafe { self.repr.get_even_unchecked() }
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
        // SAFETY: the caller promises an even generation.
        unsafe { self.repr.get_even_unchecked_mut() }
    }
}

// The representation of a `ParityUnion` comes from `G`, so without the impls
// below, the compiler could not prove in generic code that a `ParityUnion` has
// these traits when `T` and `U` have them. Every representation holds only a
// `T` or a `U` and a generation, which it stores as `G` itself or in its
// `NonZero` or `NonMax` form. `G` and both forms have all of these traits, so
// each impl holds whenever its bounds on `T` and `U` hold.

// SAFETY: a `ParityUnion` holds only a `T` or a `U`, which are `Send` here, and
// a generation, which is `Send` in every form a representation stores it in.
unsafe impl<G: KeyPiece, T: Send, U: Send> Send for ParityUnion<G, T, U> {}

// SAFETY: a `ParityUnion` holds only a `T` or a `U`, which are `Sync` here, and
// a generation, which is `Sync` in every form a representation stores it in.
// Every method that takes `&self` only reads them.
unsafe impl<G: KeyPiece, T: Sync, U: Sync> Sync for ParityUnion<G, T, U> {}

impl<G: KeyPiece, T: Unpin, U: Unpin> Unpin for ParityUnion<G, T, U> {}

impl<G: KeyPiece, T: UnwindSafe, U: UnwindSafe> UnwindSafe for ParityUnion<G, T, U> {}

impl<G: KeyPiece, T: RefUnwindSafe, U: RefUnwindSafe> RefUnwindSafe for ParityUnion<G, T, U> {}

/// A `ParityUnion` has `Send`, `Sync`, `Unpin` and `UnwindSafe` only when both
/// its `T` and its `U` have them. Each example below must fail to compile.
///
/// ```compile_fail,E0277
/// # use gen_map::{u1, ParityUnion};
/// fn assert_send<S: Send>() {}
/// assert_send::<ParityUnion<u1, std::rc::Rc<()>, ()>>();
/// ```
///
/// ```compile_fail,E0277
/// # use gen_map::ParityUnion;
/// fn assert_sync<S: Sync>() {}
/// assert_sync::<ParityUnion<u32, (), std::cell::Cell<()>>>();
/// ```
///
/// ```compile_fail,E0277
/// # use gen_map::{u1, ParityUnion};
/// fn assert_unpin<S: Unpin>() {}
/// assert_unpin::<ParityUnion<u1, (), core::marker::PhantomPinned>>();
/// ```
///
/// ```compile_fail,E0277
/// # use gen_map::ParityUnion;
/// fn assert_unwind_safe<S: core::panic::UnwindSafe>() {}
/// assert_unwind_safe::<ParityUnion<u32, &mut i32, ()>>();
/// ```
#[cfg(doctest)]
struct ParityUnionTraitsNeedBothValues;

impl<G: KeyPiece, T: Clone, U: Clone> Clone for ParityUnion<G, T, U> {
    #[inline]
    fn clone(&self) -> Self {
        match self.as_parity() {
            ParityRef::Odd(&generation, value) => Self::new_odd(generation, value.clone()),
            ParityRef::Even(&generation, value) => Self::new_even(generation, value.clone()),
        }
    }
}

impl<G: KeyPiece, T: fmt::Debug, U: fmt::Debug> fmt::Debug for ParityUnion<G, T, U> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value: &dyn fmt::Debug = match self.as_parity() {
            ParityRef::Odd(_, value) => value,
            ParityRef::Even(_, value) => value,
        };
        f.debug_struct("ParityUnion")
            .field("generation", &self.generation())
            .field("value", value)
            .finish()
    }
}

/// A `ParityRef` holds shared references to the generation and the value of a
/// [`ParityUnion`], and [`ParityUnion::as_parity`] returns one. Its variant
/// says whether the generation is odd or even, the same way the variant of a
/// [`Parity`] does.
#[derive(Debug, PartialEq, Eq, Hash)]
pub enum ParityRef<'a, G: KeyPiece, T, U> {
    /// The generation is odd, as in [`Parity::Odd`].
    Odd(&'a Odd<G>, &'a T),
    /// The generation is even, as in [`Parity::Even`].
    Even(&'a Even<G>, &'a U),
}

// Deriving `Clone` and `Copy` would require the value types to implement them,
// but the references a `ParityRef` holds can always be copied.
impl<G: KeyPiece, T, U> Clone for ParityRef<'_, G, T, U> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<G: KeyPiece, T, U> Copy for ParityRef<'_, G, T, U> {}

/// A `ParityMut` holds mutable references to the generation and the value of a
/// [`ParityUnion`], and [`ParityUnion::as_parity_mut`] returns one. Its
/// variant says whether the generation is odd or even, the same way the
/// variant of a [`Parity`] does. Code that writes through the reference to the
/// generation can change the generation but not its parity, because an [`Odd`]
/// is always odd and an [`Even`] is always even.
#[derive(Debug, PartialEq, Eq, Hash)]
pub enum ParityMut<'a, G: KeyPiece, T, U> {
    /// The generation is odd, as in [`Parity::Odd`].
    Odd(&'a mut Odd<G>, &'a mut T),
    /// The generation is even, as in [`Parity::Even`].
    Even(&'a mut Even<G>, &'a mut U),
}
