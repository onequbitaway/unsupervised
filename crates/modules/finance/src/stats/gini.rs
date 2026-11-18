//! The Gini coefficient.

use crate::util::BP;

/// Returns the Gini coefficient of a list of wealth values in basis
/// points: 0 when all values are equal, near 10,000 when one value has
/// all. The list is sorted in place. An empty list and a list whose sum is
/// zero give 0.
///
/// With the values sorted from low to high and numbered from 1 to `n`:
///
/// ```text
/// gini = (2 * sum(i * x_i) - (n + 1) * sum(x_i)) / (n * sum(x_i))
/// ```
#[must_use]
pub fn gini_bp(wealth: &mut [i64]) -> i64 {
    wealth.sort_unstable();
    let n = wealth.len() as i128;
    let total: i128 = wealth.iter().map(|&x| i128::from(x)).sum();
    if n == 0 || total <= 0 {
        return 0;
    }
    let weighted: i128 = wealth.iter().zip(1i128..).map(|(&x, rank)| rank * i128::from(x)).sum();
    let numer = 2 * weighted - (n + 1) * total;
    // The coefficient is from 0 to 1, so the result fits.
    i64::try_from(numer * i128::from(BP) / (n * total)).unwrap_or(0)
}
