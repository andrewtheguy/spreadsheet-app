//! Pure spreadsheet logic — CSV parsing, serializing, and row matching — with no UI, dialog,
//! or filesystem-dialog dependencies, so it can be unit-tested in isolation and reused from
//! any front end. The app currently drives it from Slint; nothing here knows that.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read, Seek, SeekFrom, Write};

use calamine::{open_workbook_auto_from_rs, Data, Reader};
use quick_xml::events::{BytesStart, Event};

/// A CSV file parsed into a header row plus data rows. Held by the app's controller and used by
/// all the row-matching logic; the UI only ever receives a bounded [`TablePreview`] of it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CsvTable {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// Maximum rows shipped to the UI for display. Building a view model for every row of an
/// extremely large table would cost far more than anyone can scroll through. The full table
/// stays with the controller, which is what sort/filter/compare/export actually operate on.
pub const MAX_PREVIEW_ROWS: usize = 1000;

/// A bounded view of a [`CsvTable`] for the UI: the first [`MAX_PREVIEW_ROWS`] rows plus the
/// true total, so the UI renders quickly and can still show "first 1,000 of N rows".
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TablePreview {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub total_rows: usize,
}

impl TablePreview {
    /// Builds a preview of `table`, copying at most [`MAX_PREVIEW_ROWS`] rows and recording the
    /// full row count in `total_rows`.
    pub fn from_table(table: &CsvTable) -> Self {
        TablePreview {
            headers: table.headers.clone(),
            rows: table.rows.iter().take(MAX_PREVIEW_ROWS).cloned().collect(),
            total_rows: table.rows.len(),
        }
    }
}

/// A bounded view of a [`ComparisonResult`] for the UI, mirroring [`TablePreview`]. The
/// `summary` is computed over the full result, so its tallies stay accurate even when `rows`
/// is capped; `total_rows` is the full row count.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComparisonPreview {
    pub rows: Vec<ComparisonRow>,
    pub key_column: String,
    pub value_column: String,
    pub summary: ComparisonSummary,
    pub total_rows: usize,
}

impl ComparisonPreview {
    /// Builds a preview of `result`, copying at most [`MAX_PREVIEW_ROWS`] rows and recording the
    /// full row count in `total_rows`.
    pub fn from_result(result: &ComparisonResult) -> Self {
        ComparisonPreview {
            rows: result.rows.iter().take(MAX_PREVIEW_ROWS).cloned().collect(),
            key_column: result.key_column.clone(),
            value_column: result.value_column.clone(),
            summary: result.summary.clone(),
            total_rows: result.rows.len(),
        }
    }
}

/// Parses CSV from `reader`, treating the first record as headers. Ragged rows are
/// accepted and normalized to the header width — short rows are padded with empty strings,
/// long rows are truncated — so cells stay column-aligned when rendered.
///
/// Cells are read as raw bytes (via [`csv::ByteRecord`]) so files that aren't valid UTF-8
/// (e.g. Excel exports in Windows-1252) load instead of failing hard: see [`sanitize_field`].
pub fn parse_csv<R: Read>(reader: R) -> Result<CsvTable, csv::Error> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(reader);

    let headers: Vec<String> = rdr.byte_headers()?.iter().map(sanitize_field).collect();
    let column_count = headers.len();

    let mut rows = Vec::new();
    let mut record = csv::ByteRecord::new();
    while rdr.read_byte_record(&mut record)? {
        let mut row: Vec<String> = record.iter().map(sanitize_field).collect();
        row.resize(column_count, String::new());
        rows.push(row);
    }

    Ok(CsvTable { headers, rows })
}

/// Converts a raw CSV cell to a `String`. Valid UTF-8 is kept intact; otherwise the non-ASCII
/// bytes are dropped, mirroring Ruby's `encode("US-ASCII", invalid: :replace, undef: :replace,
/// replace: "")`. Stripping is safe even mid-stream: UTF-8 multi-byte sequences only ever use
/// bytes `>= 0x80`, so they never collide with the ASCII delimiters/quotes the parser relies on.
fn sanitize_field(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(_) => bytes.iter().filter(|b| b.is_ascii()).map(|&b| b as char).collect(),
    }
}

/// Errors that can occur while loading an Excel workbook into a [`CsvTable`].
#[derive(Debug)]
pub enum ExcelError {
    Io(std::io::Error),
    Open(calamine::Error),
    NotSingleSheet(Vec<String>),
    Range(String),
}

impl std::fmt::Display for ExcelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExcelError::Io(err) => write!(f, "failed to read Excel workbook: {err}"),
            ExcelError::Open(err) => write!(f, "failed to open Excel workbook: {err}"),
            ExcelError::NotSingleSheet(names) if names.is_empty() => {
                write!(f, "Excel file must contain exactly one sheet; found 0")
            }
            ExcelError::NotSingleSheet(names) => write!(
                f,
                "Excel file must contain exactly one sheet; found {}: {}",
                names.len(),
                names.join(", ")
            ),
            ExcelError::Range(err) => write!(f, "failed to read Excel worksheet: {err}"),
        }
    }
}

