//! Display models: the presentation rules that used to live in the TypeScript frontend, moved
//! into Rust so any UI can render the app by walking these plain structs. Nothing here knows
//! about Slint — `main` maps a [`TableView`] onto Slint's models and picks the actual colours.

use sheet_core::{ComparisonPreview, ComparisonStatus, TablePreview};

use crate::state::{AppState, OperationMode, SortState};

/// A column header as rendered: its label, its sort indicator, and how wide it wants to be.
pub struct ColumnView {
    pub title: String,
    pub sort: ColumnSort,
    /// Roughly how many characters the widest cell in this column needs. The UI turns this into
    /// pixels, since only it knows the font.
    pub width_chars: usize,
}

pub enum ColumnSort {
    Unsorted,
    Ascending,
    Descending,
}

/// Why a row is tinted. `None` for ordinary rows; the rest mirror the comparison statuses that
/// the old UI shaded red / orange / blue.
pub enum RowTint {
    None,
    Diff,
    OnlyLeft,
    OnlyRight,
}

pub struct RowView {
    pub cells: Vec<String>,
    pub tint: RowTint,
}

/// Everything needed to draw one table.
pub struct TableView {
    pub columns: Vec<ColumnView>,
    pub rows: Vec<RowView>,
    /// The row-count line under the table, e.g. `"Showing 1,000 of 48,120 rows"`.
    pub status: String,
}

/// Widest column we'll size to up front; anything longer is elided and can be dragged wider.
const MAX_WIDTH_CHARS: usize = 40;
/// Narrowest, so a column of single characters still has a clickable header.
const MIN_WIDTH_CHARS: usize = 8;

/// A row is treated as blank when every cell is empty or whitespace-only. Blank rows are hidden
/// in the rendered tables (display-only) — the underlying data keeps them, so filtering,
/// comparing, and exporting all still see them.
fn is_blank_row(row: &[String]) -> bool {
    row.iter().all(|cell| cell.trim().is_empty())
}

/// Empty/whitespace-only headers render as `(Empty column N)` using their 1-based position, so
/// blank columns stay identifiable and pickable.
pub fn display_headers(headers: &[String]) -> Vec<String> {
    headers
        .iter()
        .enumerate()
        .map(|(index, header)| {
            if header.trim().is_empty() {
                format!("(Empty column {})", index + 1)
            } else {
                header.clone()
            }
        })
        .collect()
}

/// Groups digits in threes, standing in for JavaScript's `toLocaleString` in the old frontend.
pub fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Middle-truncates a long file path to `max_chars`, keeping the filename intact. Handles both
/// POSIX (`/`) and Windows (`\`) separators.
pub fn truncate_path(path: &str, max_chars: usize) -> String {
    let chars: Vec<char> = path.chars().collect();
    if chars.len() <= max_chars {
        return path.to_string();
    }

    let separator = path.rfind(['/', '\\']);
    // Rejoin with whatever separator the path actually used, so a Windows path doesn't come
    // back with a stray forward slash.
    let separator_char = separator.map_or('/', |index| path[index..].chars().next().unwrap());
    let file_name: Vec<char> = match separator {
        Some(index) => path[index + 1..].chars().collect(),
        None => chars.clone(),
    };
    let dir: Vec<char> = match separator {
        Some(index) => path[..index].chars().collect(),
        None => Vec::new(),
    };

    // When the filename alone won't fit there's no room for any directory context, so truncate
    // the filename itself rather than producing something longer than `max_chars`.
    if file_name.len() + 4 >= max_chars {
        let tail: String = file_name[file_name.len() - (max_chars - 3)..].iter().collect();
        return format!("...{tail}");
    }

    if dir.is_empty() {
        return format!("...{separator_char}{}", file_name.iter().collect::<String>());
    }

    let keep = (max_chars - file_name.len() - 4) / 2;
    let start: String = dir[..keep].iter().collect();
    let end: String = dir[dir.len() - keep..].iter().collect();
    format!(
        "{start}...{end}{separator_char}{}",
        file_name.iter().collect::<String>()
    )
}

/// The row-count line. `visible` is what's on screen after blank rows are dropped and the
/// preview cap is applied; `total` is the true row count of the full dataset behind it.
fn status_line(visible: usize, total: usize) -> String {
    if total == 0 {
        "No rows".to_string()
    } else if visible == total {
        format!("{} rows", thousands(total))
    } else {
        format!("Showing {} of {} rows", thousands(visible), thousands(total))
    }
}

fn column_sort(sort: Option<SortState>, column: usize) -> ColumnSort {
    match sort {
        Some(sort) if sort.column == column && sort.ascending => ColumnSort::Ascending,
        Some(sort) if sort.column == column => ColumnSort::Descending,
        _ => ColumnSort::Unsorted,
    }
}

/// Sizes each column to the widest thing it has to show, clamped so neither a one-character
/// column nor a paragraph-length cell distorts the layout.
fn column_widths(titles: &[String], rows: &[RowView]) -> Vec<usize> {
    let mut widths: Vec<usize> = titles.iter().map(|t| t.chars().count()).collect();
    for row in rows {
        for (index, cell) in row.cells.iter().enumerate() {
            if let Some(width) = widths.get_mut(index) {
                *width = (*width).max(cell.chars().count());
            }
        }
    }
    widths
        .into_iter()
        .map(|width| width.clamp(MIN_WIDTH_CHARS, MAX_WIDTH_CHARS))
        .collect()
}

