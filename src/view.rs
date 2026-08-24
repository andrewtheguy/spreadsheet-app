//! Display models: the presentation rules that used to live in the TypeScript frontend, moved
//! into Rust so any UI can render the app by walking these plain structs. Nothing here knows
//! about Slint — `main` maps a [`TableView`] onto Slint's models and picks the actual colours.

use sheet_core::{ComparisonPreview, ComparisonStatus, TablePreview};

use crate::state::{AppState, OperationMode, Side, SortState, TableTarget};

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
    // the filename itself rather than producing something longer than `max_chars`. The tail is
    // clamped to the filename's own length: this branch also catches the case where the filename
    // is *just* short enough to fit but leaves no room for a directory, and taking `max_chars - 3`
    // characters from it would run off the front.
    if file_name.len() + 4 >= max_chars {
        let tail_len = max_chars.saturating_sub(3).min(file_name.len());
        let tail: String = file_name[file_name.len() - tail_len..].iter().collect();
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
                sheet_core::status_label(row.status).to_string(),
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

/// The view rendered for `target` right now, if it has a table. This is the same construction
/// the UI paints from, so display-row indices coming back from clicks line up with the rows the
/// copy functions below extract.
pub fn target_view(state: &AppState, target: TableTarget) -> Option<TableView> {
    match target {
        TableTarget::Panel(side) => {
            let panel = state.panel(side);
            panel
                .display()
                .map(|preview| table_view(preview, panel.sort()))
        }
        TableTarget::Result => match state.mode() {
            OperationMode::Filter => state
                .filtered()
                .map(|preview| table_view(preview, state.filtered_sort())),
            OperationMode::Compare => state
                .comparison()
                .map(|preview| comparison_view(preview, state.comparison_sort())),
        },
    }
}

/// How many rows and columns `target` actually displays (blank rows dropped, preview capped)
/// — the space cell selections live in.
pub fn display_size(state: &AppState, target: TableTarget) -> (usize, usize) {
    target_view(state, target).map_or((0, 0), |table| (table.rows.len(), table.columns.len()))
}

/// The table Copy Table and the keyboard-selection actions fall back to when nothing is
/// selected: the result when it's showing, else the first panel that displays rows. Without
/// this, loading a single file would leave every keyboard selection action a silent no-op.
pub fn default_copy_target(state: &AppState) -> Option<TableTarget> {
    [
        TableTarget::Result,
        TableTarget::Panel(Side::Left),
        TableTarget::Panel(Side::Right),
    ]
    .into_iter()
    .find(|target| display_size(state, *target).0 > 0)
}

/// A cell as clipboard text. Cells containing a tab, newline, or quote are quoted the way Excel
/// quotes them on the clipboard, so pasting reproduces the cell instead of splitting the grid.
fn tsv_cell(cell: &str) -> String {
    if cell.contains(['\t', '\n', '\r', '"']) {
        format!("\"{}\"", cell.replace('"', "\"\""))
    } else {
        cell.to_string()
    }
}

/// Rows as tab-separated lines, the format both Excel and Numbers paste as a grid.
fn tsv<'a>(rows: impl Iterator<Item = &'a [String]>) -> String {
    rows.map(|row| {
        row.iter()
            .map(|cell| tsv_cell(cell))
            .collect::<Vec<_>>()
            .join("\t")
    })
    .collect::<Vec<_>>()
    .join("\n")
}

/// "1 row" / "48,120 rows" — the status-strip label for a row-shaped copy.
fn rows_label(count: usize) -> String {
    if count == 1 {
        "1 row".to_string()
    } else {
        format!("{} rows", thousands(count))
    }
}

/// The selected cells as tab-separated text, plus a status-strip label: "1 cell" for a single
/// cell, "N rows" when the selection spans every column, "N cells" for a narrower rectangle.
/// `None` when nothing is selected or the selection no longer intersects its table.
pub fn selection_tsv(state: &AppState) -> Option<(String, String)> {
    let selection = state.selection()?;
    let table = target_view(state, selection.target)?;
    let (first_row, last_row) = selection.row_bounds();
    let last_row = last_row.min(table.rows.len().checked_sub(1)?);
    let (first_column, last_column) = selection.column_bounds();
    let last_column = last_column.min(table.columns.len().checked_sub(1)?);
    if first_row > last_row || first_column > last_column {
        return None;
    }

    // Sliced per row because CSV rows can be ragged; a row shorter than the selection just
    // contributes the cells it has.
    let text = tsv(table.rows[first_row..=last_row].iter().map(|row| {
        let end = (last_column + 1).min(row.cells.len());
        &row.cells[first_column.min(end)..end]
    }));
    let rows = last_row - first_row + 1;
    let cells = rows * (last_column - first_column + 1);
    let label = if cells == 1 {
        "1 cell".to_string()
    } else if (first_column, last_column) == (0, table.columns.len() - 1) {
        rows_label(rows)
    } else {
        format!("{} cells", thousands(cells))
    };
    Some((text, label))
}

