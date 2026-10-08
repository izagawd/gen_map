use crate::storage::{ReserveStorage, SliceStorage};
use core::fmt;
use core::iter::FusedIterator;

/// The collection a dense map keeps its keys and values in. A pair storage
/// holds pairs of items. It keeps the first item of every pair in one
/// slice, and the second item of every pair at the same position in
/// another slice.
///
/// A [`DenseGenMapConfig`](crate::DenseGenMapConfig) or a
/// [`DenseSecondaryMapConfig`](crate::DenseSecondaryMapConfig) chooses one
/// with its `PairStorage` type. The map keeps its values one after another in
/// the second slice, and the key of each value at the same position in the
/// first slice.
///
/// [`PairVec`](crate::PairVec) keeps the two slices in one buffer, with one
/// length and one capacity for both. [`SplitPair`] keeps each slice in a
/// [`SliceStorage`] of its own, so it works with any slice storage, such as an
/// `ArrayVec`.
///
/// A pair storage whose capacity can grow past what it was created with also
/// implements the [`ReservePairStorage`] marker. A pair storage that
/// implements `IntoIterator` gives a dense map an owning `into_iter`. The
/// iterator that `into_iter` returns implements `DoubleEndedIterator` when the
/// storage's iterator implements both `DoubleEndedIterator` and
/// `ExactSizeIterator`.
///
/// # Safety
///
/// A dense map reads its keys and values without bounds checks at the
/// positions its slots store, and it reads the key at a position to find the
/// slot of the value at the same position. So a pair storage must behave like
/// two `Vec`s that always change together, in the ways listed below.
///
/// - [`slices`](Self::slices) and [`slices_mut`](Self::slices_mut) must
///   return exactly the pairs pushed with [`try_push`](Self::try_push) or
///   [`push_unchecked`](Self::push_unchecked) since the last
///   [`clear`](Self::clear) and not taken out by [`pop`](Self::pop) since,
///   in the order they were pushed, and no others. The first item of
///   each pair must sit in the first slice, and the second item must sit at
///   the same position in the second slice. The other slice methods, as
///   well as [`len`](Self::len) and [`is_empty`](Self::is_empty), must agree
///   with them, as the provided methods do.
/// - Apart from `pop` and `clear`, and dropping the storage itself, no method/function
///   implemented in this trait may remove, drop, replace or change an item.
///   An item only mutates through the slice methods.
///   Growing may move the items in memory, but must keep them in the same order and indices.
/// - `try_push` must either append both items and return `Ok`, or hand both
///   items back in `Err` and leave the storage as it was.
/// - `pop` must take out the last pair and return it, or return `None` and
///   leave the storage as it was if it has no pairs.
/// - Once [`ensure_room`](Self::ensure_room) has returned `Ok` for `n` pairs,
///   the next `n` pushes must succeed, as long as no other `&mut self` method
///   of this trait runs in between. A push is a call of `try_push` or
///   `push_unchecked`.
/// - When another storage of the same type holds `n` pairs, the first `n`
///   pushes into a storage that [`with_capacity`](Self::with_capacity)
///   returns for `n` pairs must succeed, as long as no other `&mut self`
///   method of this trait runs in between. The dense maps rely on this when
///   they clone a storage.
/// - When the two rules above promise that a push succeeds, `push_unchecked`
///   must append both items, as `try_push` does.
/// - `clear` must drop every item and leave the storage empty. It must leave
///   the storage empty even when dropping an item panics.
/// - Only [`with_capacity`](Self::with_capacity), `ensure_room` and `clear`
///   may panic, and `clear` only when dropping an item panics. A dense map
///   calls the other methods partway through changes that a panic would leave
///   half done.
/// - [`empty`](Self::empty) and `with_capacity` must return a storage with no
///   pairs.
/// - If the storage implements `IntoIterator<Item = (Self::First,
///   Self::Second)>`, `into_iter` must yield the same pairs as `slices`, in
///   the same order.
/// - If that iterator also implements `DoubleEndedIterator` and
///   `ExactSizeIterator`, `next_back` must yield the pairs starting from the
///   last one, and `len` must be the number of pairs not yet yielded.
pub unsafe trait PairStorage {
    /// The type of the items in the first slice. A dense map keeps its keys
    /// in this slice.
    type First;

    /// The type of the items in the second slice. A dense map keeps its
    /// values in this slice.
    type Second;

    /// Why the storage could not make room for another pair.
    type Error: fmt::Debug;

