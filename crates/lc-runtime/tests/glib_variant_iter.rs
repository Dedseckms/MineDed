#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used)]

use glib::variant::ToVariant;

/// Optimized execution covers the mutable FFI output in VariantStrIter.
#[test]
fn borrowed_string_iteration_preserves_values_in_both_directions() {
    let values = ["first", "second", "third", "fourth"];
    let variant = values.to_vec().to_variant();
    let mut iter = variant.array_iter_str().unwrap();
    assert_eq!(iter.next(), Some("first"));
    assert_eq!(iter.next_back(), Some("fourth"));
    assert_eq!(iter.nth(1), Some("third"));
    assert_eq!(iter.next(), None);
    assert_eq!(iter.next_back(), None);
    assert_eq!(
        variant.array_iter_str().unwrap().collect::<Vec<_>>(),
        values
    );
    assert_eq!(
        variant.array_iter_str().unwrap().rev().collect::<Vec<_>>(),
        values.into_iter().rev().collect::<Vec<_>>()
    );
}
