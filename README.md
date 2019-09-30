cow
=======

A light HTTP server for Rust.

# Getting Started

## example cow.toml config

```toml
debug = false
port = "3000"
address = "0.0.0.0"
root_dir = "/home/betta/license_generator/templates"

[[server]]
  [server.location]
    pattern = "/api/v1"
    proxy_pass = "127.0.0.1:8000"

[[server]]
  [server.location]
    pattern = "/"
    static_path = "/"
```

## install and run
To build the cow from the root directory of this repo:

```
> cargo build
```

## License

Licensed under either of

 * Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any
additional terms or conditions.
