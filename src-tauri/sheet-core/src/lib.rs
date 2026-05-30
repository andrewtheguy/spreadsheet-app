//! Pure spreadsheet logic — CSV parsing, serializing, and row matching — extracted from
//! the Tauri app so it carries no CEF/dialog dependencies and can be unit-tested in
//! isolation.

use std::collections::{HashMap, HashSet};
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

/// How a key compares between the two tables in [`compare`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ComparisonStatus {
    /// Key present on both sides with equal (trimmed) values.
    Matched,
    /// Key present on both sides with differing values.
    Diff,
    /// Key present only in the left table.
    OnlyLeft,
    /// Key present only in the right table.
    OnlyRight,
}

/// One key's comparison across the two tables. `left_value`/`right_value` are `None` when
/// that side lacks the key. `serde` uses camelCase so the field names cross the Tauri
/// boundary as `leftValue`/`rightValue`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComparisonRow {
    pub key: String,
    pub left_value: Option<String>,
    pub right_value: Option<String>,
    pub status: ComparisonStatus,
}

/// Per-status tallies over a [`ComparisonResult`]'s rows.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComparisonSummary {
    pub total: usize,
    pub matched: usize,
    pub diff: usize,
    pub only_left: usize,
    pub only_right: usize,
}

/// The full result of [`compare`]: one row per distinct key plus summary tallies.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComparisonResult {
    pub rows: Vec<ComparisonRow>,
    pub key_column: String,
    pub value_column: String,
    pub summary: ComparisonSummary,
}

/// Header names present in BOTH tables, in `left`'s first-occurrence order and de-duplicated.
/// Populates the compare key/value selectors.
pub fn common_columns(left: &CsvTable, right: &CsvTable) -> Vec<String> {
    let mut seen = HashSet::new();
    left.headers
        .iter()
        .filter(|h| column_index(right, h).is_some() && seen.insert(h.as_str()))
        .cloned()
        .collect()
}

// Sentinel for a key column that is absent from a table, so it never collides with an
// empty-string key.
const MISSING_KEY: &str = "\0__NULL__\0";

/// VLOOKUP-style diff: classifies every distinct value of `key_column` across the two tables
/// as matched / diff / only-left / only-right, comparing the corresponding `value_column`
/// cells (equal after trimming → matched). Columns are resolved by header name per table.
///
/// Keys are compared after optional lowercasing (`case_insensitive`); when a key repeats in a
/// table the last occurrence wins. Rows appear in `left`'s first-occurrence order followed by
/// right-only keys.
pub fn compare(
    left: &CsvTable,
    right: &CsvTable,
    key_column: &str,
    value_column: &str,
    case_insensitive: bool,
) -> ComparisonResult {
    let normalize_key = |raw: Option<&str>| -> String {
        match raw {
            None => MISSING_KEY.to_owned(),
            Some(s) if case_insensitive => s.to_lowercase(),
            Some(s) => s.to_owned(),
        }
    };

    // Maps a normalized key to its original (display) key and the value-column cell. Last
    // occurrence wins. Returns the map plus the normalized keys in first-occurrence order.
    let build = |table: &CsvTable| -> (HashMap<String, (String, String)>, Vec<String>) {
        let key_idx = column_index(table, key_column);
        let val_idx = column_index(table, value_column);
        let mut map: HashMap<String, (String, String)> = HashMap::new();
        let mut order: Vec<String> = Vec::new();
        for row in &table.rows {
            let raw_key = key_idx.map(|i| row[i].as_str());
            let nk = normalize_key(raw_key);
            if !map.contains_key(&nk) {
                order.push(nk.clone());
            }
            let orig = raw_key.unwrap_or("").to_owned();
            let value = val_idx.map(|i| row[i].clone()).unwrap_or_default();
            map.insert(nk, (orig, value));
        }
        (map, order)
    };

    let (left_map, left_order) = build(left);
    let (right_map, right_order) = build(right);

    // Left's keys first (in order), then keys only present on the right.
    let mut ordered_keys = left_order;
    for nk in right_order {
        if !left_map.contains_key(&nk) {
            ordered_keys.push(nk);
        }
    }

    let mut rows = Vec::with_capacity(ordered_keys.len());
    let mut summary = ComparisonSummary {
        total: 0,
        matched: 0,
        diff: 0,
        only_left: 0,
        only_right: 0,
    };

    for nk in ordered_keys {
        let left_entry = left_map.get(&nk);
        let right_entry = right_map.get(&nk);
        let status = match (left_entry, right_entry) {
            (Some(_), None) => {
                summary.only_left += 1;
                ComparisonStatus::OnlyLeft
            }
            (None, Some(_)) => {
                summary.only_right += 1;
                ComparisonStatus::OnlyRight
            }
            (Some((_, lv)), Some((_, rv))) => {
                if lv.trim() == rv.trim() {
                    summary.matched += 1;
                    ComparisonStatus::Matched
                } else {
                    summary.diff += 1;
                    ComparisonStatus::Diff
                }
            }
            (None, None) => unreachable!("key came from one of the two maps"),
        };
        // Prefer the left side's original key for display.
        let key = left_entry
            .or(right_entry)
            .map(|(orig, _)| orig.clone())
            .unwrap_or_default();
        rows.push(ComparisonRow {
            key,
            left_value: left_entry.map(|(_, v)| v.clone()),
            right_value: right_entry.map(|(_, v)| v.clone()),
            status,
        });
    }

    summary.total = rows.len();

    ComparisonResult {
        rows,
        key_column: key_column.to_owned(),
        value_column: value_column.to_owned(),
        summary,
    }
}