impl std::error::Error for ExcelError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ExcelError::Io(err) => Some(err),
            ExcelError::Open(err) => Some(err),
            ExcelError::NotSingleSheet(_) | ExcelError::Range(_) => None,
        }
    }
}

/// Parses a single-sheet Excel workbook into a [`CsvTable`]. The first row becomes headers;
/// remaining rows are padded/truncated to that same width, matching [`parse_csv`].
pub fn parse_excel<R: Read + Seek>(mut reader: R) -> Result<CsvTable, ExcelError> {
    let mut bytes = Vec::new();
    reader
        .seek(SeekFrom::Start(0))
        .map_err(ExcelError::Io)?;
    reader.read_to_end(&mut bytes).map_err(ExcelError::Io)?;

    // Recover each cell's display number format before calamine consumes `bytes`. calamine only
    // hands us the raw stored value, so a price like 30.40 arrives as 30.400000000000002; the
    // format codes (e.g. "0.00", "$#,##0.00") let us render what the sheet shows. Empty for
    // non-xlsx (.xls/.xlsb) or any read hiccup — we just fall back to the raw value then.
    let formats = read_cell_formats(&bytes);

    let mut workbook = open_workbook_auto_from_rs(Cursor::new(bytes)).map_err(ExcelError::Open)?;
    let names = workbook.sheet_names();
    if names.len() != 1 {
        return Err(ExcelError::NotSingleSheet(names));
    }

    let range = workbook
        .worksheet_range(&names[0])
        .map_err(|err| ExcelError::Range(err.to_string()))?;

    // `rows()` yields rows starting at the range's top-left, so add this base to each (row, col)
    // offset to recover the absolute coordinate that keys `formats`.
    let (base_row, base_col) = range.start().unwrap_or((0, 0));

    let mut excel_rows = range.rows().enumerate();
    let Some((_, header_row)) = excel_rows.next() else {
        return Ok(CsvTable {
            headers: Vec::new(),
            rows: Vec::new(),
        });
    };

    // Headers are text; never apply a number format to them.
    let mut headers: Vec<String> = header_row.iter().map(|c| cell_to_string(c, None)).collect();
    while headers.last().is_some_and(|header| header.is_empty()) {
        headers.pop();
    }
    let column_count = headers.len();
    let rows = excel_rows
        .map(|(row_offset, row)| {
            let abs_row = base_row + row_offset as u32;
            let mut cells: Vec<String> = row
                .iter()
                .enumerate()
                .map(|(col_offset, cell)| {
                    let abs_col = base_col + col_offset as u32;
                    let format = formats.get(&(abs_row, abs_col)).map(String::as_str);
                    cell_to_string(cell, format)
                })
                .collect();
            cells.resize(column_count, String::new());
            cells
        })
        .collect();

    Ok(CsvTable { headers, rows })
}

/// Converts a cell to its display string. `format` is the cell's Excel number-format code (when
/// known); numeric cells try to honor it via [`apply_number_format`] and otherwise fall back to
/// [`float_to_string`].
fn cell_to_string(cell: &Data, format: Option<&str>) -> String {
    match cell {
        Data::Empty => String::new(),
        Data::String(value) => value.clone(),
        Data::Bool(value) => value.to_string(),
        Data::Int(value) => format
            .and_then(|f| apply_number_format(*value as f64, f))
            .unwrap_or_else(|| value.to_string()),
        Data::Float(value) => format
            .and_then(|f| apply_number_format(*value, f))
            .unwrap_or_else(|| float_to_string(*value)),
        Data::DateTime(value) => {
            let (year, month, day, hour, minute, second, millisecond) = value.to_ymd_hms_milli();
            format!(
                "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millisecond:03}"
            )
        }
        Data::DateTimeIso(value) | Data::DurationIso(value) => value.clone(),
        Data::Error(_) => String::new(),
    }
}

fn float_to_string(value: f64) -> String {
    if !value.is_finite() {
        return value.to_string();
    }
    // Excel itself only keeps 15 significant digits, so the shortest round-trip of the stored
    // double can show binary artifacts Excel never displays — a "20% off" price computed as
    // `38 * 0.8` is stored as 30.400000000000002. calamine can't hand us the cell's display
    // format, so we re-round to Excel's own 15-digit precision, which collapses it back to 30.4.
    let rounded: f64 = format!("{value:.14e}").parse().unwrap_or(value);
    if rounded.fract() == 0.0 && rounded.abs() < 1e15 {
        format!("{rounded:.0}")
    } else {
        rounded.to_string()
    }
}

