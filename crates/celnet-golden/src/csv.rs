//! A tiny dependency-free CSV reader for the frozen golden tables.
//!
//! The reference tables are emitted by `tools/goldgen/generate.py` in a strict,
//! self-imposed dialect — plain ASCII, comma-separated, no quoting, no embedded
//! commas or newlines in fields, a single header row, and `#`-prefixed comment
//! lines at the top — so a full RFC-4180 parser is unnecessary. Keeping the
//! reader dependency-free means the golden gate has no third-party parsing crate
//! in its trust boundary: the oracle and its loader are both auditable in-tree.

use std::collections::HashMap;
use std::fmt;

/// An error raised while loading or accessing a CSV table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CsvError {
    /// The file could not be read from disk.
    Io(String),
    /// The file had no header row (only comments or empty).
    MissingHeader,
    /// A data row had a different column count than the header.
    RaggedRow {
        /// 0-based data-row index (comments and header excluded).
        row: usize,
        /// Number of columns the header declared.
        expected: usize,
        /// Number of columns the offending row had.
        found: usize,
    },
    /// A named column was requested but is not present in the header.
    UnknownColumn(String),
    /// A field failed to parse as the requested numeric type.
    Parse {
        /// 0-based data-row index.
        row: usize,
        /// Column name whose value failed to parse.
        column: String,
        /// The raw field text that could not be parsed.
        value: String,
    },
}

impl fmt::Display for CsvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CsvError::Io(e) => write!(f, "I/O error reading CSV: {e}"),
            CsvError::MissingHeader => write!(f, "CSV has no header row"),
            CsvError::RaggedRow {
                row,
                expected,
                found,
            } => write!(f, "CSV row {row} has {found} columns, expected {expected}"),
            CsvError::UnknownColumn(c) => write!(f, "unknown CSV column: {c}"),
            CsvError::Parse { row, column, value } => {
                write!(f, "CSV row {row} column {column}: cannot parse {value:?}")
            }
        }
    }
}

impl std::error::Error for CsvError {}

/// A parsed CSV table: a header and the data rows, with by-name column access.
///
/// Lines beginning with `#` (after trimming leading whitespace) and wholly
/// blank lines are ignored. The first remaining line is the header; the rest are
/// data rows. All access is by column name to keep call sites order-independent.
#[derive(Debug, Clone)]
pub struct CsvTable {
    header: Vec<String>,
    index: HashMap<String, usize>,
    rows: Vec<Vec<String>>,
}

impl CsvTable {
    /// Parse a CSV table from in-memory text.
    ///
    /// # Errors
    /// Returns [`CsvError::MissingHeader`] if no non-comment line exists, or
    /// [`CsvError::RaggedRow`] if a data row's column count differs from the
    /// header's.
    pub fn parse(text: &str) -> Result<Self, CsvError> {
        let mut lines = text.lines().map(str::trim_end_matches_cr).filter(|l| {
            let t = l.trim_start();
            !t.is_empty() && !t.starts_with('#')
        });

        let header_line = lines.next().ok_or(CsvError::MissingHeader)?;
        let header: Vec<String> = split_fields(header_line);
        let index: HashMap<String, usize> = header
            .iter()
            .enumerate()
            .map(|(i, name)| (name.clone(), i))
            .collect();

        let mut rows = Vec::new();
        for (row, line) in lines.enumerate() {
            let fields = split_fields(line);
            if fields.len() != header.len() {
                return Err(CsvError::RaggedRow {
                    row,
                    expected: header.len(),
                    found: fields.len(),
                });
            }
            rows.push(fields);
        }

        Ok(Self {
            header,
            index,
            rows,
        })
    }

    /// Load and parse a CSV table from a file path.
    ///
    /// # Errors
    /// Returns [`CsvError::Io`] if the file cannot be read, plus any error from
    /// [`CsvTable::parse`].
    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Self, CsvError> {
        let text = std::fs::read_to_string(path.as_ref())
            .map_err(|e| CsvError::Io(format!("{}: {e}", path.as_ref().display())))?;
        Self::parse(&text)
    }

    /// The header column names, in order.
    #[must_use]
    pub fn header(&self) -> &[String] {
        &self.header
    }