    /// Creates a storage with no pairs.
    fn empty() -> Self;

    /// Creates a storage with no pairs and room for at least `capacity` of
    /// them. A storage whose type limits how many pairs it can hold can
    /// ignore the argument.
    fn with_capacity(capacity: usize) -> Self;

    /// How many pairs the storage can hold before it has to grow, or in total
    /// if it cannot grow.
    fn capacity(&self) -> usize;

    /// Returns the first item of every pair as a slice, in the order the
    /// pairs were pushed.
    #[inline]
    fn first_slice(&self) -> &[Self::First] {
        self.slices().0
    }

    /// Returns the second item of every pair as a slice, in the order the
    /// pairs were pushed.
    #[inline]
    fn second_slice(&self) -> &[Self::Second] {
        self.slices().1
    }

    /// Returns the first and the second item of every pair as two slices. The
    /// two items at the same position in the two slices belong to the same
    /// pair.
    fn slices(&self) -> (&[Self::First], &[Self::Second]);

    /// Returns the first item of every pair as a mutable slice, in the order
    /// the pairs were pushed.
    #[inline]
    fn first_slice_mut(&mut self) -> &mut [Self::First] {
        self.slices_mut().0
    }

    /// Returns the second item of every pair as a mutable slice, in the order
    /// the pairs were pushed.
    #[inline]
    fn second_slice_mut(&mut self) -> &mut [Self::Second] {
        self.slices_mut().1
    }

    /// Returns the first and the second item of every pair as two mutable
    /// slices. The two items at the same position in the two slices belong to
    /// the same pair.
    fn slices_mut(&mut self) -> (&mut [Self::First], &mut [Self::Second]);

    /// Makes sure the next `additional` calls of [`try_push`](Self::try_push)
    /// or [`push_unchecked`](Self::push_unchecked) will succeed, growing if
    /// the storage can and has to. A dense map passes on the room its
    /// `try_reserve` is asked for, so `additional` can be larger than any
    /// storage can hold. The map expects an error in that case, not a panic.
    ///
    /// # Errors
    ///
    /// Returns the reason the pushes would fail if the storage cannot make
    /// room for all of them.
    fn ensure_room(&mut self, additional: usize) -> Result<(), Self::Error>;

    /// Appends `first` to the first slice and `second` to the second slice, as
    /// one pair.
    ///
    /// # Errors
    ///
    /// Hands both items back if the storage cannot make room for them.
    fn try_push(
        &mut self,
        first: Self::First,
        second: Self::Second,
    ) -> Result<(), (Self::First, Self::Second)>;

    /// Appends `first` to the first slice and `second` to the second slice, as
    /// one pair, without checking that the storage has room for them. The
    /// provided implementation calls [`try_push`](Self::try_push) and assumes
    /// it succeeds. A storage can override it with one that does not check
    /// for room at all.
    ///
    /// # Safety
    ///
    /// The rules of this trait must promise that this push succeeds.
    #[inline]
    unsafe fn push_unchecked(&mut self, first: Self::First, second: Self::Second) {
        // SAFETY: the caller promises that the rules of the trait make this
        // push succeed, so `try_push` returns `Ok`.
        unsafe { self.try_push(first, second).unwrap_unchecked() };
    }

    /// Takes out the last pair and returns it, or returns `None` if there are
    /// no pairs.
    fn pop(&mut self) -> Option<(Self::First, Self::Second)>;

    /// Drops every pair, which leaves the storage empty.
    fn clear(&mut self);

    /// The number of pairs.
    #[inline]
    fn len(&self) -> usize {
        self.first_slice().len()
    }

    /// Returns `true` if there are no pairs.
    #[inline]
    fn is_empty(&self) -> bool {
        self.first_slice().is_empty()
    }
}

/// Marks a [`PairStorage`] whose capacity can grow past what it was created
/// with. [`ensure_room`](PairStorage::ensure_room) grows such a storage when
/// it needs more room. A dense map has `with_capacity_and_config`, `reserve`
/// and `try_reserve` only when its pair storage implements this trait and its
/// slot storage implements [`ReserveStorage`].
pub trait ReservePairStorage: PairStorage {}

