// Keeps a second console window from appearing behind the app on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! The Slint shell. Everything here is glue: it turns widget events into `state::AppState`
//! calls, and turns the `view` display models into Slint models. All spreadsheet behaviour
//! lives in `sheet-core`, and all application behaviour in `state`.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use sheet_core::CsvTable;
use slint::language::{SortOrder, TableColumn};
use slint::{Model, ModelRc, SharedString, VecModel, Weak};

mod state;
mod view;

use state::AppState;

slint::include_modules!();

/// Shared because a background parse/export thread hands its result back through
/// `Weak::upgrade_in_event_loop`, whose closure must be `Send`. Everything still *runs* on the
/// UI thread, so the mutex is never contended for long.
type Shared = Arc<Mutex<AppState>>;

/// The first entry of every column picker, so index 0 always means "nothing selected".
const PICKER_PLACEHOLDER: &str = "— Select a column —";

/// Roughly the advance width of a character at the default UI font size. Only used to turn
/// `view::ColumnView::width_chars` into an initial pixel width — columns are resizable, so
/// being a little off is cosmetic.
const CHAR_WIDTH: f32 = 7.5;
/// Horizontal padding a cell adds around its text (see `data-table.slint`).
const CELL_PADDING: f32 = 20.0;
/// Below this, a header is too narrow to click comfortably.
const MIN_COLUMN_WIDTH: f32 = 56.0;

/// How much of a source path to show under a panel title before middle-truncating it.
const PATH_CHARS: usize = 60;

/// Set on the child process when we relaunch with the software renderer, so a machine that
/// can't start either renderer fails once instead of forking forever.
const SOFTWARE_RENDERER_ENV: &str = "SPREADSHEET_APP_SOFTWARE_RENDERER";

fn main() -> ExitCode {
    let Err(error) = run_ui() else {
        return ExitCode::SUCCESS;
    };

    // Slint's default renderer needs an OpenGL 3 driver, and plenty of real Windows sessions
    // don't have one — RDP, VMs and servers among them, where it dies with "Failed to
    // initialize OpenGL driver". The failure only surfaces once the window is shown, by which
    // point Slint's platform is set process-globally and can't be swapped, so falling back to
    // the software renderer means a fresh process rather than a retry inside this one.
    if std::env::var_os(SOFTWARE_RENDERER_ENV).is_none() {
        if let Some(code) = relaunch_with_software_renderer() {
            return code;
        }
    }

    eprintln!("{error}");
    ExitCode::FAILURE
}

/// Relaunches this executable with Slint pinned to its software renderer, returning the child's
/// exit code — or `None` when the child couldn't be spawned or failed as well, so the caller
/// reports the original error instead of hiding it behind a second one.
fn relaunch_with_software_renderer() -> Option<ExitCode> {
    let exe = std::env::current_exe().ok()?;
    let status = std::process::Command::new(exe)
        .args(std::env::args_os().skip(1))
        .env(SOFTWARE_RENDERER_ENV, "1")
        .env("SLINT_BACKEND", "winit-software")
        .status()
        .ok()?;
    status.success().then_some(ExitCode::SUCCESS)
}

/// The family name of the platform's native UI font. Slint takes a single family with no
/// fallback list, so this can't be expressed in the markup; leaving it unset means the
/// renderer's font database picks for us, and on macOS that means Helvetica rather than the
/// system font. `SPREADSHEET_APP_UI_FONT` overrides it, which is handy when comparing fonts
/// during QA.
fn ui_font() -> String {
    // macOS and Windows are the only supported desktops.
    let family = if cfg!(target_os = "macos") {
        // The hidden family name of San Francisco. It resolves through the platform font
        // database, where "San Francisco" and "SF Pro" don't.
        ".SF NS"
    } else {
        "Segoe UI"
    };
    // A set-but-empty override would otherwise mean "no family", silently putting the renderer
    // default back; only a non-empty value counts.
    std::env::var("SPREADSHEET_APP_UI_FONT")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| family.to_string())
}

