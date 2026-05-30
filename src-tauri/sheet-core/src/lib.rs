//! Pure spreadsheet logic — CSV parsing, serializing, and row matching — extracted from
//! the Tauri app so it carries no CEF/dialog dependencies and can be unit-tested in
//! isolation.

use std::collections::HashSet;
use std::io::{Read, Write};

/// A CSV file parsed into a header row plus data rows. Serialized to the frontend for
/// display (`load_csv`) and deserialized back from it for export (`save_csv`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CsvTable {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// Parses CSV from `reader`, treating the first record as headers. Ragged rows are
/// accepted and normalized to the header width — short rows are padded with empty strings,
/// long rows are truncated — so cells stay column-aligned on the frontend.
pub fn parse_csv<R: Read>(reader: R) -> Result<CsvTable, csv::Error> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(reader);

    let headers: Vec<String> = rdr.headers()?.iter().map(String::from).collect();
    let column_count = headers.len();

    let rows = rdr
        .records()
        .map(|record| {
            record.map(|r| {
                let mut row: Vec<String> = r.iter().map(String::from).collect();
                row.resize(column_count, String::new());
                row
            })
        })
        .collect::<Result<Vec<Vec<String>>, _>>()?;

    Ok(CsvTable { headers, rows })
}

/// Writes `table` to `writer` as CSV: the header row followed by each data row.
pub fn write_csv<W: Write>(writer: W, table: &CsvTable) -> Result<(), csv::Error> {
    let mut wtr = csv::Writer::from_writer(writer);
    wtr.write_record(&table.headers)?;
    for row in &table.rows {
        wtr.write_record(row)?;
    }
    wtr.flush()?;
    Ok(())
}

/// Returns the rows of `left` whose first column also appears in `right`'s first column.
///
/// First-column values are compared after trimming surrounding whitespace, and the match
/// is case-sensitive. Blank or whitespace-only keys are skipped on both sides, so a blank
/// left key never matches and a blank right key never becomes a match target. Matching
/// rows keep their original (untrimmed) cells, preserving order and duplicates.
pub fn matching_rows(left: &CsvTable, right: &CsvTable) -> Vec<Vec<String>> {
    let right_keys: HashSet<&str> = right
        .rows
        .iter()
        .filter_map(|row| row.first())
        .map(|cell| cell.trim())
        .filter(|key| !key.is_empty())
        .collect();

    left.rows
        .iter()
        .filter(|row| {
            let key = row.first().map_or("", |cell| cell.trim());
            !key.is_empty() && right_keys.contains(key)
        })
        .cloned()
        .collect()
}

/// Builds the merged result: `left`'s headers paired with only the rows whose first column
/// matches `right`'s first column (see [`matching_rows`]).
pub fn merge(left: &CsvTable, right: &CsvTable) -> CsvTable {
    CsvTable {
        headers: left.headers.clone(),
        rows: matching_rows(left, right),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(data: &[&[&str]]) -> Vec<Vec<String>> {
        data.iter()
            .map(|r| r.iter().map(|s| s.to_string()).collect())
            .collect()
    }

    fn table(headers: &[&str], data: &[&[&str]]) -> CsvTable {
        CsvTable {
            headers: headers.iter().map(|s| s.to_string()).collect(),
            rows: rows(data),
        }
    }

    // --- parse_csv ---

    #[test]
    fn parse_reads_headers_and_rows() {
        let parsed = parse_csv("id,name\nA,Apple\nB,Banana\n".as_bytes()).unwrap();
        assert_eq!(
            parsed,
            table(&["id", "name"], &[&["A", "Apple"], &["B", "Banana"]])
        );
    }

    #[test]
    fn parse_pads_short_rows_and_truncates_long_rows() {
        // Header width is 2: the short row is padded with "", the long row is truncated.
        let parsed = parse_csv("a,b\nx\ny,z,extra\n".as_bytes()).unwrap();
        assert_eq!(parsed.rows, rows(&[&["x", ""], &["y", "z"]]));
    }

    #[test]
    fn parse_empty_input_yields_no_headers_or_rows() {
        let parsed = parse_csv("".as_bytes()).unwrap();
        assert!(parsed.headers.is_empty());
        assert!(parsed.rows.is_empty());
    }

    // --- write_csv ---

    #[test]
    fn write_then_parse_roundtrips() {
        let original = table(&["id", "name"], &[&["A", "Apple"], &["B", "Banana"]]);
        let mut buf = Vec::new();
        write_csv(&mut buf, &original).unwrap();
        assert_eq!(parse_csv(buf.as_slice()).unwrap(), original);
    }

    #[test]
    fn write_quotes_cells_containing_the_delimiter() {
        let mut buf = Vec::new();
        write_csv(&mut buf, &table(&["a", "b"], &[&["x,y", "z"]])).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("\"x,y\""), "comma cell should be quoted: {out}");
    }

    // --- matching_rows / merge ---

    #[test]
    fn keeps_only_left_rows_with_matching_first_column() {
        let left = table(
            &["k", "v"],
            &[&["A", "Apple"], &["B", "Banana"], &["C", "Cherry"]],
        );
        let right = table(&["k"], &[&["B"], &["C"]]);
        assert_eq!(
            matching_rows(&left, &right),
            rows(&[&["B", "Banana"], &["C", "Cherry"]])
        );
    }

    #[test]
    fn skips_blank_left_keys() {
        let left = table(
            &["k", "v"],
            &[&["", "blank"], &["   ", "spaces"], &["B", "Banana"]],
        );
        let right = table(&["k"], &[&["B"], &[""]]);
        assert_eq!(matching_rows(&left, &right), rows(&[&["B", "Banana"]]));
    }

    #[test]
    fn blank_right_keys_are_not_match_targets() {
        let left = table(&["k"], &[&[""]]);
        let right = table(&["k"], &[&[""], &["  "]]);
        assert!(matching_rows(&left, &right).is_empty());
    }

    #[test]
    fn trims_surrounding_whitespace_on_both_sides() {
        let left = table(&["k", "v"], &[&["  B  ", "Banana"]]);
        let right = table(&["k"], &[&[" B "]]);
        assert_eq!(matching_rows(&left, &right), rows(&[&["  B  ", "Banana"]]));
    }

    #[test]
    fn matching_is_case_sensitive() {
        let left = table(&["k", "v"], &[&["b", "lower"], &["B", "upper"]]);
        let right = table(&["k"], &[&["B"]]);
        assert_eq!(matching_rows(&left, &right), rows(&[&["B", "upper"]]));
    }

    #[test]
    fn preserves_duplicate_left_rows_and_order() {
        let left = table(
            &["k", "v"],
            &[&["B", "first"], &["A", "a"], &["B", "second"]],
        );
        let right = table(&["k"], &[&["B"]]);
        assert_eq!(
            matching_rows(&left, &right),
            rows(&[&["B", "first"], &["B", "second"]])
        );
    }

    #[test]
    fn merge_keeps_left_headers_with_matched_rows() {
        let left = table(&["id", "name"], &[&["A", "Apple"], &["B", "Banana"]]);
        let right = table(&["other"], &[&["B"]]);
        let merged = merge(&left, &right);
        assert_eq!(merged.headers, vec!["id".to_string(), "name".to_string()]);
        assert_eq!(merged.rows, rows(&[&["B", "Banana"]]));
    }
}
