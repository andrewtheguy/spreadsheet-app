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

/// How [`filter_rows`] treats rows whose value is present in `right`'s column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FilterMode {
    /// Drop rows whose value appears in `right`'s column (the default).
    Exclude,
    /// Keep only rows whose value appears in `right`'s column.
    Include,
}

/// Options controlling [`filter_rows`]. `camelCase` on the wire so the frontend can send
/// `caseInsensitive` (Tauri only auto-converts top-level command args, not nested fields).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterOptions {
    pub mode: FilterMode,
    /// When true, values are compared case-insensitively.
    pub case_insensitive: bool,
}

/// Index of the first header equal to `name`, if any.
pub fn column_index(table: &CsvTable, name: &str) -> Option<usize> {
    table.headers.iter().position(|header| header == name)
}

/// Filters `left`'s rows by whether their value in the `column` (resolved by header name)
/// appears among `right`'s values in that same-named column.
///
/// The column is resolved independently in each table, so it may sit at different positions
/// on the two sides. If `right` lacks the column, `left` is returned unchanged. Comparison
/// is exact (optionally case-insensitive via [`FilterOptions::case_insensitive`]); a `left`
/// table that lacks the column never matches. `Include` keeps matching rows, `Exclude` drops
/// them. Kept rows preserve their original cells, order, and duplicates.
pub fn filter_rows(
    left: &CsvTable,
    right: &CsvTable,
    column: &str,
    opts: &FilterOptions,
) -> CsvTable {
    let Some(right_idx) = column_index(right, column) else {
        return left.clone();
    };
    let left_idx = column_index(left, column);

    let normalize = |value: &str| -> String {
        if opts.case_insensitive {
            value.to_lowercase()
        } else {
            value.to_owned()
        }
    };

    // Rows are header-width (see `parse_csv`), so the cell at `right_idx` is always present.
    let right_values: HashSet<String> = right
        .rows
        .iter()
        .map(|row| normalize(&row[right_idx]))
        .collect();

    let rows = left
        .rows
        .iter()
        .filter(|row| {
            let in_set = left_idx.is_some_and(|i| right_values.contains(&normalize(&row[i])));
            match opts.mode {
                FilterMode::Include => in_set,
                FilterMode::Exclude => !in_set,
            }
        })
        .cloned()
        .collect();

    CsvTable {
        headers: left.headers.clone(),
        rows,
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

    // --- filter_rows ---

    fn opts(mode: FilterMode, case_insensitive: bool) -> FilterOptions {
        FilterOptions {
            mode,
            case_insensitive,
        }
    }

    #[test]
    fn exclude_drops_rows_whose_value_is_in_right() {
        let left = table(
            &["id", "name"],
            &[&["A", "Apple"], &["B", "Banana"], &["C", "Cherry"]],
        );
        let right = table(&["id"], &[&["B"], &["C"]]);
        let result = filter_rows(&left, &right, "id", &opts(FilterMode::Exclude, false));
        assert_eq!(result.headers, vec!["id".to_string(), "name".to_string()]);
        assert_eq!(result.rows, rows(&[&["A", "Apple"]]));
    }

    #[test]
    fn include_keeps_only_rows_whose_value_is_in_right() {
        let left = table(
            &["id", "name"],
            &[&["A", "Apple"], &["B", "Banana"], &["C", "Cherry"]],
        );
        let right = table(&["id"], &[&["B"], &["C"]]);
        let result = filter_rows(&left, &right, "id", &opts(FilterMode::Include, false));
        assert_eq!(result.rows, rows(&[&["B", "Banana"], &["C", "Cherry"]]));
    }

    #[test]
    fn case_sensitive_by_default() {
        let left = table(&["id"], &[&["b"], &["B"]]);
        let right = table(&["id"], &[&["B"]]);
        let result = filter_rows(&left, &right, "id", &opts(FilterMode::Include, false));
        assert_eq!(result.rows, rows(&[&["B"]]));
    }

    #[test]
    fn case_insensitive_matches_regardless_of_case() {
        let left = table(&["id"], &[&["b"], &["B"], &["c"]]);
        let right = table(&["id"], &[&["B"]]);
        let result = filter_rows(&left, &right, "id", &opts(FilterMode::Include, true));
        assert_eq!(result.rows, rows(&[&["b"], &["B"]]));
    }

    #[test]
    fn column_missing_in_right_returns_left_unchanged() {
        let left = table(&["id", "name"], &[&["A", "Apple"], &["B", "Banana"]]);
        let right = table(&["other"], &[&["A"]]);
        let result = filter_rows(&left, &right, "id", &opts(FilterMode::Exclude, false));
        assert_eq!(result.rows, left.rows);
    }

    #[test]
    fn resolves_column_by_name_at_different_indices() {
        // "id" is column 1 on the left but column 0 on the right.
        let left = table(&["name", "id"], &[&["Apple", "A"], &["Banana", "B"]]);
        let right = table(&["id", "extra"], &[&["B", "x"]]);
        let result = filter_rows(&left, &right, "id", &opts(FilterMode::Include, false));
        assert_eq!(result.rows, rows(&[&["Banana", "B"]]));
    }

    #[test]
    fn preserves_duplicate_left_rows_and_order() {
        let left = table(
            &["id", "v"],
            &[&["B", "first"], &["A", "a"], &["B", "second"]],
        );
        let right = table(&["id"], &[&["B"]]);
        let result = filter_rows(&left, &right, "id", &opts(FilterMode::Include, false));
        assert_eq!(result.rows, rows(&[&["B", "first"], &["B", "second"]]));
    }
}