    /// The number of data rows (header and comments excluded).
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether the table has zero data rows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Borrow the raw field text at `(row, column-name)`.
    ///
    /// # Errors
    /// Returns [`CsvError::UnknownColumn`] if `column` is not in the header.
    /// Panics in debug builds if `row` is out of range (callers iterate by
    /// `0..len()`).
    pub fn get(&self, row: usize, column: &str) -> Result<&str, CsvError> {
        let col = *self
            .index
            .get(column)
            .ok_or_else(|| CsvError::UnknownColumn(column.to_owned()))?;
        Ok(self.rows[row][col].as_str())
    }

    /// Parse the field at `(row, column-name)` as an `f64`.
    ///
    /// # Errors
    /// Returns [`CsvError::UnknownColumn`] or [`CsvError::Parse`].
    pub fn get_f64(&self, row: usize, column: &str) -> Result<f64, CsvError> {
        let raw = self.get(row, column)?;
        raw.parse::<f64>().map_err(|_| CsvError::Parse {
            row,
            column: column.to_owned(),
            value: raw.to_owned(),
        })
    }

    /// Parse the field at `(row, column-name)` as an optional `f64`: an empty
    /// field yields `None` (used for the touch table's mutually-exclusive
    /// single-`barrier` vs corridor-`lower`/`upper` columns).
    ///
    /// # Errors
    /// Returns [`CsvError::UnknownColumn`] or [`CsvError::Parse`].
    pub fn get_opt_f64(&self, row: usize, column: &str) -> Result<Option<f64>, CsvError> {
        let raw = self.get(row, column)?;
        if raw.is_empty() {
            return Ok(None);
        }
        raw.parse::<f64>().map(Some).map_err(|_| CsvError::Parse {
            row,
            column: column.to_owned(),
            value: raw.to_owned(),
        })
    }
}

/// Split a single line into comma-separated, whitespace-trimmed fields.
fn split_fields(line: &str) -> Vec<String> {
    line.split(',').map(|f| f.trim().to_owned()).collect()
}

/// Helper extension to strip a trailing carriage return on Windows-style files.
trait TrimCr {
    fn trim_end_matches_cr(&self) -> &str;
}

impl TrimCr for str {
    fn trim_end_matches_cr(&self) -> &str {
        self.strip_suffix('\r').unwrap_or(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_header_and_rows() {
        let text = "# a comment\n\na,b,c\n1,2,3\n 4 , 5 ,6\n";
        let t = CsvTable::parse(text).unwrap();
        assert_eq!(t.header(), ["a", "b", "c"]);
        assert_eq!(t.len(), 2);
        assert!(!t.is_empty());
        assert_eq!(t.get(0, "b").unwrap(), "2");
        // Whitespace around fields is trimmed.
        assert_eq!(t.get(1, "a").unwrap(), "4");
    }

    #[test]
    fn parses_floats_including_scientific_and_negative() {
        let text = "x\n-1.7860836378222024e-17\n100.5\n";
        let t = CsvTable::parse(text).unwrap();
        assert!((t.get_f64(0, "x").unwrap() - -1.786_083_637_822_202_4e-17).abs() < 1e-30);
        assert!((t.get_f64(1, "x").unwrap() - 100.5).abs() < 1e-12);
    }

    #[test]
    fn rejects_ragged_rows() {
        let text = "a,b\n1,2\n3\n";
        let err = CsvTable::parse(text).unwrap_err();
        assert_eq!(
            err,
            CsvError::RaggedRow {
                row: 1,
                expected: 2,
                found: 1
            }
        );
    }

    #[test]
    fn rejects_missing_header() {
        assert_eq!(
            CsvTable::parse("# only comments\n\n").unwrap_err(),
            CsvError::MissingHeader
        );
    }

    #[test]
    fn unknown_column_and_bad_parse() {
        let t = CsvTable::parse("a\nhello\n").unwrap();
        assert_eq!(
            t.get(0, "missing").unwrap_err(),
            CsvError::UnknownColumn("missing".to_owned())
        );
        assert!(matches!(t.get_f64(0, "a"), Err(CsvError::Parse { .. })));
    }
}