fn run_ui() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;
    ui.set_ui_font(ui_font().into());
    let state: Shared = Arc::new(Mutex::new(AppState::default()));

    ui.on_load_file({
        let weak = ui.as_weak();
        let state = state.clone();
        move |side| load_file(&weak, &state, core_side(side))
    });

    ui.on_export_result({
        let weak = ui.as_weak();
        let state = state.clone();
        move || export_result(&weak, &state)
    });

    ui.on_swap_sides({
        let weak = ui.as_weak();
        let state = state.clone();
        move || update(&weak, &state, |app| app.swap())
    });

    ui.on_sort_panel({
        let weak = ui.as_weak();
        let state = state.clone();
        move |side, column| {
            let side = core_side(side);
            update(&weak, &state, |app| {
                app.sort_panel(side, column.max(0) as usize)
            });
        }
    });

    ui.on_sort_result({
        let weak = ui.as_weak();
        let state = state.clone();
        move |column| update(&weak, &state, |app| app.sort_result(column.max(0) as usize))
    });

    ui.on_set_mode({
        let weak = ui.as_weak();
        let state = state.clone();
        move |mode| {
            let mode = match mode {
                OperationMode::Filter => state::OperationMode::Filter,
                OperationMode::Compare => state::OperationMode::Compare,
            };
            update(&weak, &state, |app| app.set_mode(mode));
        }
    });

    ui.on_set_filter_column({
        let weak = ui.as_weak();
        let state = state.clone();
        move |index| {
            let column = picked_column(index);
            update(&weak, &state, |app| app.set_filter_column(column));
        }
    });

    ui.on_set_filter_exclude({
        let weak = ui.as_weak();
        let state = state.clone();
        move |exclude| update(&weak, &state, |app| app.set_filter_exclude(exclude))
    });

    ui.on_set_case_insensitive({
        let weak = ui.as_weak();
        let state = state.clone();
        move |value| update(&weak, &state, |app| app.set_case_insensitive(value))
    });

    ui.on_set_key_column({
        let weak = ui.as_weak();
        let state = state.clone();
        move |index| {
            let column = picked_column(index);
            update(&weak, &state, |app| app.set_key_column(column));
        }
    });

    ui.on_set_value_column({
        let weak = ui.as_weak();
        let state = state.clone();
        move |index| {
            let column = picked_column(index);
            update(&weak, &state, |app| app.set_value_column(column));
        }
    });

    ui.on_toggle_filter_exclude({
        let weak = ui.as_weak();
        let state = state.clone();
        move || update(&weak, &state, |app| app.toggle_filter_exclude())
    });

    ui.on_toggle_case_insensitive({
        let weak = ui.as_weak();
        let state = state.clone();
        move || update(&weak, &state, |app| app.toggle_case_insensitive())
    });

    ui.on_step_primary_column({
        let weak = ui.as_weak();
        let state = state.clone();
        move |delta| update(&weak, &state, |app| app.step_primary_column(delta))
    });

    ui.on_step_value_column({
        let weak = ui.as_weak();
        let state = state.clone();
        move |delta| update(&weak, &state, |app| app.step_value_column(delta))
    });

    ui.on_clear_column_selection({
        let weak = ui.as_weak();
        let state = state.clone();
        move || update(&weak, &state, |app| app.clear_column_selection())
    });

    ui.on_step_panel_sort({
        let weak = ui.as_weak();
        let state = state.clone();
        move |side, delta| {
            let side = core_side(side);
            update(&weak, &state, |app| app.step_panel_sort(side, delta));
        }
    });

    ui.on_cycle_panel_sort({
        let weak = ui.as_weak();
        let state = state.clone();
        move |side| {
            let side = core_side(side);
            update(&weak, &state, |app| app.cycle_panel_sort(side));
        }
    });

    ui.on_step_result_sort({
        let weak = ui.as_weak();
        let state = state.clone();
        move |delta| update(&weak, &state, |app| app.step_result_sort(delta))
    });

    ui.on_cycle_result_sort({
        let weak = ui.as_weak();
        let state = state.clone();
        move || update(&weak, &state, |app| app.cycle_result_sort())
    });

    ui.on_clear_sorts({
        let weak = ui.as_weak();
        let state = state.clone();
        move || update(&weak, &state, |app| app.clear_sorts())
    });

    ui.on_clear_message({
        let weak = ui.as_weak();
        move || {
            if let Some(ui) = weak.upgrade() {
                notify(&ui, "", false);
            }
        }
    });

    refresh(&ui, &state.lock().unwrap());
    ui.run()
}

// --- Conversions ---------------------------------------------------------------------------

fn core_side(side: Side) -> state::Side {
    match side {
        Side::Left => state::Side::Left,
        Side::Right => state::Side::Right,
    }
}

