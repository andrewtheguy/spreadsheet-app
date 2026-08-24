//! The application's controller: everything the app *is* and *does*, with no reference to
//! Slint (or any other toolkit). The UI layer pushes user intent in through these methods and
//! reads the result back out through the accessors; `view` turns that into display models.
//!
//! Keeping this layer toolkit-free is why swapping the previous webview frontend for Slint
//! touched no logic: all of it lives here and in `sheet-core`.

use std::sync::Arc;

use sheet_core::{
    ComparisonPreview, ComparisonResult, CsvTable, FilterMode, FilterOptions, TablePreview,
};

/// Which of the two source panels an operation targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// Row-matching (Filter) vs VLOOKUP-style diff (Compare). Only the active mode's result is
/// computed; switching modes recomputes and drops the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OperationMode {
    #[default]
    Filter,
    Compare,
}

/// Which column a table is sorted by, and in which direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortState {
    pub column: usize,
    pub ascending: bool,
}

/// The next state when a header is clicked, cycling ascending → descending → unsorted. A click
/// on a different column starts that column ascending; `None` means "back to unsorted".
pub fn next_sort(prev: Option<SortState>, column: usize) -> Option<SortState> {
    match prev {
        Some(prev) if prev.column == column && prev.ascending => Some(SortState {
            column,
            ascending: false,
        }),
        Some(prev) if prev.column == column => None,
        _ => Some(SortState {
            column,
            ascending: true,
        }),
    }
}

/// Steps an optional selection through `0..count` treating "nothing selected" as one extra
/// slot, so repeatedly stepping walks every column and then back to no selection. `delta` is
/// normally ±1; the result wraps in both directions.
fn step_selection(current: Option<usize>, count: usize, delta: i32) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let slots = count as i32 + 1;
    let index = current.map_or(0, |column| column as i32 + 1);
    let next = (index + delta).rem_euclid(slots);
    (next > 0).then(|| next as usize - 1)
}

/// The keyboard equivalent of clicking a different header: move to the next/previous column
/// (or to unsorted) and sort it ascending. Direction is changed by [`next_sort`] instead.
fn step_sort(current: Option<SortState>, count: usize, delta: i32) -> Option<SortState> {
    step_selection(current.map(|sort| sort.column), count, delta).map(|column| SortState {
        column,
        ascending: true,
    })
}

/// A comparison result always has the same four sortable columns: key, left value, right
/// value, status — the indices `sheet_core::sort_comparison` expects.
const COMPARISON_COLUMNS: usize = 4;

/// A loaded source spreadsheet.
///
/// `table` keeps the file's original row order — filter and compare always read it, so sorting
/// for display can never change what they produce. `preview` is the bounded view of that order;
/// `sorted` holds a second preview built from a *globally* sorted copy, so the first rows shown
/// are the real first rows of the sort rather than a sort of the first 1,000.
#[derive(Default)]
pub struct Panel {
    table: Option<Arc<CsvTable>>,
    path: Option<String>,
    preview: Option<TablePreview>,
    sorted: Option<TablePreview>,
    sort: Option<SortState>,
    /// True from the moment a load is requested until the file is parsed (or the dialog is
    /// cancelled), so the UI can disable the button.
    pub loading: bool,
}

impl Panel {
    pub fn table(&self) -> Option<&Arc<CsvTable>> {
        self.table.as_ref()
    }

    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    /// The preview to render: the sorted one when a sort is active, else the original order.
    pub fn display(&self) -> Option<&TablePreview> {
        self.sorted.as_ref().or(self.preview.as_ref())
    }

    pub fn sort(&self) -> Option<SortState> {
        self.sort
    }
}

/// The latest derived result. `full` is the complete data (what an export writes), `preview` a
/// bounded view of it in computed order, and `sorted` the same view rebuilt from a globally
/// sorted copy when the user sorts a column.
struct Derived<T, P> {
    full: T,
    preview: P,
    sorted: Option<P>,
    sort: Option<SortState>,
}

impl<T, P> Derived<T, P> {
    fn new(full: T, preview: P) -> Self {
        Derived {
            full,
            preview,
            sorted: None,
            sort: None,
        }
    }

    fn display(&self) -> &P {
        self.sorted.as_ref().unwrap_or(&self.preview)
    }
}