/// Reads each numeric cell's Excel number-format code from an xlsx package, keyed by absolute
/// `(row, col)` (both 0-based). An xlsx is a zip: `xl/styles.xml` maps style indices to format
/// codes, and the worksheet xml tags each cell with its style index. Returns an empty map for
/// non-xlsx input or any parse failure, so the caller transparently falls back to raw values.
fn read_cell_formats(bytes: &[u8]) -> HashMap<(u32, u32), String> {
    let empty = HashMap::new();
    let Ok(mut zip) = zip::ZipArchive::new(Cursor::new(bytes)) else {
        return empty;
    };
    let Some(styles) = read_zip_entry(&mut zip, "xl/styles.xml") else {
        return empty;
    };
    let (num_fmts, cell_xfs) = parse_styles(&styles);

    // Single-sheet workbooks (the only kind we accept) have exactly one worksheet xml; its exact
    // name varies, so find it rather than assuming `sheet1.xml`.
    let sheet_path = zip
        .file_names()
        .find(|name| {
            name.starts_with("xl/worksheets/") && name.ends_with(".xml") && !name.contains("/_rels/")
        })
        .map(String::from);
    let Some(sheet_path) = sheet_path else {
        return empty;
    };
    let Some(sheet) = read_zip_entry(&mut zip, &sheet_path) else {
        return empty;
    };
    parse_sheet_formats(&sheet, &num_fmts, &cell_xfs)
}

fn read_zip_entry<R: Read + Seek>(zip: &mut zip::ZipArchive<R>, name: &str) -> Option<String> {
    let mut file = zip.by_name(name).ok()?;
    let mut contents = String::new();
    file.read_to_string(&mut contents).ok()?;
    Some(contents)
}

/// Value of attribute `key` on `element`, as an owned `String`.
fn xml_attr(element: &BytesStart, key: &[u8]) -> Option<String> {
    element
        .attributes()
        .flatten()
        .find(|attr| attr.key.as_ref() == key)
        .map(|attr| String::from_utf8_lossy(&attr.value).into_owned())
}

/// Parses `styles.xml` into `(custom numFmtId -> format code, cellXfs index -> numFmtId)`. Only
/// the `<cellXfs>` block matters (cell-level styles); the same-named `<xf>` under `<cellStyleXfs>`
/// must be ignored, so we track which block we're inside.
fn parse_styles(xml: &str) -> (HashMap<u32, String>, Vec<u32>) {
    let mut num_fmts: HashMap<u32, String> = HashMap::new();
    let mut cell_xfs: Vec<u32> = Vec::new();
    let mut in_cell_xfs = false;

    let mut reader = quick_xml::Reader::from_str(xml);
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => match e.local_name().as_ref() {
                b"numFmt" => {
                    if let (Some(id), Some(code)) =
                        (xml_attr(&e, b"numFmtId"), xml_attr(&e, b"formatCode"))
                    {
                        if let Ok(id) = id.parse::<u32>() {
                            num_fmts.insert(id, code);
                        }
                    }
                }
                b"cellXfs" => in_cell_xfs = true,
                b"xf" if in_cell_xfs => {
                    let id = xml_attr(&e, b"numFmtId")
                        .and_then(|s| s.parse::<u32>().ok())
                        .unwrap_or(0);
                    cell_xfs.push(id);
                }
                _ => {}
            },
            Ok(Event::End(e)) if e.local_name().as_ref() == b"cellXfs" => in_cell_xfs = false,
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    (num_fmts, cell_xfs)
}