/// A [`PairStorage`] that keeps each slice in a [`SliceStorage`] of its own.
/// The first storage holds the first slice, and the second storage holds the
/// second slice. It lets a dense map keep its keys and values in any slice
/// storage, such as an `ArrayVec` that never allocates.
///
/// Each of the two storages keeps a length and a capacity of its own. The
/// capacity of a `SplitPair` is the smaller of the two capacities. A
/// `SplitPair` implements [`ReservePairStorage`] when both storages implement
/// [`ReserveStorage`].
///
/// # Examples
///
/// ```
/// use gen_map::{DenseGenMap, DenseGenMapConfig, GenSlotItem, MapConfig, Split, SplitPair};
///
/// /// Maps with this config keep their keys in one `Vec` and their values in
/// /// another.
/// struct TwoVecs;
///
/// impl MapConfig for TwoVecs {
///     type KeyConfig = Split<u32, u32>;
/// }
///
/// impl DenseGenMapConfig for TwoVecs {
///     type SlotStorage<S: GenSlotItem> = Vec<S>;
///     type PairStorage<K, V> = SplitPair<Vec<K>, Vec<V>>;
/// }
///
/// let mut map = DenseGenMap::<&str, TwoVecs>::new_with_config();
/// let key = map.insert("a");
/// assert_eq!(map[key], "a");
/// ```
#[derive(Debug)]
pub struct SplitPair<A, B> {
    first: A,
    second: B,
}

impl<A: SliceStorage, B: SliceStorage> SplitPair<A, B> {
    /// Puts two storages together as the two slices of a pair storage, or
    /// returns `None` if they hold different numbers of items. The two items
    /// at the same position in the two storages become one pair.
    #[inline]
    pub fn from_parts(first: A, second: B) -> Option<Self> {
        (first.len() == second.len()).then_some(Self { first, second })
    }

    /// Takes the pair storage apart into the storage of its first slice and
    /// the storage of its second slice.
    #[inline]
    pub fn into_parts(self) -> (A, B) {
        (self.first, self.second)
    }
}

// SAFETY: each slice is kept in a `SliceStorage`, which behaves like a
// `Vec`, and every method keeps the two storages at the same length.
// `from_parts` only accepts two storages of the same length, and `try_push`
// takes the first item back out when the second storage refuses its item.
// Neither storage panics outside `with_capacity`, `ensure_room` and `clear`,
// so `try_push` and `pop` never stop after changing only one storage, and no
// other method panics either. When another `SplitPair` of the same type holds
// `n` pairs, each of its storages holds `n` items, so both storages that
// `with_capacity(n)` makes accept the first `n` pushes. `push_unchecked`
// pushes into both storages without a check, and the rules only promise that
// a pair push succeeds when they promise it for both storages. `clear` empties
// the second storage even when dropping an item of the first one panics.
unsafe impl<A: SliceStorage, B: SliceStorage> PairStorage for SplitPair<A, B> {
    type First = A::Item;
    type Second = B::Item;
    type Error = SplitPairError<A::Error, B::Error>;

    #[inline]
    fn empty() -> Self {
        Self {
            first: A::empty(),
            second: B::empty(),
        }
    }

    #[inline]
    fn with_capacity(capacity: usize) -> Self {
        Self {
            first: A::with_capacity(capacity),
            second: B::with_capacity(capacity),
        }
    }

    #[inline]
    fn capacity(&self) -> usize {
        self.first.capacity().min(self.second.capacity())
    }

    #[inline]
    fn first_slice(&self) -> &[A::Item] {
        self.first.as_slice()
    }

    #[inline]
    fn second_slice(&self) -> &[B::Item] {
        self.second.as_slice()
    }

    #[inline]
    fn slices(&self) -> (&[A::Item], &[B::Item]) {
        (self.first.as_slice(), self.second.as_slice())
    }

    #[inline]
    fn first_slice_mut(&mut self) -> &mut [A::Item] {
        self.first.as_mut_slice()
    }

    #[inline]
    fn second_slice_mut(&mut self) -> &mut [B::Item] {
        self.second.as_mut_slice()
    }

    #[inline]
    fn slices_mut(&mut self) -> (&mut [A::Item], &mut [B::Item]) {
        (self.first.as_mut_slice(), self.second.as_mut_slice())
    }

    #[inline]
    fn ensure_room(&mut self, additional: usize) -> Result<(), Self::Error> {
        self.first
            .ensure_room(additional)
            .map_err(SplitPairError::First)?;
        self.second
            .ensure_room(additional)
            .map_err(SplitPairError::Second)
    }

