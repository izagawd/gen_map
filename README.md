# gen_map

A customizable generational map that returns a `Key` upon inserting a value. The key can be used to later access or remove the value, and removing a value bumps its slot's generation, so the old key no longer matches.
The operations for inserting, removing and accessing a value are all O(1).

The crate never uses `std`, so it also works on targets that do not have it.

```toml
[dependencies]
gen_map = "0.3"
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

A `KeyConfig` picks the key's index and generation types and how the key
stores them. `Split<Idx, Gen>` keeps the two as separate fields, and
`Packed<R, GEN_BITS>` puts them in the bits of one integer and picks the
smallest types that can hold them. A `MapConfig` is used to decide the key
config, and a `GenMapConfig` is used to decide what happens when a slot's
generation runs out and where the slots live.

```rust
use gen_map::{GenMap, GenMapConfig, GenSlotItem, MapConfig, Packed};

/// Maps with this config hand out four byte keys with 24 bits of index and 8
/// bits of generation.
struct CompactConfig;

impl MapConfig for CompactConfig {
    type KeyConfig = Packed<u32, 8>;
}

// `S` is the slot the map keeps each value in.
impl<S: GenSlotItem> GenMapConfig<S> for CompactConfig {
    type Storage = Vec<S>;
}

let mut map = GenMap::<&str, CompactConfig>::new_with_config();
let key = map.insert("a");
assert_eq!(core::mem::size_of_val(&key), 4);
```

The [documentation](https://docs.rs/gen_map) covers the rest, such as key
configs, storage, what happens when a generation runs out, and limiting which
maps can use a config.

## Secondary maps

A `SecondaryMap` stores values under the keys a `GenMap` hands out, to add
data to a `GenMap`'s values without changing their type. Like a `GenMap`, it
can be configured, and the [documentation](https://docs.rs/gen_map) covers
how it can be configured.

```rust
use gen_map::{GenMap, SecondaryMap};

let mut people = GenMap::new();
let mut ages = SecondaryMap::new();
let alice = people.insert("Alice");
ages.insert(alice, 30).unwrap();
assert_eq!(ages[alice], 30);
```

## Cargo features

- `alloc` is on by default. It adds the `Vec` storage and the default
  config. Turn default features off and use `arrayvec` instead to run
  without an allocator.
- `arrayvec` adds `ArrayVec` storage for a `GenMap` or a `SecondaryMap`. An
  `ArrayVec` has a fixed capacity and never allocates.
- `smallvec` adds `SmallVec` storage for a `GenMap` or a `SecondaryMap`. A
  `SmallVec` keeps a few slots inline before it allocates. The feature uses a
  beta of smallvec 2.0, so it is not covered by semver.

## License

gen_map is released under the MIT license.