# gen_map

A customizable generational map that returns a `Key` upon inserting a value. The key can be used to later access or remove the value, and removing a value bumps its slot's generation, so the old key no longer matches.
The operations for inserting, removing and accessing a value are all O(1).

```toml
[dependencies]
gen_map = "0.1"
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
assert!(map.get(a).is_none()); // A removed key stays invalid forever.

let c = map.insert("c"); // This takes the slot `a` had, but under a new key.
assert_ne!(a, c);

for (key, value) in &map {
    println!("{key:?} = {value}");
}
```

## Configuring the key

A `Config` picks the index and generation integers and what happens when a
slot's generation overflows. The default uses a `u32` index and a `u32`
generation, and a slot whose generation overflows is retired, so no stale key
can ever match a new value.

```rust
use gen_map::{Config, GenMap};

struct Tiny;

impl Config for Tiny {
    type Idx = u8;
    type Gen = u8;
    // A slot whose generation overflows is reused instead of retired. The
    // default is to retire it.
    const WRAP_ON_OVERFLOW: bool = true;
}

let mut map = GenMap::<u64, Tiny>::new_with_config();
let key = map.insert(7);
assert_eq!(core::mem::size_of_val(&key), 2);
```

`Option<Key>` is the same size as `Key`.

## License

gen_map is released under the MIT license.