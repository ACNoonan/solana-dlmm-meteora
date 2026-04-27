// Replacement for `anyhow::Error` / `anchor_lang::error::Error` in extracted
// MeteoraAg/dlmm-sdk math. Variants are the subset actually referenced from
// the ported math.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    MathOverflow,
    BinIdOutOfBounds,
}

impl ErrorCode {
    /// Stable, human-readable reason. Stable in the sense that adding new
    /// variants is a minor bump but renaming an existing variant's reason
    /// string is breaking — keep these wording-stable across patch releases.
    pub const fn reason(self) -> &'static str {
        match self {
            ErrorCode::MathOverflow => "arithmetic overflow",
            ErrorCode::BinIdOutOfBounds => "bin_id out of [MIN_BIN_ID, MAX_BIN_ID]",
        }
    }
}

impl core::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.reason())
    }
}

impl core::error::Error for ErrorCode {}

pub type Result<T> = core::result::Result<T, ErrorCode>;

#[macro_export]
macro_rules! require {
    ($cond:expr, $err:expr $(,)?) => {
        if !($cond) {
            return Err($err);
        }
    };
}

#[macro_export]
macro_rules! require_gt {
    ($a:expr, $b:expr, $err:expr $(,)?) => {
        if !($a > $b) {
            return Err($err);
        }
    };
}

#[macro_export]
macro_rules! require_gte {
    ($a:expr, $b:expr, $err:expr $(,)?) => {
        if !($a >= $b) {
            return Err($err);
        }
    };
}

#[macro_export]
macro_rules! err {
    ($err:expr $(,)?) => {
        Err($err)
    };
}