/// Everything the app holds. Constructed once and mutated in place by the UI callbacks; every
/// mutator that changes a filter/compare input recomputes the active result before returning,
/// so the accessors are always consistent with the inputs.
pub struct AppState {
    left: Panel,
    right: Panel,

    mode: OperationMode,
    /// Index into the *right* table's headers. The header *name* is what actually drives
    /// `sheet_core::filter_rows`; the index only identifies the picker entry, which matters
    /// when headers are blank or duplicated.
    filter_column: Option<usize>,
    filter_exclude: bool,
    /// Shared by both modes, as in the original UI.
    case_insensitive: bool,

    /// Header names present in both tables — the candidate compare columns. Recomputed
    /// whenever either table changes, which clears any stale key/value selection.
    common: Vec<String>,
    key_column: Option<usize>,
    value_column: Option<usize>,

    filtered: Option<Derived<CsvTable, TablePreview>>,
    comparison: Option<Derived<ComparisonResult, ComparisonPreview>>,

    /// True while a save dialog is open or an export is being written.
    pub exporting: bool,
}

// Hand-written rather than derived because Exclude is the default filter direction, and
// `bool::default()` would silently make it Include.
impl Default for AppState {
    fn default() -> Self {
        AppState {
            left: Panel::default(),
            right: Panel::default(),
            mode: OperationMode::default(),
            filter_column: None,
            filter_exclude: true,
            case_insensitive: false,
            common: Vec::new(),
            key_column: None,
            value_column: None,
            filtered: None,
            comparison: None,
            exporting: false,
        }
    }
}

impl AppState {
    // --- Panels ---------------------------------------------------------------------

    pub fn panel(&self, side: Side) -> &Panel {
        match side {
            Side::Left => &self.left,
            Side::Right => &self.right,
        }
    }

    pub fn panel_mut(&mut self, side: Side) -> &mut Panel {
        match side {
            Side::Left => &mut self.left,
            Side::Right => &mut self.right,
        }
    }

    /// Installs a freshly parsed file into `side`, dropping any sort of the file it replaces.
    /// The filter column is cleared when the right table changes because its index refers to
    /// that table's headers, which the new file need not share.
    pub fn set_table(&mut self, side: Side, table: CsvTable, path: String) {
        let preview = TablePreview::from_table(&table);
        let panel = self.panel_mut(side);
        panel.table = Some(Arc::new(table));
        panel.path = Some(path);
        panel.preview = Some(preview);
        panel.sorted = None;
        panel.sort = None;

        if side == Side::Right {
            self.filter_column = None;
        }
        self.refresh_common_columns();
        self.recompute();
    }

    /// Swaps the two panels, tables and sorted views alike. The tables themselves are untouched
    /// — only which side points at which — so nothing is reparsed. Every column selection refers
    /// to a specific side, so all of them are cleared.
    pub fn swap(&mut self) {
        std::mem::swap(&mut self.left, &mut self.right);
        self.filter_column = None;
        self.refresh_common_columns();
        self.recompute();
    }

    /// Cycles `side`'s sort by `column` (ascending → descending → unsorted). The *full* table is
    /// sorted and re-previewed so the visible rows are the global first rows; the stored table
    /// keeps its original order, so filter and compare are unaffected.
    pub fn sort_panel(&mut self, side: Side, column: usize) {
        let next = next_sort(self.panel(side).sort, column);
        self.apply_panel_sort(side, next);
    }

    /// Keyboard equivalent of clicking the header to the left/right of the sorted one.
    pub fn step_panel_sort(&mut self, side: Side, delta: i32) {
        let count = self.panel(side).table().map_or(0, |t| t.headers.len());
        let next = step_sort(self.panel(side).sort, count, delta);
        self.apply_panel_sort(side, next);
    }

    /// Keyboard equivalent of clicking the already-sorted header again: ascending → descending
    /// → unsorted. With nothing sorted yet this starts on the first column.
    pub fn cycle_panel_sort(&mut self, side: Side) {
        if self.panel(side).table().is_none() {
            return;
        }
        let column = self.panel(side).sort.map_or(0, |sort| sort.column);
        self.sort_panel(side, column);
    }

    fn apply_panel_sort(&mut self, side: Side, sort: Option<SortState>) {
        let sorted = sort.and_then(|sort| {
            let table = self.panel(side).table()?;
            Some(TablePreview::from_table(&sheet_core::sort_rows(
                table,
                sort.column,
                sort.ascending,
            )))
        });
        let panel = self.panel_mut(side);
        panel.sort = sort;
        panel.sorted = sorted;
    }

