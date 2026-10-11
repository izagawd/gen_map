use crate::key::piece::KeyPiece;

/// This trait is used to choose whether a [`GenMap`](crate::GenMap) or a
/// [`SecondaryMap`](crate::SecondaryMap) keeps a count of its values. The
/// built-in choices are [`KeepLen`] and [`NoLen`].
///
/// It is sealed, so it cannot be implemented outside this crate.
pub trait Len: sealed::Sealed {
    /// The type the map keeps its count of values in, where `I` is the index
    /// type of the map's keys. It is `I` for [`KeepLen`] and `()` for
    /// [`NoLen`].
    type Count<I: KeyPiece>: sealed::Counter<I>;
}

/// A map whose config picks `KeepLen` keeps a count of its values. The map
/// has `len` and `is_empty`, and its iterators implement
/// `ExactSizeIterator`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct KeepLen;

impl sealed::Sealed for KeepLen {}

impl Len for KeepLen {
    type Count<I: KeyPiece> = I;
}

/// A map whose config picks `NoLen` keeps no count of its values. The map has
/// no `len` or `is_empty`, and its iterators do not implement
/// `ExactSizeIterator`.
///
/// # Examples
///
/// ```
/// use gen_map::{GenMap, GenMapConfig, GenSlotItem, MapConfig, NoLen, Split};
///
/// struct Uncounted;
///
/// impl MapConfig for Uncounted {
///     type KeyConfig = Split<u32, u32>;
/// }
///
/// impl GenMapConfig for Uncounted {
///     type Len = NoLen;
///     type Storage<S: GenSlotItem> = Vec<S>;
/// }
///
/// let mut map = GenMap::<&str, Uncounted>::new_with_config();
/// let a = map.insert("a");
/// map.insert("b");
/// map.remove(a);
/// assert_eq!(map.iter().count(), 1);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct NoLen;

impl sealed::Sealed for NoLen {}

impl Len for NoLen {
    type Count<I: KeyPiece> = ();
}

/// A map whose config picks [`NoLen`] has no `len` or `is_empty`, and its
/// iterators do not implement `ExactSizeIterator`. Each example below must
/// fail to compile.
///
/// ```compile_fail,E0599
/// # use gen_map::{GenMap, GenMapConfig, GenSlotItem, MapConfig, NoLen, Split};
/// # struct Uncounted;
/// # impl MapConfig for Uncounted {
/// #     type KeyConfig = Split<u32, u32>;
/// # }
/// # impl GenMapConfig for Uncounted {
/// #     type Len = NoLen;
/// #     type Storage<S: GenSlotItem> = Vec<S>;
/// # }
/// let map = GenMap::<u32, Uncounted>::new_with_config();
/// map.len();
/// ```
///
/// ```compile_fail,E0599
/// # use gen_map::{GenMap, GenMapConfig, GenSlotItem, MapConfig, NoLen, Split};
/// # struct Uncounted;
/// # impl MapConfig for Uncounted {
/// #     type KeyConfig = Split<u32, u32>;
/// # }
/// # impl GenMapConfig for Uncounted {
/// #     type Len = NoLen;
/// #     type Storage<S: GenSlotItem> = Vec<S>;
/// # }
/// let map = GenMap::<u32, Uncounted>::new_with_config();
/// map.is_empty();
/// ```
///
/// ```compile_fail,E0599
/// # use gen_map::{GenMap, GenMapConfig, GenSlotItem, MapConfig, NoLen, Split};
/// # struct Uncounted;
/// # impl MapConfig for Uncounted {
/// #     type KeyConfig = Split<u32, u32>;
/// # }
/// # impl GenMapConfig for Uncounted {
/// #     type Len = NoLen;
/// #     type Storage<S: GenSlotItem> = Vec<S>;
/// # }
/// let map = GenMap::<u32, Uncounted>::new_with_config();
/// map.iter().len();
/// ```
///
/// ```compile_fail,E0599
/// # use gen_map::{
/// #     MapConfig, NewerWins, NoLen, SecondaryMap, SecondaryMapConfig, SecondarySlotItem, Split,
/// # };
/// # struct Uncounted;
/// # impl MapConfig for Uncounted {
/// #     type KeyConfig = Split<u32, u32>;
/// # }
/// # impl SecondaryMapConfig for Uncounted {
/// #     type ReplaceStrategy = NewerWins;
/// #     type Len = NoLen;
/// #     type Storage<S: SecondarySlotItem> = Vec<S>;
/// # }
/// let map = SecondaryMap::<u32, Uncounted>::new_with_config();
/// map.len();
/// ```
///
/// ```compile_fail,E0599
/// # use gen_map::{
/// #     MapConfig, NewerWins, NoLen, SecondaryMap, SecondaryMapConfig, SecondarySlotItem, Split,
/// # };
/// # struct Uncounted;
/// # impl MapConfig for Uncounted {
/// #     type KeyConfig = Split<u32, u32>;
/// # }
/// # impl SecondaryMapConfig for Uncounted {
/// #     type ReplaceStrategy = NewerWins;
/// #     type Len = NoLen;
/// #     type Storage<S: SecondarySlotItem> = Vec<S>;
/// # }
/// let map = SecondaryMap::<u32, Uncounted>::new_with_config();
/// map.is_empty();
/// ```
///
/// ```compile_fail,E0599
/// # use gen_map::{
/// #     MapConfig, NewerWins, NoLen, SecondaryMap, SecondaryMapConfig, SecondarySlotItem, Split,
/// # };
/// # struct Uncounted;
/// # impl MapConfig for Uncounted {
/// #     type KeyConfig = Split<u32, u32>;
/// # }
/// # impl SecondaryMapConfig for Uncounted {
/// #     type ReplaceStrategy = NewerWins;
/// #     type Len = NoLen;
/// #     type Storage<S: SecondarySlotItem> = Vec<S>;
/// # }
/// let map = SecondaryMap::<u32, Uncounted>::new_with_config();
/// map.iter().len();
/// ```
#[cfg(doctest)]
struct NoLenHasNoCount;

pub(crate) mod sealed {
    use crate::key::piece::KeyPiece;

    pub trait Sealed {}

    /// A map keeps its count of values in a type that implements this trait,
    /// where `I` is the index type of the map's keys. A map that keeps no
    /// count stores `()` instead.
    pub trait Counter<I: KeyPiece>: Copy {
        /// Returns the count of a map with no values.
        fn zero() -> Self;

        /// Returns the count, or `None` if the map keeps no count.
        fn get(self) -> Option<I>;

        /// Replaces the count with what `f` returns for it. It does nothing
        /// if the map keeps no count.
        fn update(&mut self, f: impl FnOnce(I) -> I);
    }

    impl<I: KeyPiece> Counter<I> for I {
        #[inline]
        fn zero() -> Self {
            I::ZERO
        }

        #[inline]
        fn get(self) -> Option<I> {
            Some(self)
        }

        #[inline]
        fn update(&mut self, f: impl FnOnce(I) -> I) {
            *self = f(*self);
        }
    }

    impl<I: KeyPiece> Counter<I> for () {
        #[inline]
        fn zero() -> Self {}

        #[inline]
        fn get(self) -> Option<I> {
            None
        }

        #[inline]
        fn update(&mut self, _f: impl FnOnce(I) -> I) {}
    }
}
