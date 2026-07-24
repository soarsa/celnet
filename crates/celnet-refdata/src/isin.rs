//! ISO 6166 (ISIN) integrity: the mod-10 (Luhn) check-digit computation, a
//! well-formedness verifier, and a builder that stamps the correct check digit onto
//! an 11-character country+NSIN body.
//!
//! The algorithm is re-derived from the published ISO 6166 standard (each letter
//! expands to two digits `A`=10 … `Z`=35, then Luhn runs right-to-left over the
//! resulting digit string) and anchored in tests against real, published Treasury
//! ISINs — so a curated non-US ISIN we mint is verifiably standard-conformant, not
//! merely length-checked.

/// The ISO-6166 mod-10 (Luhn) check digit for an 11-character ISIN **body** (the
/// 2-letter country prefix + 9-character NSIN, without the final check digit).
///
/// Each letter expands to two digits (`A`=10 … `Z`=35); the Luhn algorithm then
/// runs right-to-left over the resulting digit string. Non-alphanumeric bytes are
/// skipped (a caller passing a well-formed body will never hit that).
#[must_use]
pub fn check_digit(body: &str) -> u8 {
    // Expand letters to digits, building the digit sequence left-to-right.
    let mut digits: Vec<u8> = Vec::with_capacity(22);
    for c in body.bytes() {
        if c.is_ascii_digit() {
            digits.push(c - b'0');
        } else if c.is_ascii_uppercase() {
            let v = c - b'A' + 10; // A..Z => 10..35
            digits.push(v / 10);
            digits.push(v % 10);
        }
    }
    // Luhn from the right: double every second digit (starting with the rightmost).
    let mut sum = 0u32;
    for (i, &d) in digits.iter().rev().enumerate() {
        let mut v = u32::from(d);
        if i % 2 == 0 {
            v *= 2;
            if v > 9 {
                v -= 9;
            }
        }
        sum += v;
    }
    ((10 - (sum % 10)) % 10) as u8
}

/// Whether an ISIN is well-formed: 12 chars, two leading letters, an alphanumeric
/// 11-char body, a trailing digit, and a matching ISO-6166 check digit.
#[must_use]
pub fn is_well_formed(isin: &str) -> bool {
    let bytes = isin.as_bytes();
    if bytes.len() != 12 {
        return false;
    }
    if !bytes[0].is_ascii_uppercase() || !bytes[1].is_ascii_uppercase() {
        return false;
    }
    if !bytes[..11].iter().all(u8::is_ascii_alphanumeric) || !bytes[11].is_ascii_digit() {
        return false;
    }
    check_digit(&isin[..11]) == (bytes[11] - b'0')
}

/// Build a check-valid 12-character ISIN from an 11-character `body` (2-letter
/// country prefix + 9-character NSIN), appending the computed ISO-6166 check digit.
///
/// Returns `None` if `body` is not exactly 11 characters, does not start with two
/// uppercase letters, or is not otherwise uppercase-alphanumeric — so a caller can
/// never accidentally forge a malformed identifier.
#[must_use]
pub fn build(body: &str) -> Option<String> {
    let bytes = body.as_bytes();
    if bytes.len() != 11 {
        return None;
    }
    if !bytes[0].is_ascii_uppercase() || !bytes[1].is_ascii_uppercase() {
        return None;
    }
    if !bytes
        .iter()
        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    {
        return None;
    }
    let cd = check_digit(body);
    Some(format!("{body}{cd}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_real_published_treasury_isins() {
        // Real Treasury ISINs from the bundled securities master (their published
        // check digits): the algorithm must reproduce each one, anchoring the code
        // to the ISO-6166 standard rather than to itself.
        for isin in ["US912797TS67", "US91282CPZ85"] {
            assert!(is_well_formed(isin), "{isin} should validate");
            let rebuilt = build(&isin[..11]).expect("11-char body builds");
            assert_eq!(rebuilt, isin, "recomputed check digit must match {isin}");
        }
    }

    #[test]
    fn rejects_corruption_and_bad_shape() {
        assert!(!is_well_formed("US912797TS60"), "wrong check digit");
        assert!(!is_well_formed("US912797TS6"), "too short");
        assert!(!is_well_formed("1S912797TS67"), "non-letter country");
    }

    #[test]
    fn build_rejects_bad_bodies() {
        assert!(build("US912797TS").is_none(), "10-char body rejected");
        assert!(build("us912797TS6X").is_none(), "wrong length rejected");
        assert!(build("1S00GILT10Y").is_none(), "non-letter country rejected");
        let ok = build("GBGILT10Y36").expect("valid GB body builds");
        assert!(is_well_formed(&ok), "built ISIN self-verifies");
        assert_eq!(ok.len(), 12);
    }
}