    // --- Operation inputs -----------------------------------------------------------

    pub fn mode(&self) -> OperationMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: OperationMode) {
        self.mode = mode;
        self.recompute();
    }

    pub fn filter_column(&self) -> Option<usize> {
        self.filter_column
    }

    pub fn set_filter_column(&mut self, column: Option<usize>) {
        self.filter_column = column;
        self.recompute();
    }

    pub fn filter_exclude(&self) -> bool {
        self.filter_exclude
    }

    pub fn set_filter_exclude(&mut self, exclude: bool) {
        self.filter_exclude = exclude;
        self.recompute();
    }

    pub fn toggle_filter_exclude(&mut self) {
        self.set_filter_exclude(!self.filter_exclude);
    }

    pub fn case_insensitive(&self) -> bool {
        self.case_insensitive
    }

    pub fn set_case_insensitive(&mut self, value: bool) {
        self.case_insensitive = value;
        self.recompute();
    }

    pub fn toggle_case_insensitive(&mut self) {
        self.set_case_insensitive(!self.case_insensitive);
    }

    /// Steps the picker that Filter and Compare each lead with — the filter column, or the
    /// compare key column — so both modes share one pair of shortcuts.
    pub fn step_primary_column(&mut self, delta: i32) {
        match self.mode {
            OperationMode::Filter => {
                let count = self
                    .right
                    .table()
                    .map_or(0, |table| table.headers.len());
                self.filter_column = step_selection(self.filter_column, count, delta);
            }
            OperationMode::Compare => {
                self.key_column = step_selection(self.key_column, self.common.len(), delta);
            }
        }
        self.recompute();
    }

    /// Steps the compare value column. A no-op in Filter mode, which has only one picker.
    pub fn step_value_column(&mut self, delta: i32) {
        if self.mode == OperationMode::Compare {
            self.value_column = step_selection(self.value_column, self.common.len(), delta);
            self.recompute();
        }
    }

    /// Drops every column selection, returning both modes to their "pick a column" state.
    pub fn clear_column_selection(&mut self) {
        self.filter_column = None;
        self.key_column = None;
        self.value_column = None;
        self.recompute();
    }

    pub fn common_columns(&self) -> &[String] {
        &self.common
    }

    pub fn key_column(&self) -> Option<usize> {
        self.key_column
    }

    pub fn set_key_column(&mut self, column: Option<usize>) {
        self.key_column = column;
        self.recompute();
    }

    pub fn value_column(&self) -> Option<usize> {
        self.value_column
    }

    pub fn set_value_column(&mut self, column: Option<usize>) {
        self.value_column = column;
        self.recompute();
    }

    // --- Results --------------------------------------------------------------------

    /// The filter result to render, if the current inputs produced one.
    pub fn filtered(&self) -> Option<&TablePreview> {
        self.filtered.as_ref().map(Derived::display)
    }

    pub fn filtered_sort(&self) -> Option<SortState> {
        self.filtered.as_ref().and_then(|d| d.sort)
    }

    /// The comparison result to render, if the current inputs produced one.
    pub fn comparison(&self) -> Option<&ComparisonPreview> {
        self.comparison.as_ref().map(Derived::display)
    }

    pub fn comparison_sort(&self) -> Option<SortState> {
        self.comparison.as_ref().and_then(|d| d.sort)
    }

    /// Cycles the active result's sort by `column`, sorting the full result so the preview shows
    /// the global first rows. The unsorted copy is kept, so a third click restores it.
    pub fn sort_result(&mut self, column: usize) {
        let next = next_sort(self.result_sort(), column);
        self.apply_result_sort(next);
    }

    /// Keyboard equivalent of clicking the neighbouring header of the result table.
    pub fn step_result_sort(&mut self, delta: i32) {
        let next = step_sort(self.result_sort(), self.result_column_count(), delta);
        self.apply_result_sort(next);
    }

    /// Keyboard equivalent of re-clicking the sorted header: ascending → descending → unsorted.
    pub fn cycle_result_sort(&mut self) {
        if self.result_column_count() == 0 {
            return;
        }
        let column = self.result_sort().map_or(0, |sort| sort.column);
        self.sort_result(column);
    }

    /// Returns every table — both panels and the result — to its original order.
    pub fn clear_sorts(&mut self) {
        self.apply_panel_sort(Side::Left, None);
        self.apply_panel_sort(Side::Right, None);
        self.apply_result_sort(None);
    }

    /// The active result's sort, whichever mode is showing.
    fn result_sort(&self) -> Option<SortState> {
        match self.mode {
            OperationMode::Filter => self.filtered_sort(),
            OperationMode::Compare => self.comparison_sort(),
        }
    }

    /// How many sortable columns the active result has, or 0 when there is no result.
    fn result_column_count(&self) -> usize {
        match self.mode {
            OperationMode::Filter => self
                .filtered
                .as_ref()
                .map_or(0, |derived| derived.full.headers.len()),
            OperationMode::Compare => {
                if self.comparison.is_some() {
                    COMPARISON_COLUMNS
                } else {
                    0
                }
            }
        }
    }

    fn apply_result_sort(&mut self, sort: Option<SortState>) {
        match self.mode {
            OperationMode::Filter => {
                if let Some(derived) = self.filtered.as_mut() {
                    derived.sort = sort;
                    derived.sorted = sort.map(|sort| {
                        TablePreview::from_table(&sheet_core::sort_rows(
                            &derived.full,
                            sort.column,
                            sort.ascending,
                        ))
                    });
                }
            }
            OperationMode::Compare => {
                if let Some(derived) = self.comparison.as_mut() {
                    derived.sort = sort;
                    derived.sorted = sort.map(|sort| {
                        ComparisonPreview::from_result(&sheet_core::sort_comparison(
                            &derived.full,
                            sort.column,
                            sort.ascending,
                        ))
                    });
                }
            }
        }
    }

    /// Whether there's a non-empty result to write out.
    pub fn can_export(&self) -> bool {
        match self.mode() {
            OperationMode::Filter => self.filtered.as_ref().is_some_and(|d| !d.full.rows.is_empty()),
            OperationMode::Compare => self
                .comparison
                .as_ref()
                .is_some_and(|d| !d.full.rows.is_empty()),
        }
    }

    /// The *full* active result as a CSV table plus a suggested filename, with the on-screen
    /// sort applied so the file matches what the user is looking at. `None` when there's
    /// nothing to export.
    pub fn export_table(&self) -> Option<(CsvTable, &'static str)> {
        if !self.can_export() {
            return None;
        }
        match self.mode() {
            OperationMode::Filter => {
                let derived = self.filtered.as_ref()?;
                let table = match derived.sort {
                    Some(sort) => sheet_core::sort_rows(&derived.full, sort.column, sort.ascending),
                    None => derived.full.clone(),
                };
                Some((table, "filtered.csv"))
            }
            OperationMode::Compare => {
                let derived = self.comparison.as_ref()?;
                let result = match derived.sort {
                    Some(sort) => {
                        sheet_core::sort_comparison(&derived.full, sort.column, sort.ascending)
                    }
                    None => derived.full.clone(),
                };
                Some((sheet_core::comparison_to_table(&result), "comparison.csv"))
            }
        }
    }

    // --- Internals ------------------------------------------------------------------

    /// Recomputes the compare candidates and drops any selection that no longer applies. The
    /// previous UI cleared key/value on every table change; doing it here keeps the indices and
    /// the list they index in sync by construction.
    fn refresh_common_columns(&mut self) {
        self.common = match (self.left.table(), self.right.table()) {
            (Some(left), Some(right)) => sheet_core::common_columns(left, right),
            _ => Vec::new(),
        };
        self.key_column = None;
        self.value_column = None;
    }

    /// Rebuilds the active mode's result from the current inputs and drops the other mode's, so
    /// only one derived dataset is ever held. A result that can't be computed (a table missing,
    /// no column picked) clears to `None`, which the UI renders as a hint.
    fn recompute(&mut self) {
        self.filtered = None;
        self.comparison = None;

        let (Some(left), Some(right)) = (self.left.table().cloned(), self.right.table().cloned())
        else {
            return;
        };

        match self.mode() {
            OperationMode::Filter => {
                let Some(column) = self
                    .filter_column
                    .and_then(|index| right.headers.get(index))
                    .cloned()
                else {
                    return;
                };
                let options = FilterOptions {
                    mode: if self.filter_exclude {
                        FilterMode::Exclude
                    } else {
                        FilterMode::Include
                    },
                    case_insensitive: self.case_insensitive,
                };
                let result = sheet_core::filter_rows(&left, &right, &column, &options);
                let preview = TablePreview::from_table(&result);
                self.filtered = Some(Derived::new(result, preview));
            }
            OperationMode::Compare => {
                let (Some(key), Some(value)) = (
                    self.key_column.and_then(|i| self.common.get(i)).cloned(),
                    self.value_column.and_then(|i| self.common.get(i)).cloned(),
                ) else {
                    return;
                };
                let result = sheet_core::compare(&left, &right, &key, &value, self.case_insensitive);
                let preview = ComparisonPreview::from_result(&result);
                self.comparison = Some(Derived::new(result, preview));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(headers: &[&str], rows: &[&[&str]]) -> CsvTable {
        CsvTable {
            headers: headers.iter().map(|h| h.to_string()).collect(),
            rows: rows
                .iter()
                .map(|row| row.iter().map(|c| c.to_string()).collect())
                .collect(),
        }
    }

    fn left() -> CsvTable {
        table(
            &["sku", "qty"],
            &[&["a", "1"], &["b", "2"], &["c", "3"]],
        )
    }

    fn right() -> CsvTable {
        table(&["sku", "region"], &[&["a", "EU"], &["c", "US"]])
    }

    /// Both sides loaded, Filter mode, filtering on `sku`.
    fn loaded() -> AppState {
        let mut app = AppState::default();
        app.set_table(Side::Left, left(), "/tmp/left.csv".into());
        app.set_table(Side::Right, right(), "/tmp/right.csv".into());
        app
    }

    #[test]
    fn sort_cycles_ascending_descending_unsorted() {
        let mut sort = next_sort(None, 1);
        assert_eq!(sort, Some(SortState { column: 1, ascending: true }));
        sort = next_sort(sort, 1);
        assert_eq!(sort, Some(SortState { column: 1, ascending: false }));
        sort = next_sort(sort, 1);
        assert_eq!(sort, None);
        // A different column always restarts ascending.
        assert_eq!(
            next_sort(Some(SortState { column: 1, ascending: false }), 0),
            Some(SortState { column: 0, ascending: true })
        );
    }

    #[test]
    fn step_selection_wraps_through_an_unselected_slot() {
        assert_eq!(step_selection(None, 3, 1), Some(0));
        assert_eq!(step_selection(Some(0), 3, 1), Some(1));
        assert_eq!(step_selection(Some(2), 3, 1), None);
        assert_eq!(step_selection(None, 3, -1), Some(2));
        assert_eq!(step_selection(Some(0), 3, -1), None);
        // Nothing to step through.
        assert_eq!(step_selection(Some(0), 0, 1), None);
    }

    #[test]
    fn stepping_a_panel_sort_walks_columns_then_clears() {
        let mut app = loaded();
        assert_eq!(app.panel(Side::Left).sort(), None);

        app.step_panel_sort(Side::Left, 1);
        assert_eq!(
            app.panel(Side::Left).sort(),
            Some(SortState { column: 0, ascending: true })
        );
        app.step_panel_sort(Side::Left, 1);
        assert_eq!(
            app.panel(Side::Left).sort(),
            Some(SortState { column: 1, ascending: true })
        );
        // Past the last column is "unsorted" again.
        app.step_panel_sort(Side::Left, 1);
        assert_eq!(app.panel(Side::Left).sort(), None);
        assert!(app.panel(Side::Left).display().is_some());
    }

    #[test]
    fn cycling_a_panel_sort_flips_direction_in_place() {
        let mut app = loaded();
        app.step_panel_sort(Side::Left, 1);
        app.cycle_panel_sort(Side::Left);
        assert_eq!(
            app.panel(Side::Left).sort(),
            Some(SortState { column: 0, ascending: false })
        );
        app.cycle_panel_sort(Side::Left);
        assert_eq!(app.panel(Side::Left).sort(), None);
    }

    #[test]
    fn sorting_a_panel_does_not_disturb_the_source_order() {
        let mut app = loaded();
        app.sort_panel(Side::Left, 1);
        app.set_filter_column(Some(0));
        // The stored table is still in file order, so the filter result is too.
        let filtered = app.filtered().expect("filter result");
        assert_eq!(filtered.rows, vec![vec!["b".to_string(), "2".to_string()]]);
        assert_eq!(**app.panel(Side::Left).table().unwrap(), left());
    }

    #[test]
    fn filter_excludes_by_default_and_include_flips_it() {
        let mut app = loaded();
        app.set_filter_column(Some(0));
        assert!(app.filter_exclude());
        assert_eq!(app.filtered().unwrap().total_rows, 1);

        app.toggle_filter_exclude();
        assert!(!app.filter_exclude());
        assert_eq!(app.filtered().unwrap().total_rows, 2);
    }

    #[test]
    fn stepping_the_primary_column_follows_the_active_mode() {
        let mut app = loaded();
        // Filter mode steps the right table's columns.
        app.step_primary_column(1);
        assert_eq!(app.filter_column(), Some(0));
        assert!(app.filtered().is_some());

        app.set_mode(OperationMode::Compare);
        // Compare mode steps the shared columns instead, leaving the filter column alone.
        app.step_primary_column(1);
        assert_eq!(app.key_column(), Some(0));
        assert_eq!(app.filter_column(), Some(0));

        app.step_value_column(1);
        assert_eq!(app.value_column(), Some(0));
        assert!(app.comparison().is_some());
    }

    #[test]
    fn value_column_stepping_is_inert_in_filter_mode() {
        let mut app = loaded();
        app.step_value_column(1);
        assert_eq!(app.value_column(), None);
    }

    #[test]
    fn clearing_the_column_selection_drops_the_result() {
        let mut app = loaded();
        app.set_filter_column(Some(0));
        assert!(app.filtered().is_some());
        app.clear_column_selection();
        assert_eq!(app.filter_column(), None);
        assert!(app.filtered().is_none());
    }

    #[test]
    fn result_sort_steps_over_the_comparison_columns() {
        let mut app = loaded();
        app.set_mode(OperationMode::Compare);
        app.set_key_column(Some(0));
        app.set_value_column(Some(0));
        assert!(app.comparison().is_some());

        for column in 0..COMPARISON_COLUMNS {
            app.step_result_sort(1);
            assert_eq!(
                app.comparison_sort(),
                Some(SortState { column, ascending: true })
            );
        }
        app.step_result_sort(1);
        assert_eq!(app.comparison_sort(), None);
    }

    #[test]
    fn clear_sorts_resets_every_table() {
        let mut app = loaded();
        app.set_filter_column(Some(0));
        app.sort_panel(Side::Left, 0);
        app.sort_panel(Side::Right, 1);
        app.sort_result(0);

        app.clear_sorts();
        assert_eq!(app.panel(Side::Left).sort(), None);
        assert_eq!(app.panel(Side::Right).sort(), None);
        assert_eq!(app.filtered_sort(), None);
    }

    #[test]
    fn exporting_applies_the_on_screen_sort_to_the_full_data() {
        let mut app = loaded();
        app.set_filter_column(Some(0));
        app.set_filter_exclude(false);
        let (unsorted, name) = app.export_table().expect("export");
        assert_eq!(name, "filtered.csv");
        assert_eq!(unsorted.rows[0][0], "a");

        // Descending by sku puts "c" first in the file, matching the screen.
        app.sort_result(0);
        app.sort_result(0);
        let (sorted, _) = app.export_table().expect("export");
        assert_eq!(sorted.rows[0][0], "c");
    }

    #[test]
    fn swapping_moves_the_tables_and_clears_selections() {
        let mut app = loaded();
        app.set_filter_column(Some(0));
        app.swap();
        assert_eq!(**app.panel(Side::Left).table().unwrap(), right());
        assert_eq!(**app.panel(Side::Right).table().unwrap(), left());
        assert_eq!(app.filter_column(), None);
        assert_eq!(app.key_column(), None);
        assert!(app.filtered().is_none());
    }

    #[test]
    fn reloading_the_right_side_drops_a_filter_column_that_may_no_longer_exist() {
        let mut app = loaded();
        app.set_filter_column(Some(1));
        app.set_table(Side::Right, table(&["other"], &[&["x"]]), "/tmp/other.csv".into());
        assert_eq!(app.filter_column(), None);
        assert!(app.common_columns().is_empty());
    }

    #[test]
    fn nothing_to_export_without_a_result() {
        let mut app = AppState::default();
        assert!(!app.can_export());
        assert!(app.export_table().is_none());
        // ...and the keyboard sort actions are no-ops rather than panics.
        app.step_result_sort(1);
        app.cycle_result_sort();
        app.step_panel_sort(Side::Left, 1);
        app.cycle_panel_sort(Side::Left);
        assert_eq!(app.panel(Side::Left).sort(), None);
    }
}