/// Maps a picker selection back to a column index. Index 0 is the placeholder, so it means
/// "nothing selected".
fn picked_column(index: i32) -> Option<usize> {
    (index > 0).then(|| index as usize - 1)
}

/// The inverse: the picker row for a column index, defaulting to the placeholder.
fn picker_index(column: Option<usize>) -> i32 {
    column.map_or(0, |index| index as i32 + 1)
}

fn sort_order(sort: &view::ColumnSort) -> SortOrder {
    match sort {
        view::ColumnSort::Unsorted => SortOrder::Unsorted,
        view::ColumnSort::Ascending => SortOrder::Ascending,
        view::ColumnSort::Descending => SortOrder::Descending,
    }
}

/// The last column stretches to fill whatever width is left over, so the table doesn't leave a
/// dead strip beside it. `data-table.slint` treats a zero `width` as "unbounded", which is what
/// lets it grow; `min_width` still keeps its own content readable.
fn table_column(column: &view::ColumnView, last: bool) -> TableColumn {
    let content_width = column.width_chars as f32 * CHAR_WIDTH + CELL_PADDING;
    // `TableColumn` is `#[non_exhaustive]`, so it has to be built by mutating a default.
    let mut table_column = TableColumn::default();
    table_column.title = column.title.as_str().into();
    table_column.min_width = content_width.max(MIN_COLUMN_WIDTH);
    table_column.horizontal_stretch = if last { 1.0 } else { 0.0 };
    table_column.sort_order = sort_order(&column.sort);
    table_column.width = if last { 0.0 } else { content_width };
    table_column
}

fn strings_model(values: Vec<String>) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        values
            .iter()
            .map(|value| SharedString::from(value.as_str()))
            .collect::<Vec<_>>(),
    ))
}

fn rows_model(rows: &[view::RowView]) -> ModelRc<DataRow> {
    let items: Vec<DataRow> = rows
        .iter()
        .map(|row| DataRow {
            cells: strings_model(row.cells.clone()),
            tint: match row.tint {
                view::RowTint::None => RowTint::None,
                view::RowTint::Diff => RowTint::Diff,
                view::RowTint::OnlyLeft => RowTint::OnlyLeft,
                view::RowTint::OnlyRight => RowTint::OnlyRight,
            },
        })
        .collect();
    ModelRc::new(VecModel::from(items))
}

/// Reconciles a table's column model with `wanted`.
///
/// When the columns are the same ones as last time, only the sort indicator can have changed —
/// updating it in place preserves any width the user dragged. Returns a replacement model only
/// when the columns themselves changed (a new file, a switch between Filter and Compare).
fn sync_columns(
    current: &ModelRc<TableColumn>,
    wanted: &[view::ColumnView],
) -> Option<ModelRc<TableColumn>> {
    let same_columns = current.row_count() == wanted.len()
        && wanted.iter().enumerate().all(|(index, column)| {
            current
                .row_data(index)
                .is_some_and(|existing| existing.title == column.title.as_str())
        });

    if !same_columns {
        let last = wanted.len().saturating_sub(1);
        return Some(ModelRc::new(VecModel::from(
            wanted
                .iter()
                .enumerate()
                .map(|(index, column)| table_column(column, index == last))
                .collect::<Vec<_>>(),
        )));
    }

    for (index, column) in wanted.iter().enumerate() {
        if let Some(mut existing) = current.row_data(index) {
            let order = sort_order(&column.sort);
            if existing.sort_order != order {
                existing.sort_order = order;
                current.set_row_data(index, existing);
            }
        }
    }
    None
}

// --- Rendering -----------------------------------------------------------------------------

