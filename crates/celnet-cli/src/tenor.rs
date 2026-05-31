//! Parsing of the standard FX tenor shorthand (`ON`, `1W`, `3M`, `1Y`) into the
//! [`celnet_types::Tenor`] vocabulary.
//!
//! The market quotes pillars as a count plus a unit letter; this is the single
//! place that lexes that shorthand so every subcommand accepts the same syntax.
//! Pure and total: an unrecognised string is a typed error, never a panic.

use celnet_types::{BrokenDate, Tenor};

/// A failure parsing a tenor shorthand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TenorParseError {
    /// The string did not match `ON` or `<count><unit>` with a valid unit.
    Malformed(String),
    /// The numeric count was absent or not a positive integer.
    BadCount(String),
}

impl core::fmt::Display for TenorParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TenorParseError::Malformed(s) => write!(
                f,
                "malformed tenor `{s}`: expected ON or <count><W|M|Y> (e.g. 1W, 3M, 1Y)"
            ),
            TenorParseError::BadCount(s) => {
                write!(f, "bad tenor count in `{s}`: need a positive integer")
            }
        }
    }
}

impl std::error::Error for TenorParseError {}

/// Parse the FX tenor shorthand into a [`Tenor`].
///
/// Accepts (case-insensitive): the short-end words `ON` → [`Tenor::Overnight`],
/// `TN` → [`Tenor::TomNext`], `SN` → [`Tenor::SpotNext`]; the ladder `<n>W` →
/// [`Tenor::Weeks`], `<n>M` → [`Tenor::Months`], `<n>Y` → [`Tenor::Years`]; the
/// IMM ordinal `<n>IMM` → [`Tenor::Imm`]; and an explicit broken date
/// `YYYY-MM-DD` → [`Tenor::BrokenDate`]. `n` is a positive integer. Surrounding
/// whitespace is ignored.
///
/// # Errors
///
/// [`TenorParseError`] if the string is empty, has an unknown unit, carries a
/// non-positive / non-integer count, or is a malformed date.
pub(crate) fn parse_tenor(s: &str) -> Result<Tenor, TenorParseError> {
    let trimmed = s.trim();
    let upper = trimmed.to_ascii_uppercase();
    match upper.as_str() {
        "ON" => return Ok(Tenor::Overnight),
        "TN" => return Ok(Tenor::TomNext),
        "SN" => return Ok(Tenor::SpotNext),
        _ => {}
    }
    // Explicit broken date: YYYY-MM-DD.
    if let Some(bd) = parse_broken_date(trimmed) {
        return Ok(Tenor::BrokenDate(bd));
    }
    // IMM ordinal: `<n>IMM`.
    if let Some(digits) = upper.strip_suffix("IMM") {
        let n: u8 = digits
            .parse()
            .map_err(|_| TenorParseError::BadCount(s.to_owned()))?;
        if n == 0 {
            return Err(TenorParseError::BadCount(s.to_owned()));
        }
        return Ok(Tenor::Imm(n));
    }
    let mut chars = upper.chars();
    let unit = chars
        .next_back()
        .ok_or_else(|| TenorParseError::Malformed(s.to_owned()))?;
    let digits = chars.as_str();
    let count: u16 = digits
        .parse()
        .map_err(|_| TenorParseError::BadCount(s.to_owned()))?;
    if count == 0 {
        return Err(TenorParseError::BadCount(s.to_owned()));
    }
    match unit {
        'W' => Ok(Tenor::Weeks(count)),
        'M' => Ok(Tenor::Months(count)),
        'Y' => Ok(Tenor::Years(count)),
        _ => Err(TenorParseError::Malformed(s.to_owned())),
    }
}

/// Parse a `YYYY-MM-DD` broken-date shorthand into a [`BrokenDate`], or `None`
/// if the string is not in that form. The civil triple's range is validated by
/// the calendar layer on resolution, not here.
fn parse_broken_date(s: &str) -> Option<BrokenDate> {
    let mut parts = s.split('-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u8 = parts.next()?.parse().ok()?;
    let day: u8 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(BrokenDate::new(year, month, day))
}

/// Render a [`Tenor`] back to its shorthand (the inverse of [`parse_tenor`] for
/// every value [`parse_tenor`] produces).
#[must_use]
pub(crate) fn format_tenor(tenor: Tenor) -> String {
    match tenor {
        Tenor::Overnight => "ON".to_owned(),
        Tenor::TomNext => "TN".to_owned(),
        Tenor::SpotNext => "SN".to_owned(),
        Tenor::Weeks(n) => format!("{n}W"),
        Tenor::Months(n) => format!("{n}M"),
        Tenor::Years(n) => format!("{n}Y"),
        Tenor::Imm(n) => format!("{n}IMM"),
        Tenor::BrokenDate(b) => format!("{:04}-{:02}-{:02}", b.year, b.month, b.day),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_unit() {
        assert_eq!(parse_tenor("ON").unwrap(), Tenor::Overnight);
        assert_eq!(parse_tenor("on").unwrap(), Tenor::Overnight);
        assert_eq!(parse_tenor("2W").unwrap(), Tenor::Weeks(2));
        assert_eq!(parse_tenor("3m").unwrap(), Tenor::Months(3));
        assert_eq!(parse_tenor(" 1Y ").unwrap(), Tenor::Years(1));
    }

    #[test]
    fn rejects_bad_input() {
        assert!(matches!(
            parse_tenor("1X"),
            Err(TenorParseError::Malformed(_))
        ));
        assert!(matches!(
            parse_tenor("Y"),
            Err(TenorParseError::BadCount(_))
        ));
        assert!(matches!(
            parse_tenor("0M"),
            Err(TenorParseError::BadCount(_))
        ));
        assert!(matches!(
            parse_tenor(""),
            Err(TenorParseError::Malformed(_))
        ));
    }

    #[test]
    fn round_trips_through_format() {
        for t in [
            Tenor::Overnight,
            Tenor::Weeks(2),
            Tenor::Months(6),
            Tenor::Years(1),
        ] {
            assert_eq!(parse_tenor(&format_tenor(t)).unwrap(), t);
        }
    }
}
