# limited-join

[![Docs.rs][docs-badge]][docs-url]
[![Crates.io][crates-badge]][crates-url]
[![Unlicense][license-badge]][license-url]

[crates-badge]: https://img.shields.io/crates/v/limited-join.svg?style=for-the-badge&
[crates-url]: https://crates.io/crates/limited-join
[license-badge]: https://img.shields.io/badge/license-MIT-blue.svg?style=for-the-badge&
[license-url]: https://github.com/zebp/limited-join/blob/main/LICENSE
[docs-badge]: https://img.shields.io/badge/docs.rs-rustdoc-green?style=for-the-badge&
[docs-url]: https://docs.rs/limited-join/

A zero-dependency crate providing a join [future](https://doc.rust-lang.org/stable/std/future/trait.Future.html) with limited concurrency.

## Example

```rust
// Pretend we have a ton of files we want to download, but don't want to overwhelm the server.
let futures = files_to_download.into_iter().map(download::download_file);

// Let's limit the number of concurrent downloads to 4, and wait for all the files to download.
limited_join::join(futures, 4).await;
```