/// Pushes the whole of `app` onto the window. Cheap enough to run after every interaction (the
/// models are capped at `sheet_core::MAX_PREVIEW_ROWS`), which keeps the UI a pure function of
/// the state rather than a pile of incremental updates.
fn refresh(ui: &MainWindow, app: &AppState) {
    refresh_panel(ui, app, state::Side::Left);
    refresh_panel(ui, app, state::Side::Right);
    ui.set_can_swap(
        app.panel(state::Side::Left).table().is_some()
            || app.panel(state::Side::Right).table().is_some(),
    );

    ui.set_mode(match app.mode() {
        state::OperationMode::Filter => OperationMode::Filter,
        state::OperationMode::Compare => OperationMode::Compare,
    });
    ui.set_case_insensitive(app.case_insensitive());
    ui.set_filter_exclude(app.filter_exclude());
    ui.set_exporting(app.exporting);
    ui.set_can_export(app.can_export());

    // The filter picker lists the *right* table's columns; the compare pickers list the names
    // the two tables share. Both are prefixed with the placeholder.
    let mut filter_columns = vec![PICKER_PLACEHOLDER.to_string()];
    if let Some(right) = app.panel(state::Side::Right).table() {
        filter_columns.extend(view::display_headers(&right.headers));
    }
    ui.set_filter_columns(strings_model(filter_columns));
    ui.set_filter_column_index(picker_index(app.filter_column()));

    let mut common = vec![PICKER_PLACEHOLDER.to_string()];
    common.extend(app.common_columns().iter().map(|name| {
        if name.trim().is_empty() {
            "(empty header)".to_string()
        } else {
            name.clone()
        }
    }));
    ui.set_common_columns(strings_model(common));
    ui.set_key_column_index(picker_index(app.key_column()));
    ui.set_value_column_index(picker_index(app.value_column()));

    let result = match app.mode() {
        state::OperationMode::Filter => app
            .filtered()
            .map(|preview| view::table_view(preview, app.filtered_sort())),
        state::OperationMode::Compare => app
            .comparison()
            .map(|preview| view::comparison_view(preview, app.comparison_sort())),
    };
    match result {
        Some(table) => {
            ui.set_result_has_table(true);
            ui.set_result_status(table.status.as_str().into());
            ui.set_result_rows(rows_model(&table.rows));
            if let Some(model) = sync_columns(&ui.get_result_columns(), &table.columns) {
                ui.set_result_columns(model);
            }
        }
        None => {
            ui.set_result_has_table(false);
            ui.set_result_status(SharedString::new());
            ui.set_result_rows(rows_model(&[]));
        }
    }
    ui.set_result_hint(view::result_hint(app).into());

    match app.comparison() {
        Some(preview) => {
            let summary = &preview.summary;
            ui.set_show_summary(true);
            ui.set_summary_total(format!("Total {}", view::thousands(summary.total)).into());
            ui.set_summary_matched(format!("Matched {}", view::thousands(summary.matched)).into());
            ui.set_summary_diff(format!("Diff {}", view::thousands(summary.diff)).into());
            ui.set_summary_only_left(
                format!("Only Left {}", view::thousands(summary.only_left)).into(),
            );
            ui.set_summary_only_right(
                format!("Only Right {}", view::thousands(summary.only_right)).into(),
            );
        }
        None => ui.set_show_summary(false),
    }
}

fn refresh_panel(ui: &MainWindow, app: &AppState, side: state::Side) {
    let panel = app.panel(side);
    let path = panel
        .path()
        .map(|path| view::truncate_path(path, PATH_CHARS))
        .unwrap_or_default();
    let table = panel
        .display()
        .map(|preview| view::table_view(preview, panel.sort()));
    let (columns, rows, status) = match &table {
        Some(table) => (
            table.columns.as_slice(),
            rows_model(&table.rows),
            table.status.as_str(),
        ),
        None => (&[][..], rows_model(&[]), ""),
    };

    match side {
        state::Side::Left => {
            ui.set_left_path(path.into());
            ui.set_left_loading(panel.loading);
            ui.set_left_has_table(panel.table().is_some());
            ui.set_left_status(status.into());
            ui.set_left_rows(rows);
            if let Some(model) = sync_columns(&ui.get_left_columns(), columns) {
                ui.set_left_columns(model);
            }
        }
        state::Side::Right => {
            ui.set_right_path(path.into());
            ui.set_right_loading(panel.loading);
            ui.set_right_has_table(panel.table().is_some());
            ui.set_right_status(status.into());
            ui.set_right_rows(rows);
            if let Some(model) = sync_columns(&ui.get_right_columns(), columns) {
                ui.set_right_columns(model);
            }
        }
    }
}

/// Shows a transient message in the header strip. An empty `message` hides it; `MainWindow`'s
/// timer clears it on its own after a few seconds.
fn notify(ui: &MainWindow, message: &str, is_error: bool) {
    ui.set_message(message.into());
    ui.set_message_is_error(is_error);
}

/// Mutates the state and re-renders, holding the lock for exactly one turn.
fn update(weak: &Weak<MainWindow>, state: &Shared, mutate: impl FnOnce(&mut AppState)) {
    let Some(ui) = weak.upgrade() else {
        return;
    };
    let mut app = state.lock().unwrap();
    mutate(&mut app);
    refresh(&ui, &app);
}

