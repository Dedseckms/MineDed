# glib 0.18.5 compatible security backport

Source: the unmodified crates.io `glib` 0.18.5 package, maintained by the gtk-rs
Project Developers, under its original MIT license (`LICENSE`). Copyright notices
in the upstream sources are retained.

Applied change: `VariantStrIter::impl_get` passes a mutable output pointer to
`g_variant_get_child`. This is the fix by decathorpe from the upstream
[gtk-rs-core PR 1343](https://github.com/gtk-rs/gtk-rs-core/pull/1343), addressing
RUSTSEC-2024-0429. No other upstream implementation changes are made.

GTK 0.18 currently constrains this dependency to glib 0.18. The package version is
kept truthful; version-only advisory tools may still report 0.18.5. No advisory
ignore is added. Remove this vendor patch when the GTK dependency chain supports
an upstream fixed version.

Linux regression: `cargo test -p lc-runtime --release --test glib_variant_iter`.
It exercises forward/backward/nth borrowed-string iteration with optimizations.
