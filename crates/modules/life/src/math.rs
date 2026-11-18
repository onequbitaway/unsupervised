//! Small integer helpers.

/// Returns `a / b` rounded up, for `a` at or above zero and `b` above zero.
/// A divisor below 1 counts as 1.
pub(crate) fn div_ceil(a: i64, b: i64) -> i64 {
    let b = b.max(1);
    a.max(0).saturating_add(b - 1) / b
}
