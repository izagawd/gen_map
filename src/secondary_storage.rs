/// A collection of slots. A
/// [`SecondaryMapConfig`](crate::SecondaryMapConfig) chooses one with its
/// [`Storage`](crate::SecondaryMapConfig::Storage) type.
pub trait SecondarySlotStorage {
    /// The type of the slots in the collection.
    type Item;
}