/// Parses worksheet xml into `(row, col) -> format code`, resolving each cell's style index
/// through `cell_xfs`/`num_fmts`. Only cells whose format resolves to a numeric code we handle
/// are recorded; everything else is left to the raw-value fallback.
fn parse_sheet_formats(
    xml: &str,
    num_fmts: &HashMap<u32, String>,
    cell_xfs: &[u32],
) -> HashMap<(u32, u32), String> {
    let mut formats = HashMap::new();
    let mut reader = quick_xml::Reader::from_str(xml);
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if e.local_name().as_ref() == b"c" => {
                let Some(coord) = xml_attr(&e, b"r").and_then(|r| parse_cell_ref(&r)) else {
                    continue;
                };
                let style_index = xml_attr(&e, b"s")
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(0);
                let num_fmt_id = cell_xfs.get(style_index).copied().unwrap_or(0);
                if let Some(code) = resolve_format_code(num_fmt_id, num_fmts) {
                    formats.insert(coord, code);
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    formats
}

/// Converts an A1-style cell reference (e.g. `"B2"`) to a 0-based `(row, col)`.
fn parse_cell_ref(reference: &str) -> Option<(u32, u32)> {
    let split = reference.find(|c: char| c.is_ascii_digit())?;
    let (letters, digits) = reference.split_at(split);
    if letters.is_empty() {
        return None;
    }
    let mut col = 0u32;
    for byte in letters.bytes() {
        if !byte.is_ascii_alphabetic() {
            return None;
        }
        // Reject overlong references rather than overflow (panic in debug, wrap in release).
        col = col
            .checked_mul(26)?
            .checked_add(u32::from(byte.to_ascii_uppercase() - b'A' + 1))?;
    }
    let row: u32 = digits.parse().ok()?;
    Some((row.checked_sub(1)?, col.checked_sub(1)?))
}

/// Resolves a `numFmtId` to a format code. Custom ids (>= 164) come from the workbook's
/// `<numFmt>` table; lower ids are Excel built-ins, of which we map only the common numeric ones
/// — anything else (General, dates, accounting/multi-section built-ins) returns `None` to fall
/// back to the raw value.
fn resolve_format_code(num_fmt_id: u32, custom: &HashMap<u32, String>) -> Option<String> {
    if num_fmt_id >= 164 {
        return custom.get(&num_fmt_id).cloned();
    }
    let builtin = match num_fmt_id {
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        9 => "0%",
        10 => "0.00%",
        _ => return None,
    };
    Some(builtin.to_string())
}

/// Applies an Excel number-format code to `value`, returning the display string. Handles the
/// pragmatic subset that covers price lists — fixed decimal places, thousands separators, a
/// leading currency symbol, and percent — and returns `None` for anything outside that subset
/// (multiple sections, scientific, fractions, dates, text placeholders) so the caller falls
/// back to [`float_to_string`].
fn apply_number_format(value: f64, format: &str) -> Option<String> {
    if !value.is_finite() {
        return None;
    }
    let format = format.trim();
    if format.is_empty() || format.eq_ignore_ascii_case("General") {
        return None;
    }

    let mut currency = String::new();
    let mut decimal_places = 0usize;
    let mut seen_digit = false;
    let mut in_decimals = false;
    let mut has_thousands = false;
    let mut has_percent = false;

    let mut chars = format.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            // Literals that precede the number become a currency prefix (e.g. `"$"#,##0.00`,
            // `\$0.00`, or `[$£-809]`); literals after the digits are dropped.
            '\\' => {
                if let Some(next) = chars.next() {
                    if !seen_digit {
                        currency.push(next);
                    }
                }
            }
            '"' => {
                let mut literal = String::new();
                for q in chars.by_ref() {
                    if q == '"' {
                        break;
                    }
                    literal.push(q);
                }
                if !seen_digit {
                    currency.push_str(&literal);
                }
            }
            '[' => {
                let mut inner = String::new();
                for b in chars.by_ref() {
                    if b == ']' {
                        break;
                    }
                    inner.push(b);
                }
                // `[<100]`/`[>=0]` are conditional sections — too complex, so bail. `[$sym-locale]`
                // carries a currency symbol; `[Red]`/`[$-409]` are color/locale and ignored.
                if inner.starts_with(['<', '>', '=']) {
                    return None;
                }
                if !seen_digit {
                    if let Some(rest) = inner.strip_prefix('$') {
                        currency.push_str(rest.split('-').next().unwrap_or(""));
                    }
                }
            }
            ';' => return None, // multiple sections (positive;negative;zero;text)
            'E' | 'e' | '/' => return None, // scientific / fractions
            'y' | 'Y' | 'm' | 'M' | 'd' | 'D' | 'h' | 'H' | 's' | 'S' | '@' => return None, // date/time/text
            '%' => has_percent = true,
            '0' | '#' | '?' => {
                seen_digit = true;
                if in_decimals {
                    decimal_places += 1;
                }
            }
            '.' => {
                if in_decimals {
                    return None; // two decimal points: not a format we understand
                }
                in_decimals = true;
            }
            ',' => {
                if seen_digit && !in_decimals {
                    has_thousands = true;
                }
            }
            '$' if !seen_digit => currency.push('$'),
            _ => {} // spaces and other literal characters
        }
    }

    if !seen_digit {
        return None;
    }

    let mut scaled = value;
    if has_percent {
        scaled *= 100.0;
    }
    let negative = scaled.is_sign_negative() && scaled != 0.0;
    let mut digits = format!("{:.*}", decimal_places, scaled.abs());
    if has_thousands {
        digits = group_thousands(&digits);
    }

    let mut out = String::new();
    if negative {
        out.push('-');
    }
    out.push_str(&currency);
    out.push_str(&digits);
    if has_percent {
        out.push('%');
    }
    Some(out)
}