/// The whole displayed table for `target` as tab-separated text, headers first, plus the
/// status-strip label for its row count (headers not counted).
pub fn table_tsv(state: &AppState, target: TableTarget) -> Option<(String, String)> {
    let table = target_view(state, target)?;
    let headers: Vec<String> = table
        .columns
        .iter()
        .map(|column| column.title.clone())
        .collect();
    let label = rows_label(table.rows.len());
    let text = tsv(
        std::iter::once(headers.as_slice())
            .chain(table.rows.iter().map(|row| row.cells.as_slice())),
    );
    Some((text, label))
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
    fn filename_lengths_around_the_boundary_dont_overflow() {
        // At exactly `max_chars - 4` the filename takes the truncate-the-filename branch while
        // being shorter than the `max_chars - 3` characters that branch wants to keep. Sweep the
        // whole neighbourhood, since only one length used to underflow.
        const MAX: usize = 40;
        for name_len in 1..=(MAX + 8) {
            let file_name = format!("{}.csv", "x".repeat(name_len.saturating_sub(4)));
            let path = format!("/abcd/{file_name}");
            let out = truncate_path(&path, MAX);
            assert!(
                out.chars().count() <= MAX,
                "{out} ({} chars) exceeds {MAX} for a {}-char filename",
                out.chars().count(),
                file_name.chars().count()
            );
        }
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
    fn tsv_quotes_cells_that_would_break_the_grid() {
        assert_eq!(tsv_cell("plain"), "plain");
        assert_eq!(tsv_cell("a\tb"), "\"a\tb\"");
        assert_eq!(tsv_cell("two\nlines"), "\"two\nlines\"");
        assert_eq!(tsv_cell("say \"hi\""), "\"say \"\"hi\"\"\"");
    }

    fn csv(headers: &[&str], rows: &[&[&str]]) -> sheet_core::CsvTable {
        sheet_core::CsvTable {
            headers: headers.iter().map(|h| h.to_string()).collect(),
            rows: rows
                .iter()
                .map(|row| row.iter().map(|c| c.to_string()).collect())
                .collect(),
        }
    }

    fn app_with_left(rows: &[&[&str]]) -> AppState {
        let mut app = AppState::default();
        app.set_table(
            crate::state::Side::Left,
            csv(&["sku", "qty"], rows),
            "/tmp/left.csv".into(),
        );
        app
    }

    const LEFT: TableTarget = TableTarget::Panel(crate::state::Side::Left);

    #[test]
    fn selection_tsv_extracts_the_selected_rectangle() {
        let mut app = app_with_left(&[&["a", "1"], &["b", "2"], &["c", "3"]]);

        // A single cell.
        app.select_cell(LEFT, 1, 1, false);
        let (text, label) = selection_tsv(&app).expect("cell");
        assert_eq!(text, "2");
        assert_eq!(label, "1 cell");

        // Extended to a full-width range, which reads as rows.
        app.select_cell(LEFT, 2, 0, true);
        let (text, label) = selection_tsv(&app).expect("rows");
        assert_eq!(text, "b\t2\nc\t3");
        assert_eq!(label, "2 rows");

        // A single-column rectangle stays cell-labelled.
        app.select_cell(LEFT, 0, 0, false);
        app.select_cell(LEFT, 1, 0, true);
        let (text, label) = selection_tsv(&app).expect("column");
        assert_eq!(text, "a\nb");
        assert_eq!(label, "2 cells");
    }

    #[test]
    fn selection_rows_are_display_rows_not_data_rows() {
        // The blank data row is hidden, so display row 1 is the *third* data row.
        let mut app = app_with_left(&[&["a", "1"], &["", "  "], &["c", "3"]]);
        app.select_cell(LEFT, 1, 0, false);
        app.select_cell(LEFT, 1, 1, true);
        let (text, label) = selection_tsv(&app).expect("selection");
        assert_eq!(label, "1 row");
        assert_eq!(text, "c\t3");
    }

    #[test]
    fn a_selection_past_the_table_copies_nothing() {
        let mut app = app_with_left(&[&["a", "1"]]);
        app.select_cell(LEFT, 5, 0, false);
        assert!(selection_tsv(&app).is_none());
    }

    #[test]
    fn a_selection_wider_than_the_table_clamps_to_it() {
        let mut app = app_with_left(&[&["a", "1"], &["b", "2"]]);
        app.select_cell(LEFT, 0, 1, false);
        app.select_cell(LEFT, 1, 9, true);
        let (text, label) = selection_tsv(&app).expect("selection");
        assert_eq!(text, "1\n2");
        assert_eq!(label, "2 cells");
    }

    #[test]
    fn table_tsv_leads_with_headers() {
        let app = app_with_left(&[&["a", "1"], &["b", "2"]]);
        let (text, label) = table_tsv(&app, LEFT).expect("table");
        assert_eq!(label, "2 rows");
        assert_eq!(text, "sku\tqty\na\t1\nb\t2");
    }

    #[test]
    fn copy_falls_back_to_the_result_then_the_loaded_panels() {
        let mut app = AppState::default();
        assert_eq!(default_copy_target(&app), None);

        app.set_table(
            crate::state::Side::Right,
            csv(&["sku", "region"], &[&["b", "EU"]]),
            "/tmp/right.csv".into(),
        );
        assert_eq!(
            default_copy_target(&app),
            Some(TableTarget::Panel(crate::state::Side::Right))
        );

        app.set_table(
            crate::state::Side::Left,
            csv(&["sku", "qty"], &[&["a", "1"], &["b", "2"]]),
            "/tmp/left.csv".into(),
        );
        assert_eq!(default_copy_target(&app), Some(LEFT));

        // Once a (non-empty) result is showing, it wins.
        app.set_filter_column(Some(0));
        assert_eq!(default_copy_target(&app), Some(TableTarget::Result));
    }

    #[test]
    fn copying_an_unloaded_table_yields_nothing() {
        let app = AppState::default();
        assert!(table_tsv(&app, TableTarget::Result).is_none());
        assert!(selection_tsv(&app).is_none());
        assert_eq!(display_size(&app, LEFT), (0, 0));
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
