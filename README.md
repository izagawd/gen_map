# gen_map

A customizable generational map that returns a `Key` upon inserting a value. The key can be used to later access or remove the value, and removing a value bumps its slot's generation, so the old key no longer matches.
The operations for inserting, removing and accessing a value are all O(1).

The crate is `no_std`.

```toml
[dependencies]
gen_map = "0.2.2"
```

## Example

```rust
use gen_map::GenMap;

let mut map = GenMap::new();
let a = map.insert("a");
let b = map.insert("b");

assert_eq!(map[a], "a");
assert_eq!(map[b], "b");
assert_eq!(map.remove(a), Some("a"));
assert!(map.get(a).is_none()); // A removed key never matches again.

let c = map.insert("c"); // This takes the slot `a` had, but under a new key.
assert_ne!(a, c);

for (key, value) in &map {
    println!("{key:?} = {value}");
}
```

## Configuring the map

A `KeyConfig` picks the index and generation integers and how a key stores
the two. A `MapConfig` picks the key config, where the slots live and what
happens when a slot's generation runs out. It is implemented for slot types,
usually for every `SlotItem` at once, so it can put bounds on the slots or on
the values they hold. Maps whose configs pick the same
key config share a key type. The default uses a `u32` index and a `u32`
generation stored as two fields, keeps the slots in a `Vec`, and retires a
slot whose generation runs out, so no stale key can ever match a new value.

```rust
use gen_map::{GenMap, KeyConfig, MapConfig, SlotItem, Split};

struct TinyKey;

impl KeyConfig for TinyKey {
    type Idx = u8;
    type Gen = u8;
    type Layout = Split;
}

struct TinyMap;

impl<S: SlotItem> MapConfig<S> for TinyMap {
    type KeyConfig = TinyKey;
    type Storage = Vec<S>;
    // A slot whose generation runs out is reused instead of retired. The
    // default is to retire it.
    const WRAP_ON_OVERFLOW: bool = true;
}

let mut map = GenMap::<u64, TinyMap>::new_with_config();
let key = map.insert(7);
assert_eq!(core::mem::size_of_val(&key), 2);
```

`Option<Key>` is the same size as `Key` with either layout.

### Packing the key into one integer

`Split` stores the index and the generation as two fields. `Packed` puts
them in the bits of one integer instead, so the two parts can have any bit
counts that add up to that integer. The low `GEN_BITS` bits hold the
generation and the bits above them hold the index. A key config whose bit
counts do not add up fails to compile.

```rust
use gen_map::{GenMap, KeyConfig, MapConfig, Packed, SlotItem};

/// Four byte keys with 24 bits of index and 8 bits of generation.
struct CompactKey;

impl KeyConfig for CompactKey {
    type Idx = u32;
    type Gen = u8;
    type Layout = Packed<u32, 8>;
}

struct CompactMap;

impl<S: SlotItem> MapConfig<S> for CompactMap {
    type KeyConfig = CompactKey;
    type Storage = Vec<S>;
}

let mut map = GenMap::<&str, CompactMap>::new_with_config();
let key = map.insert("a");
assert_eq!(core::mem::size_of_val(&key), 4);
assert_eq!(key.idx(), 0);
assert_eq!(key.generation().get().get(), 1);
```

### Choosing the storage

The slots can live in any collection that implements `SlotStorage`, even
those with a fixed capacity. Besides `Vec`, the `ArrayVec` from `arrayvec`
and the `SmallVec` from `smallvec` work out of the box when the crate's
`arrayvec` or `smallvec` feature is turned on. An `ArrayVec` has a fixed
capacity and never allocates, and a `SmallVec` keeps a few slots inline
before it allocates.

The `smallvec` feature uses a beta of smallvec 2.0. Until smallvec 2.0 is
released, a newer smallvec beta or a new release of gen_map may break this
feature, so it is not covered by semver.

`reserve`, `try_reserve` and the `with_capacity` constructors are only there
when the storage also implements `ReserveStorage`, which `Vec` and
`SmallVec` do.

## Handling a full map

`insert` panics when the map is full, which happens when the keys have no
index left for a new slot or the storage cannot make room for one.
`try_insert` lets you handle the failure.

```rust
use gen_map::{GenMap, InsertError, KeyConfig, MapConfig, SlotItem, Split};

struct TinyKey;

impl KeyConfig for TinyKey {
    type Idx = u8;
    type Gen = u8;
    type Layout = Split;
}

struct TinyMap;

impl<S: SlotItem> MapConfig<S> for TinyMap {
    type KeyConfig = TinyKey;
    type Storage = Vec<S>;
}

let mut map = GenMap::<u32, TinyMap>::new_with_config();
for i in 0..256 {
    map.insert(i);
}
assert!(matches!(map.try_insert(256), Err(InsertError::IndexExhausted(256))));
```

## License

gen_map is released under the MIT license.