/// Renders a [`ComparisonResult`] as a four-column table (key, left value, right value,
/// status label) so it can be exported via [`write_csv`].
pub fn comparison_to_table(result: &ComparisonResult) -> CsvTable {
    let headers = vec![
        result.key_column.clone(),
        format!("{} (Left)", result.value_column),
        format!("{} (Right)", result.value_column),
        "Status".to_owned(),
    ];
    let rows = result
        .rows
        .iter()
        .map(|row| {
            let status = match row.status {
                ComparisonStatus::Matched => "matched",
                ComparisonStatus::Diff => "diff",
                ComparisonStatus::OnlyLeft => "only left",
                ComparisonStatus::OnlyRight => "only right",
            };
            vec![
                row.key.clone(),
                row.left_value.clone().unwrap_or_default(),
                row.right_value.clone().unwrap_or_default(),
                status.to_owned(),
            ]
        })
        .collect();
    CsvTable { headers, rows }
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

    // --- common_columns / compare ---

    #[test]
    fn common_columns_keeps_left_order_and_intersection() {
        let left = table(&["id", "name", "city"], &[]);
        let right = table(&["city", "id", "extra"], &[]);
        assert_eq!(common_columns(&left, &right), vec!["id", "city"]);
    }

    #[test]
    fn common_columns_dedupes_repeated_left_headers() {
        let left = table(&["id", "id", "v"], &[]);
        let right = table(&["id", "v"], &[]);
        assert_eq!(common_columns(&left, &right), vec!["id", "v"]);
    }

    #[test]
    fn compare_classifies_all_four_statuses() {
        let left = table(
            &["id", "score"],
            &[&["A", "10"], &["B", "20"], &["C", "30"]],
        );
        let right = table(
            &["id", "score"],
            &[&["A", "10"], &["B", "25"], &["D", "40"]],
        );
        let result = compare(&left, &right, "id", "score", false);

        // Order: left first-occurrence (A, B, C) then right-only (D).
        let got: Vec<(&str, ComparisonStatus)> = result
            .rows
            .iter()
            .map(|r| (r.key.as_str(), r.status))
            .collect();
        assert_eq!(
            got,
            vec![
                ("A", ComparisonStatus::Matched),
                ("B", ComparisonStatus::Diff),
                ("C", ComparisonStatus::OnlyLeft),
                ("D", ComparisonStatus::OnlyRight),
            ]
        );

        assert_eq!(result.key_column, "id");
        assert_eq!(result.value_column, "score");
        assert_eq!(result.summary.total, 4);
        assert_eq!(result.summary.matched, 1);
        assert_eq!(result.summary.diff, 1);
        assert_eq!(result.summary.only_left, 1);
        assert_eq!(result.summary.only_right, 1);

        // only-left has no right value, and vice versa.
        let c = &result.rows[2];
        assert_eq!(c.left_value.as_deref(), Some("30"));
        assert_eq!(c.right_value, None);
        let d = &result.rows[3];
        assert_eq!(d.left_value, None);
        assert_eq!(d.right_value.as_deref(), Some("40"));
    }

    #[test]
    fn compare_values_match_after_trimming() {
        let left = table(&["id", "v"], &[&["A", " 100 "]]);
        let right = table(&["id", "v"], &[&["A", "100"]]);
        let result = compare(&left, &right, "id", "v", false);
        assert_eq!(result.rows[0].status, ComparisonStatus::Matched);
    }

    #[test]
    fn compare_keys_are_case_sensitive_by_default() {
        let left = table(&["id", "v"], &[&["a", "1"]]);
        let right = table(&["id", "v"], &[&["A", "1"]]);
        let result = compare(&left, &right, "id", "v", false);
        assert_eq!(result.summary.only_left, 1);
        assert_eq!(result.summary.only_right, 1);
        assert_eq!(result.summary.total, 2);
    }

    #[test]
    fn compare_keys_case_insensitive_merges_keys() {
        let left = table(&["id", "v"], &[&["a", "1"]]);
        let right = table(&["id", "v"], &[&["A", "1"]]);
        let result = compare(&left, &right, "id", "v", true);
        assert_eq!(result.summary.total, 1);
        assert_eq!(result.rows[0].status, ComparisonStatus::Matched);
        // Display key prefers the left side's original casing.
        assert_eq!(result.rows[0].key, "a");
    }

    #[test]
    fn compare_duplicate_key_last_occurrence_wins() {
        let left = table(&["id", "v"], &[&["A", "1"], &["A", "2"]]);
        let right = table(&["id", "v"], &[&["A", "2"]]);
        let result = compare(&left, &right, "id", "v", false);
        assert_eq!(result.summary.total, 1);
        assert_eq!(result.rows[0].status, ComparisonStatus::Matched);
        assert_eq!(result.rows[0].left_value.as_deref(), Some("2"));
    }

    #[test]
    fn comparison_to_table_has_four_labeled_columns() {
        let left = table(&["id", "score"], &[&["A", "10"], &["B", "20"]]);
        let right = table(&["id", "score"], &[&["A", "10"], &["C", "30"]]);
        let result = compare(&left, &right, "id", "score", false);
        let exported = comparison_to_table(&result);
        assert_eq!(
            exported.headers,
            vec!["id", "score (Left)", "score (Right)", "Status"]
        );
        assert_eq!(
            exported.rows,
            rows(&[
                &["A", "10", "10", "matched"],
                &["B", "20", "", "only left"],
                &["C", "", "30", "only right"],
            ])
        );
    }
}