// --- File I/O ------------------------------------------------------------------------------

enum Kind {
    Csv,
    Excel,
}

/// Parses a spreadsheet, dispatching on the extension. Excel and CSV both land as a `CsvTable`;
/// everything else is rejected up front rather than mis-parsed.
fn parse_file(path: &Path) -> Result<CsvTable, String> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    let kind = match extension.as_deref() {
        Some("csv") => Kind::Csv,
        Some("xlsx" | "xls") => Kind::Excel,
        _ => {
            return Err(format!(
                "unsupported file type for {}; expected .csv, .xlsx, or .xls",
                path.display()
            ))
        }
    };
    let file = File::open(path).map_err(|e| format!("failed to open {}: {e}", path.display()))?;
    match kind {
        Kind::Csv => sheet_core::parse_csv(file)
            .map_err(|e| format!("failed to read {}: {e}", path.display())),
        Kind::Excel => sheet_core::parse_excel(file)
            .map_err(|e| format!("failed to read {}: {e}", path.display())),
    }
}

/// Opens a file picker for `side` and, once a file is chosen, parses it off the UI thread.
///
/// The dialog is driven by `slint::spawn_local` (rfd runs it as a sheet on the app's window
/// rather than a nested run loop), and the parse runs on its own thread so a large workbook
/// doesn't freeze the window — the result comes back via `upgrade_in_event_loop`.
fn load_file(weak: &Weak<MainWindow>, state: &Shared, side: state::Side) {
    {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let mut app = state.lock().unwrap();
        // A dialog for this side is already open.
        if app.panel(side).loading {
            return;
        }
        app.panel_mut(side).loading = true;
        refresh(&ui, &app);
    }

    let weak = weak.clone();
    let state = state.clone();
    // Only fails when the event loop has already stopped, in which case there is no window
    // left to reset the flag for.
    let _ = slint::spawn_local(async move {
        let picked = rfd::AsyncFileDialog::new()
            .set_title("Open spreadsheet")
            .add_filter("CSV and Excel", &["csv", "xlsx", "xls"])
            .pick_file()
            .await;

        let Some(handle) = picked else {
            // Cancelled — keep whatever the side already had.
            update(&weak, &state, |app| app.panel_mut(side).loading = false);
            return;
        };

        let path = handle.path().to_path_buf();
        std::thread::spawn(move || {
            let parsed = parse_file(&path);
            let _ = weak.upgrade_in_event_loop(move |ui| {
                let mut app = state.lock().unwrap();
                app.panel_mut(side).loading = false;
                match parsed {
                    Ok(table) => {
                        app.set_table(side, table, path.display().to_string());
                        notify(&ui, "", false);
                    }
                    Err(error) => notify(&ui, &error, true),
                }
                refresh(&ui, &app);
            });
        });
    });
}

/// Writes the full active result (not the preview) to a file the user picks, in the same order
/// it's shown on screen.
fn export_result(weak: &Weak<MainWindow>, state: &Shared) {
    let export = {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let mut app = state.lock().unwrap();
        if app.exporting {
            return;
        }
        let Some(export) = app.export_table() else {
            return;
        };
        app.exporting = true;
        refresh(&ui, &app);
        export
    };
    let (table, default_name) = export;

    let weak = weak.clone();
    let state = state.clone();
    let _ = slint::spawn_local(async move {
        let picked = rfd::AsyncFileDialog::new()
            .set_title("Export result")
            .add_filter("CSV", &["csv"])
            .set_file_name(default_name)
            .save_file()
            .await;

        let Some(handle) = picked else {
            update(&weak, &state, |app| app.exporting = false);
            return;
        };

        let path = handle.path().to_path_buf();
        std::thread::spawn(move || {
            let written = write_csv(&path, &table);
            let _ = weak.upgrade_in_event_loop(move |ui| {
                let mut app = state.lock().unwrap();
                app.exporting = false;
                match written {
                    Ok(()) => notify(&ui, &format!("Saved {}", path.display()), false),
                    Err(error) => notify(&ui, &error, true),
                }
                refresh(&ui, &app);
            });
        });
    });
}

fn write_csv(path: &PathBuf, table: &CsvTable) -> Result<(), String> {
    let file =
        File::create(path).map_err(|e| format!("failed to create {}: {e}", path.display()))?;
    sheet_core::write_csv(file, table)
        .map_err(|e| format!("failed to write {}: {e}", path.display()))
}
