# easy-parallel

[![Build](https://github.com/stjepang/easy-parallel/workflows/Build%20and%20test/badge.svg)](
https://github.com/stjepang/easy-parallel/actions)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](
https://github.com/stjepang/easy-parallel)
[![Cargo](https://img.shields.io/crates/v/easy-parallel.svg)](
https://crates.io/crates/easy-parallel)
[![Documentation](https://docs.rs/easy-parallel/badge.svg)](
https://docs.rs/easy-parallel)

Run closures in parallel.

This is a simple primitive for spawning threads in bulk and waiting for them to complete.
Threads are allowed to borrow local variables from the main thread.

## Examples

Run two threads that increment a number:

```rust
use easy_parallel::Parallel;
use std::sync::Mutex;

let mut m = Mutex::new(0);

Parallel::new()
    .add(|| *m.lock().unwrap() += 1)
    .add(|| *m.lock().unwrap() += 1)
    .run();

assert_eq!(*m.get_mut().unwrap(), 2);
```

Square each number of a vector on a different thread:

```rust
use easy_parallel::Parallel;

let v = vec![10, 20, 30];

let mut squares = Parallel::new()
    .each(0..v.len(), |i| v[i] * v[i])
    .run();

squares.sort();
assert_eq!(squares, [100, 400, 900]);
```

## License

Licensed under either of

 * Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

#### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