/// Inserts thousands separators into the integer part of a formatted number like `"1234.50"`,
/// leaving any decimal part untouched: `"1234.50"` -> `"1,234.50"`.
fn group_thousands(number: &str) -> String {
    let (integer, decimal) = match number.split_once('.') {
        Some((i, d)) => (i, Some(d)),
        None => (number, None),
    };
    let mut grouped = String::with_capacity(integer.len() + integer.len() / 3);
    let len = integer.len();
    for (i, ch) in integer.bytes().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch as char);
    }
    match decimal {
        Some(d) => format!("{grouped}.{d}"),
        None => grouped,
    }
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

/// Options controlling [`filter_rows`]. The `serde` derives (here and on the types below) are
/// for callers that need to persist or ship these values; the app itself passes them directly.
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
/// that side lacks the key.
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
            // Labels mirror the ones the UI renders, so an exported CSV matches the screen.
            let status = match row.status {
                ComparisonStatus::Matched => "Matched",
                ComparisonStatus::Diff => "Diff",
                ComparisonStatus::OnlyLeft => "Only Left",
                ComparisonStatus::OnlyRight => "Only Right",
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

/// Whether every value in `values` is blank or parses as a number — lets the sort helpers
/// pick numeric vs lexicographic ordering for a column. An all-blank column is not numeric.
fn column_is_numeric<'a>(values: impl Iterator<Item = &'a str>) -> bool {
    let mut any_value = false;
    for v in values {
        let trimmed = v.trim();
        if trimmed.is_empty() {
            continue;
        }
        any_value = true;
        // `f64::from_str` accepts "NaN"/"inf"/"infinity"; require a finite value so those
        // sort as text instead of being treated as numbers.
        match trimmed.parse::<f64>() {
            Ok(n) if n.is_finite() => {}
            _ => return false,
        }
    }
    any_value
}