fn build(titles: Vec<String>, rows: Vec<RowView>, sort: Option<SortState>, total: usize) -> TableView {
    let widths = column_widths(&titles, &rows);
    let status = status_line(rows.len(), total);
    TableView {
        columns: titles
            .into_iter()
            .zip(widths)
            .enumerate()
            .map(|(index, (title, width_chars))| ColumnView {
                title,
                sort: column_sort(sort, index),
                width_chars,
            })
            .collect(),
        rows,
        status,
    }
}

/// Renders a source or filter-result preview.
pub fn table_view(preview: &TablePreview, sort: Option<SortState>) -> TableView {
    let rows: Vec<RowView> = preview
        .rows
        .iter()
        .filter(|row| !is_blank_row(row))
        .map(|row| RowView {
            cells: row.clone(),
            tint: RowTint::None,
        })
        .collect();
    build(
        display_headers(&preview.headers),
        rows,
        sort,
        preview.total_rows,
    )
}

/// The label shown in the Status column for each comparison outcome.
fn status_label(status: ComparisonStatus) -> &'static str {
    match status {
        ComparisonStatus::Matched => "Matched",
        ComparisonStatus::Diff => "Diff",
        ComparisonStatus::OnlyLeft => "Only Left",
        ComparisonStatus::OnlyRight => "Only Right",
    }
}

/// Renders a comparison preview as a four-column table: key, left value, right value, status.
/// Column indices here are what `sheet_core::sort_comparison` expects, so a header click maps
/// straight through.
pub fn comparison_view(preview: &ComparisonPreview, sort: Option<SortState>) -> TableView {
    let titles = vec![
        preview.key_column.clone(),
        format!("{} (Left)", preview.value_column),
        format!("{} (Right)", preview.value_column),
        "Status".to_string(),
    ];
    let rows: Vec<RowView> = preview
        .rows
        .iter()
        .map(|row| RowView {
            cells: vec![
                row.key.clone(),
                row.left_value.clone().unwrap_or_default(),
                row.right_value.clone().unwrap_or_default(),
                status_label(row.status).to_string(),
            ],
            tint: match row.status {
                ComparisonStatus::Matched => RowTint::None,
                ComparisonStatus::Diff => RowTint::Diff,
                ComparisonStatus::OnlyLeft => RowTint::OnlyLeft,
                ComparisonStatus::OnlyRight => RowTint::OnlyRight,
            },
        })
        .collect();
    build(titles, rows, sort, preview.total_rows)
}

/// The italic placeholder shown where the result table would be, explaining what's still needed.
pub fn result_hint(state: &AppState) -> &'static str {
    let both_loaded =
        state.panel(crate::state::Side::Left).table().is_some() && state.panel(crate::state::Side::Right).table().is_some();
    match state.mode() {
        OperationMode::Filter => {
            if both_loaded {
                "Pick a column from the right spreadsheet to filter the left spreadsheet."
            } else {
                "Load both CSV / Excel files to filter."
            }
        }
        OperationMode::Compare => {
            if !both_loaded {
                "Load both CSV / Excel files to compare."
            } else if state.common_columns().is_empty() {
                "The two spreadsheets share no columns to compare."
            } else {
                "Pick key and value columns to compare."
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands_groups_digits() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000), "1,000");
        assert_eq!(thousands(48_120), "48,120");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn blank_headers_get_positional_labels() {
        let headers = vec!["a".into(), "  ".into(), "c".into()];
        assert_eq!(
            display_headers(&headers),
            vec!["a", "(Empty column 2)", "c"]
        );
    }

    #[test]
    fn short_paths_are_left_alone() {
        assert_eq!(truncate_path("/tmp/a.csv", 50), "/tmp/a.csv");
    }

    #[test]
    fn long_paths_keep_the_filename() {
        let path = "/Users/someone/a/very/deeply/nested/directory/tree/report.csv";
        let out = truncate_path(path, 40);
        assert!(out.ends_with("/report.csv"), "{out}");
        assert!(out.chars().count() <= 40, "{out}");
    }

    #[test]
    fn windows_separators_are_handled() {
        let path = r"C:\Users\someone\a\very\deeply\nested\directory\tree\report.csv";
        let out = truncate_path(path, 40);
        assert!(out.ends_with(r"\report.csv"), "{out}");
    }

    #[test]
    fn an_over_long_filename_is_truncated_itself() {
        let path = format!("/tmp/{}.csv", "x".repeat(80));
        let out = truncate_path(&path, 30);
        assert_eq!(out.chars().count(), 30);
        assert!(out.starts_with("..."), "{out}");
    }

    #[test]
    fn status_line_reports_capping() {
        assert_eq!(status_line(0, 0), "No rows");
        assert_eq!(status_line(12, 12), "12 rows");
        assert_eq!(status_line(1_000, 48_120), "Showing 1,000 of 48,120 rows");
    }

    #[test]
    fn blank_rows_are_hidden_but_still_counted() {
        let preview = TablePreview {
            headers: vec!["a".into(), "b".into()],
            rows: vec![
                vec!["1".into(), "2".into()],
                vec!["".into(), "   ".into()],
                vec!["3".into(), "4".into()],
            ],
            total_rows: 3,
        };
        let view = table_view(&preview, None);
        assert_eq!(view.rows.len(), 2);
        // The blank row is gone from the display but the count still reflects the real data.
        assert_eq!(view.status, "Showing 2 of 3 rows");
    }

    #[test]
    fn column_widths_are_clamped() {
        let preview = TablePreview {
            headers: vec!["a".into(), "b".into()],
            rows: vec![vec!["x".into(), "y".repeat(500)]],
            total_rows: 1,
        };
        let view = table_view(&preview, None);
        assert_eq!(view.columns[0].width_chars, MIN_WIDTH_CHARS);
        assert_eq!(view.columns[1].width_chars, MAX_WIDTH_CHARS);
    }
}
