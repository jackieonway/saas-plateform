//! Bounded pagination shared by platform HTTP adapters.

/// Validated page, size and checked offset. Invalid syntax keeps legacy defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Paging {
    page: i64,
    size: i64,
    offset: i64,
}

impl Paging {
    /// Page is capped at one million; size accepts 1..=200, defaulting to ten.
    pub fn from_raw(page: Option<&str>, size: Option<&str>) -> Self {
        let page = page
            .and_then(|v| v.parse::<i64>().ok())
            .filter(|v| *v >= 1)
            .unwrap_or(1)
            .min(1_000_000);
        let size = size
            .and_then(|v| v.parse::<i64>().ok())
            .filter(|v| (1..=200).contains(v))
            .unwrap_or(10);
        let offset = page
            .checked_sub(1)
            .and_then(|v| v.checked_mul(size))
            .unwrap_or(0);
        Self { page, size, offset }
    }

    /// The stable `(page, size, offset)` adapter contract.
    pub fn parts(self) -> (i64, i64, i64) {
        (self.page, self.size, self.offset)
    }
}