/// Orders two cells: numerically when `numeric`, else lexicographically. Blank cells sort
/// before non-blank ones, so reversing for descending order pushes them to the end.
fn compare_cells(a: &str, b: &str, numeric: bool) -> Ordering {
    if numeric {
        match (a.trim().parse::<f64>().ok(), b.trim().parse::<f64>().ok()) {
            (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Less,
            (Some(_), None) => Ordering::Greater,
        }
    } else {
        a.cmp(b)
    }
}

/// Returns `table` with its rows reordered by `column` (a header index). A numeric column
/// (every non-blank cell parses as a number) sorts numerically, otherwise lexicographically.
/// The sort is stable, so cells that compare equal keep their original relative order. An
/// out-of-range `column` leaves the rows untouched.
pub fn sort_rows(table: &CsvTable, column: usize, ascending: bool) -> CsvTable {
    let mut rows = table.rows.clone();
    if column < table.headers.len() {
        let numeric = column_is_numeric(
            rows.iter().map(|r| r.get(column).map(String::as_str).unwrap_or("")),
        );
        rows.sort_by(|a, b| {
            let av = a.get(column).map(String::as_str).unwrap_or("");
            let bv = b.get(column).map(String::as_str).unwrap_or("");
            let ord = compare_cells(av, bv, numeric);
            if ascending { ord } else { ord.reverse() }
        });
    }
    CsvTable {
        headers: table.headers.clone(),
        rows,
    }
}

/// The sort rank of a status when ordering a comparison by its Status column.
fn status_rank(status: ComparisonStatus) -> u8 {
    match status {
        ComparisonStatus::Matched => 0,
        ComparisonStatus::Diff => 1,
        ComparisonStatus::OnlyLeft => 2,
        ComparisonStatus::OnlyRight => 3,
    }
}

/// Returns `result` with its rows reordered by `column`: 0 = key, 1 = left value, 2 = right
/// value, 3 = status (matched < diff < only-left < only-right). Key/value columns use the
/// same numeric-aware ordering as [`sort_rows`], treating a missing value as blank. The
/// summary and column labels are preserved; an out-of-range `column` leaves rows untouched.
pub fn sort_comparison(
    result: &ComparisonResult,
    column: usize,
    ascending: bool,
) -> ComparisonResult {
    let mut rows = result.rows.clone();
    let value = |row: &ComparisonRow| -> String {
        match column {
            0 => row.key.clone(),
            1 => row.left_value.clone().unwrap_or_default(),
            2 => row.right_value.clone().unwrap_or_default(),
            _ => String::new(),
        }
    };
    let column_values: Vec<String> = rows.iter().map(&value).collect();
    let numeric = column <= 2 && column_is_numeric(column_values.iter().map(String::as_str));
    rows.sort_by(|a, b| {
        let ord = if column == 3 {
            status_rank(a.status).cmp(&status_rank(b.status))
        } else {
            compare_cells(&value(a), &value(b), numeric)
        };
        if ascending { ord } else { ord.reverse() }
    });
    ComparisonResult {
        rows,
        key_column: result.key_column.clone(),
        value_column: result.value_column.clone(),
        summary: result.summary.clone(),
    }
}

#[cfg(test)]
mod sort_tests {
    use super::*;

    fn table(rows: &[&[&str]]) -> CsvTable {
        CsvTable {
            headers: vec!["a".into(), "b".into()],
            rows: rows
                .iter()
                .map(|r| r.iter().map(|s| s.to_string()).collect())
                .collect(),
        }
    }

    fn col(table: &CsvTable, index: usize) -> Vec<&str> {
        table.rows.iter().map(|r| r[index].as_str()).collect()
    }

    #[test]
    fn sorts_numeric_columns_numerically() {
        let t = table(&[&["10", "x"], &["2", "y"], &["1", "z"]]);
        assert_eq!(col(&sort_rows(&t, 0, true), 0), ["1", "2", "10"]);
    }

    #[test]
    fn sorts_text_columns_and_reverses() {
        let t = table(&[&["1", "banana"], &["2", "apple"], &["3", "cherry"]]);
        assert_eq!(col(&sort_rows(&t, 1, true), 1), ["apple", "banana", "cherry"]);
        assert_eq!(col(&sort_rows(&t, 1, false), 1), ["cherry", "banana", "apple"]);
    }

    #[test]
    fn blanks_sort_before_values_ascending() {
        let t = table(&[&["2", ""], &["1", ""], &["3", "5"]]);
        // Column 1 has a blank and "5": numeric, blank sorts first ascending.
        assert_eq!(col(&sort_rows(&t, 1, true), 1), ["", "", "5"]);
    }

    #[test]
    fn non_finite_values_sort_as_text() {
        // "inf"/"NaN" must not make the column numeric, so it sorts lexicographically.
        let t = table(&[&["2", "a"], &["10", "b"], &["inf", "c"]]);
        assert_eq!(col(&sort_rows(&t, 0, true), 0), ["10", "2", "inf"]);
    }

    #[test]
    fn out_of_range_column_is_unchanged() {
        let t = table(&[&["2", "y"], &["1", "x"]]);
        assert_eq!(sort_rows(&t, 9, true), t);
    }

    #[test]
    fn sort_is_stable() {
        let t = table(&[&["1", "first"], &["1", "second"], &["0", "third"]]);
        assert_eq!(col(&sort_rows(&t, 0, true), 1), ["third", "first", "second"]);
    }

    fn comparison() -> ComparisonResult {
        ComparisonResult {
            rows: vec![
                ComparisonRow {
                    key: "banana".into(),
                    left_value: None,
                    right_value: Some("1".into()),
                    status: ComparisonStatus::OnlyRight,
                },
                ComparisonRow {
                    key: "apple".into(),
                    left_value: Some("1".into()),
                    right_value: Some("1".into()),
                    status: ComparisonStatus::Matched,
                },
                ComparisonRow {
                    key: "cherry".into(),
                    left_value: Some("1".into()),
                    right_value: Some("2".into()),
                    status: ComparisonStatus::Diff,
                },
            ],
            key_column: "k".into(),
            value_column: "v".into(),
            summary: ComparisonSummary {
                total: 3,
                matched: 1,
                diff: 1,
                only_left: 0,
                only_right: 1,
            },
        }
    }

    #[test]
    fn sorts_comparison_by_key() {
        let sorted = sort_comparison(&comparison(), 0, true);
        let keys: Vec<&str> = sorted.rows.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(keys, ["apple", "banana", "cherry"]);
    }

    #[test]
    fn sorts_comparison_by_status_and_preserves_summary() {
        let sorted = sort_comparison(&comparison(), 3, true);
        let statuses: Vec<ComparisonStatus> = sorted.rows.iter().map(|r| r.status).collect();
        assert_eq!(
            statuses,
            [
                ComparisonStatus::Matched,
                ComparisonStatus::Diff,
                ComparisonStatus::OnlyRight
            ]
        );
        assert_eq!(sorted.summary.total, 3);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    use rust_xlsxwriter::{ExcelDateTime, Format, Workbook, XlsxError};

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

    fn xlsx_bytes(build: impl FnOnce(&mut Workbook) -> Result<(), XlsxError>) -> Vec<u8> {
        let mut workbook = Workbook::new();
        build(&mut workbook).unwrap();
        workbook.save_to_buffer().unwrap()
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

    #[test]
    fn parse_falls_back_to_stripping_invalid_utf8() {
        // 0x92 is a Windows-1252 curly apostrophe — invalid as standalone UTF-8. The strict
        // parse fails, so we drop the non-ASCII byte and parse the rest, like Ruby's
        // `encode("US-ASCII", invalid: :replace, undef: :replace, replace: "")`.
        let mut input = b"id,name\nA,O".to_vec();
        input.push(0x92);
        input.extend_from_slice(b"Brien\n");
        let parsed = parse_csv(input.as_slice()).unwrap();
        assert_eq!(parsed, table(&["id", "name"], &[&["A", "OBrien"]]));
    }

    // --- parse_excel ---

    #[test]
    fn parse_excel_reads_headers_and_normalizes_rows() {
        let bytes = xlsx_bytes(|workbook| {
            let worksheet = workbook.add_worksheet();
            worksheet.write_string(0, 0, "id")?;
            worksheet.write_string(0, 1, "name")?;
            worksheet.write_string(0, 2, "qty")?;
            worksheet.write_string(1, 0, "A")?;
            worksheet.write_string(1, 1, "Apple")?;
            worksheet.write_number(1, 2, 2)?;
            worksheet.write_string(2, 0, "B")?;
            worksheet.write_string(2, 1, "Banana")?;
            worksheet.write_string(3, 0, "C")?;
            worksheet.write_string(3, 1, "Cherry")?;
            worksheet.write_number(3, 2, 5)?;
            worksheet.write_string(3, 3, "ignored")?;
            Ok(())
        });

        let parsed = parse_excel(Cursor::new(bytes)).unwrap();

        assert_eq!(
            parsed,
            table(
                &["id", "name", "qty"],
                &[
                    &["A", "Apple", "2"],
                    &["B", "Banana", ""],
                    &["C", "Cherry", "5"],
                ]
            )
        );
    }

    #[test]
    fn parse_excel_rejects_multi_sheet_workbooks() {
        let bytes = xlsx_bytes(|workbook| {
            workbook.add_worksheet().set_name("Left")?;
            workbook.add_worksheet().set_name("Right")?;
            Ok(())
        });

        match parse_excel(Cursor::new(bytes)) {
            Err(ExcelError::NotSingleSheet(names)) => {
                assert_eq!(names, vec!["Left".to_string(), "Right".to_string()]);
            }
            other => panic!("expected NotSingleSheet error, got {other:?}"),
        }
    }

    #[test]
    fn parse_excel_empty_sheet_yields_no_headers_or_rows() {
        let bytes = xlsx_bytes(|workbook| {
            workbook.add_worksheet().set_name("Empty")?;
            Ok(())
        });

        let parsed = parse_excel(Cursor::new(bytes)).unwrap();

        assert!(parsed.headers.is_empty());
        assert!(parsed.rows.is_empty());
    }

    #[test]
    fn parse_excel_formats_dates_as_iso_8601() {
        let bytes = xlsx_bytes(|workbook| {
            let worksheet = workbook.add_worksheet();
            let date_format = Format::new().set_num_format("yyyy-mm-dd hh:mm:ss.000");
            let datetime = ExcelDateTime::from_ymd(2026, 5, 29)?.and_hms_milli(13, 14, 15, 123)?;
            worksheet.write_string(0, 0, "when")?;
            worksheet.write_datetime_with_format(1, 0, &datetime, &date_format)?;
            Ok(())
        });

        let parsed = parse_excel(Cursor::new(bytes)).unwrap();

        assert_eq!(parsed.headers, vec!["when".to_string()]);
        assert_eq!(parsed.rows, rows(&[&["2026-05-29T13:14:15.123"]]));
    }

    #[test]
    fn parse_excel_applies_number_formats_to_prices() {
        // The price is computed as 38 * 0.8 and stored as 30.400000000000002, but the cell's
        // "$#,##0.00" format makes Excel show $30.40 — and a thousands case to exercise grouping.
        let bytes = xlsx_bytes(|workbook| {
            let worksheet = workbook.add_worksheet();
            let money = Format::new().set_num_format("$#,##0.00");
            let plain = Format::new().set_num_format("0.00");
            worksheet.write_string(0, 0, "name")?;
            worksheet.write_string(0, 1, "price")?;
            worksheet.write_string(0, 2, "subtotal")?;
            worksheet.write_string(1, 0, "Gem")?;
            worksheet.write_number_with_format(1, 1, 38.0 * 0.8, &money)?;
            worksheet.write_number_with_format(1, 2, 1234.5, &money)?;
            worksheet.write_string(2, 0, "Plain")?;
            worksheet.write_number_with_format(2, 1, 30.4, &plain)?;
            worksheet.write_number(2, 2, 7.0)?; // no format -> raw fallback
            Ok(())
        });

        let parsed = parse_excel(Cursor::new(bytes)).unwrap();

        assert_eq!(parsed.headers, vec!["name", "price", "subtotal"]);
        assert_eq!(
            parsed.rows,
            rows(&[
                &["Gem", "$30.40", "$1,234.50"],
                &["Plain", "30.40", "7"],
            ])
        );
    }

    // --- apply_number_format ---

    #[test]
    fn apply_number_format_handles_the_pragmatic_subset() {
        let f = |v, fmt| apply_number_format(v, fmt);
        // Fixed decimals round off the binary artifact and pad trailing zeros.
        assert_eq!(f(38.0 * 0.8, "0.00").as_deref(), Some("30.40"));
        assert_eq!(f(30.4, "0.00").as_deref(), Some("30.40"));
        // Thousands separators, currency prefix (literal, escaped, and quoted), and percent.
        assert_eq!(f(1234.5, "#,##0.00").as_deref(), Some("1,234.50"));
        assert_eq!(f(1234567.0, "#,##0").as_deref(), Some("1,234,567"));
        assert_eq!(f(30.4, "$#,##0.00").as_deref(), Some("$30.40"));
        assert_eq!(f(30.4, "\\$0.00").as_deref(), Some("$30.40"));
        assert_eq!(f(30.4, "\"$\"0.00").as_deref(), Some("$30.40"));
        assert_eq!(f(0.305, "0.0%").as_deref(), Some("30.5%"));
        assert_eq!(f(-5.0, "$#,##0.00").as_deref(), Some("-$5.00"));
        // Outside the subset -> None so the caller falls back to the raw value.
        assert_eq!(f(1.0, "General"), None);
        assert_eq!(f(1.0, ""), None);
        assert_eq!(f(1.0, "0.00;(0.00)"), None); // multiple sections
        assert_eq!(f(1.0, "0.00E+00"), None); // scientific
        assert_eq!(f(1.0, "# ?/?"), None); // fraction
        assert_eq!(f(1.0, "[<100]0.00"), None); // conditional
    }

    // --- float_to_string ---

    #[test]
    fn float_to_string_rounds_off_binary_artifacts() {
        // 38 * 0.8 is stored as this exact double; Excel shows 30.4.
        assert_eq!(float_to_string(38.0 * 0.8), "30.4");
        assert_eq!(float_to_string(30.400000000000002), "30.4");
        assert_eq!(float_to_string(0.1 + 0.2), "0.3");
        assert_eq!(float_to_string(30.0), "30");
        assert_eq!(float_to_string(1234.56), "1234.56");
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

    // --- previews ---

    #[test]
    fn table_preview_passes_through_when_under_cap() {
        let t = table(&["a"], &[&["1"], &["2"]]);
        let preview = TablePreview::from_table(&t);
        assert_eq!(preview.headers, vec!["a".to_string()]);
        assert_eq!(preview.rows, rows(&[&["1"], &["2"]]));
        assert_eq!(preview.total_rows, 2);
    }

    #[test]
    fn table_preview_caps_rows_but_reports_full_total() {
        let data: Vec<Vec<String>> = (0..MAX_PREVIEW_ROWS + 50)
            .map(|i| vec![i.to_string()])
            .collect();
        let t = CsvTable {
            headers: vec!["a".to_string()],
            rows: data,
        };
        let preview = TablePreview::from_table(&t);
        assert_eq!(preview.rows.len(), MAX_PREVIEW_ROWS);
        assert_eq!(preview.total_rows, MAX_PREVIEW_ROWS + 50);
        // The cap keeps the *first* rows, in order.
        assert_eq!(preview.rows[0], vec!["0".to_string()]);
        assert_eq!(
            preview.rows[MAX_PREVIEW_ROWS - 1],
            vec![(MAX_PREVIEW_ROWS - 1).to_string()]
        );
    }

    #[test]
    fn comparison_preview_caps_rows_and_keeps_full_summary() {
        let rows: Vec<ComparisonRow> = (0..MAX_PREVIEW_ROWS + 10)
            .map(|i| ComparisonRow {
                key: i.to_string(),
                left_value: Some("x".into()),
                right_value: Some("x".into()),
                status: ComparisonStatus::Matched,
            })
            .collect();
        let result = ComparisonResult {
            rows,
            key_column: "k".into(),
            value_column: "v".into(),
            summary: ComparisonSummary {
                total: MAX_PREVIEW_ROWS + 10,
                matched: MAX_PREVIEW_ROWS + 10,
                diff: 0,
                only_left: 0,
                only_right: 0,
            },
        };
        let preview = ComparisonPreview::from_result(&result);
        assert_eq!(preview.rows.len(), MAX_PREVIEW_ROWS);
        assert_eq!(preview.total_rows, MAX_PREVIEW_ROWS + 10);
        // Summary reflects the full result, not the capped rows.
        assert_eq!(preview.summary.total, MAX_PREVIEW_ROWS + 10);
        assert_eq!(preview.summary.matched, MAX_PREVIEW_ROWS + 10);
        assert_eq!(preview.key_column, "k");
        assert_eq!(preview.value_column, "v");
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
                &["A", "10", "10", "Matched"],
                &["B", "20", "", "Only Left"],
                &["C", "", "30", "Only Right"],
            ])
        );
    }
}