    #[inline]
    fn try_push(&mut self, first: A::Item, second: B::Item) -> Result<(), (A::Item, B::Item)> {
        if let Err(first) = self.first.try_push(first) {
            return Err((first, second));
        }
        match self.second.try_push(second) {
            Ok(()) => Ok(()),
            // The first storage took `first` just before, so `pop` hands it
            // back and leaves that storage as it was. Only a broken storage
            // returns `None` here.
            Err(second) => match self.first.pop() {
                Some(first) => Err((first, second)),
                None => panic!("SliceStorage::pop returned None right after a push succeeded"),
            },
        }
    }

    #[inline]
    unsafe fn push_unchecked(&mut self, first: A::Item, second: B::Item) {
        // SAFETY: the caller promises that the rules of the trait make this
        // push succeed, and a `SplitPair` only promises that when the rules of
        // both storages promise that their next push succeeds.
        unsafe {
            self.first.push_unchecked(first);
            self.second.push_unchecked(second);
        }
    }

    #[inline]
    fn pop(&mut self) -> Option<(A::Item, B::Item)> {
        // The two storages always hold the same number of items, so either
        // both have a last item or neither does.
        Some((self.first.pop()?, self.second.pop()?))
    }

    fn clear(&mut self) {
        /// Clears the storage of the second slice when it goes out of scope,
        /// which also happens while a panic from clearing the storage of the
        /// first slice unwinds.
        struct ClearOnDrop<'a, S: SliceStorage>(&'a mut S);

        impl<S: SliceStorage> Drop for ClearOnDrop<'_, S> {
            fn drop(&mut self) {
                self.0.clear();
            }
        }

        let _second = ClearOnDrop(&mut self.second);
        self.first.clear();
    }

    #[inline]
    fn len(&self) -> usize {
        self.first.len()
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.first.is_empty()
    }
}

impl<A: ReserveStorage, B: ReserveStorage> ReservePairStorage for SplitPair<A, B> {}

/// This error says which storage of a [`SplitPair`] could not make room, and
/// it holds that storage's error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SplitPairError<A, B> {
    /// The storage of the first slice could not make room.
    First(A),
    /// The storage of the second slice could not make room.
    Second(B),
}

impl<A: fmt::Display, B: fmt::Display> fmt::Display for SplitPairError<A, B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::First(error) => write!(f, "the storage of the first slice is full: {error}"),
            Self::Second(error) => write!(f, "the storage of the second slice is full: {error}"),
        }
    }
}

impl<A, B> IntoIterator for SplitPair<A, B>
where
    A: SliceStorage + IntoIterator<Item = <A as SliceStorage>::Item>,
    B: SliceStorage + IntoIterator<Item = <B as SliceStorage>::Item>,
{
    type Item = (<A as SliceStorage>::Item, <B as SliceStorage>::Item);
    type IntoIter = SplitPairIntoIter<A, B>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        SplitPairIntoIter {
            remaining: self.first.len(),
            first: self.first.into_iter(),
            second: self.second.into_iter(),
        }
    }
}

/// Owning iterator over the pairs of a [`SplitPair`], in the order they were
/// pushed. It is created by consuming a `SplitPair` with `into_iter`, which a
/// `SplitPair` only has when both of its storages implement `IntoIterator`. It
/// implements `DoubleEndedIterator` when the iterators of both storages
/// implement `DoubleEndedIterator` and `ExactSizeIterator`.
pub struct SplitPairIntoIter<A: IntoIterator, B: IntoIterator> {
    first: A::IntoIter,
    second: B::IntoIter,
    /// The number of pairs not yet yielded.
    remaining: usize,
}

impl<A: IntoIterator, B: IntoIterator> Iterator for SplitPairIntoIter<A, B> {
    type Item = (A::Item, B::Item);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let pair = (self.first.next()?, self.second.next()?);
        self.remaining -= 1;
        Some(pair)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<A: IntoIterator, B: IntoIterator> DoubleEndedIterator for SplitPairIntoIter<A, B>
where
    A::IntoIter: DoubleEndedIterator + ExactSizeIterator,
    B::IntoIter: DoubleEndedIterator + ExactSizeIterator,
{
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let pair = (self.first.next_back()?, self.second.next_back()?);
        self.remaining -= 1;
        Some(pair)
    }
}

impl<A: IntoIterator, B: IntoIterator> ExactSizeIterator for SplitPairIntoIter<A, B> {}
impl<A: IntoIterator, B: IntoIterator> FusedIterator for SplitPairIntoIter<A, B> {}
