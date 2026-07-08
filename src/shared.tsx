import {
  Box,
  Button,
  Group,
  List,
  Pagination,
  Popover,
  Table,
  Text,
} from "@mantine/core";

// A bounded view of a table the backend ships for display: at most the first
// `MAX_PREVIEW_ROWS` (1000) rows plus the true `totalRows`, so an extremely large dataset can't
// hang the UI. The full table stays server-side (referenced by id) for sort/filter/export.
export type TablePreview = {
  headers: string[];
  rows: string[][];
  totalRows: number;
};
// `id` is the backend `TableStore` handle; it's sent to filter/compare/common-columns/sort
// instead of the whole table on every recompute.
export type LoadedCsv = { id: number; table: TablePreview; path: string };

// Which column a table view is sorted by, and in which direction. `null` means unsorted.
export type SortState = { index: number; ascending: boolean };

// The next state when a header is clicked, cycling ascending → descending → unsorted. A
// click on a different column starts that column ascending; `null` means "back to unsorted".
export function nextSort(
  prev: SortState | null,
  index: number,
): SortState | null {
  if (!prev || prev.index !== index) return { index, ascending: true };
  if (prev.ascending) return { index, ascending: false };
  return null;
}

// Renders the ▲/▼ indicator for the sort column (empty for unsorted columns).
export function sortIndicator(
  sort: SortState | null | undefined,
  index: number,
): string {
  if (!sort || sort.index !== index) return "";
  return sort.ascending ? " ▲" : " ▼";
}

export const ITEMS_PER_PAGE = 10;

// A row is treated as blank when every cell is empty or whitespace-only. Blank rows are
// hidden in the rendered tables (display-only) — the underlying data keeps them.
export function isBlankRow(row: string[]): boolean {
  return row.every((cell) => cell.trim() === "");
}

// Empty/whitespace-only headers are shown as "(Empty column N)" using their 1-based column
// position, so blank columns stay identifiable in the table head.
export function displayHeaders(headers: string[]): string[] {
  return headers.map((header, index) =>
    header.trim() ? header : `(Empty column ${index + 1})`,
  );
}

// Middle-truncates a long file path to `maxLength`, keeping the filename intact. Handles
// both POSIX (`/`) and Windows (`\`) separators.
export function truncatePath(path: string, maxLength = 50): string {
  if (path.length <= maxLength) return path;

  const sepIndex = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  const fileName = sepIndex >= 0 ? path.slice(sepIndex + 1) : path;
  const dir = sepIndex >= 0 ? path.slice(0, sepIndex) : "";

  // When the filename alone won't fit, there's no room for any directory context — truncate
  // the filename itself to keep the result within `maxLength` (and avoid a negative
  // `startLength` below).
  if (fileName.length >= maxLength - 4) {
    return `...${fileName.slice(-(maxLength - 3))}`;
  }

  if (dir.length === 0) return `.../${fileName}`;
  if (dir.length <= maxLength - fileName.length - 1) return path;

  const startLength = Math.floor((maxLength - fileName.length - 4) / 2);
  const start = dir.slice(0, startLength);
  const end = dir.slice(dir.length - startLength);
  return `${start}...${end}/${fileName}`;
}

type CsvTableViewProps = {
  table: TablePreview;
  page: number;
  onPageChange: (page: number) => void;
  // Current sort and a click handler keyed by column index. The parent owns sorting (it
  // runs in Rust); this view only renders the already-sorted `table` and emits clicks.
  sort?: SortState | null;
  onSort?: (columnIndex: number) => void;
};

export function CsvTableView({
  table,
  page,
  onPageChange,
  sort,
  onSort,
}: CsvTableViewProps) {
  const headers = displayHeaders(table.headers);
  // Display-only: hide fully-blank rows; pagination counts only the visible ones.
  const visibleRows = table.rows.filter((row) => !isBlankRow(row));
  const totalPages = Math.ceil(visibleRows.length / ITEMS_PER_PAGE);
  const startIndex = (page - 1) * ITEMS_PER_PAGE;
  const endIndex = startIndex + ITEMS_PER_PAGE;
  const pageRows = visibleRows.slice(startIndex, endIndex);
  // The backend caps previews at the first 1000 rows; note when more rows exist server-side.
  const capped = table.totalRows > table.rows.length;

  return (
    <Box
      style={{
        flex: 1,
        minHeight: 0,
        display: "flex",
        flexDirection: "column",
      }}
    >
      {/* A bounded, scrollable region: `minHeight: 0` lets it shrink below content size so
          `overflow: auto` scrolls instead of overflowing. `stickyHeader` pins the header. */}
      <Box style={{ flex: 1, minHeight: 0, overflow: "auto" }}>
        <Table stickyHeader striped withTableBorder withColumnBorders>
          <Table.Thead>
            <Table.Tr>
              {headers.map((header, i) => (
                <Table.Th
                  key={i}
                  onClick={onSort ? () => onSort(i) : undefined}
                  style={{
                    whiteSpace: "nowrap",
                    cursor: onSort ? "pointer" : undefined,
                    userSelect: "none",
                  }}
                >
                  {header}
                  {sortIndicator(sort, i)}
                </Table.Th>
              ))}
            </Table.Tr>
          </Table.Thead>
          <Table.Tbody>
            {pageRows.map((row, r) => (
              <Table.Tr key={startIndex + r}>
                {row.map((cell, c) => (
                  <Table.Td key={c} style={{ whiteSpace: "nowrap" }}>
                    {cell}
                  </Table.Td>
                ))}
              </Table.Tr>
            ))}
          </Table.Tbody>
        </Table>
      </Box>

      <Group justify="space-between" mt="xs">
        <Text size="xs" c="dimmed">
          {visibleRows.length === 0
            ? "No rows"
            : `Showing ${startIndex + 1}-${Math.min(
                endIndex,
                visibleRows.length,
              )} of ${visibleRows.length} rows`}
          {capped &&
            ` · preview of first ${table.rows.length.toLocaleString()} of ${table.totalRows.toLocaleString()} rows`}
        </Text>
        {totalPages > 1 && (
          <Pagination
            value={page}
            onChange={onPageChange}
            total={totalPages}
            size="sm"
            siblings={1}
            boundaries={1}
          />
        )}
      </Group>
    </Box>
  );
}

// Explains how Excel files are interpreted on import. The backend (sheet-core) reads raw cell
// values and only partially reconstructs Excel's display formatting, so these caveats keep the
// numbers/dates from looking like bugs. Kept in sync with `parse_excel` / `apply_number_format`.
export function ExcelNotes() {
  return (
    <Popover width={420} position="bottom-start" withArrow shadow="md">
      <Popover.Target>
        <Button variant="subtle" size="compact-sm" color="gray">
          Excel notes
        </Button>
      </Popover.Target>
      <Popover.Dropdown>
        <Text fw={600} size="sm" mb={4}>
          How Excel (.xlsx / .xls) files are read
        </Text>
        <Text size="sm" mb="xs">
          Excel stores the full-precision number and a separate display format; we read the
          number and reapply the <strong>common</strong> formats — fixed decimals
          (e.g. <code>30.40</code>), thousands separators, a leading currency symbol, and
          percent. So a 20%-off price stored as <code>30.400000000000002</code> shows as
          its formatted value, not the raw double.
        </Text>
        <Text fw={600} size="sm" mb={4}>
          Known limitations &amp; gotchas
        </Text>
        <List size="sm" spacing={4}>
          <List.Item>
            <strong>Uncommon formats fall back</strong> to the plain number rounded to Excel&apos;s
            15-digit precision: accounting/negative-in-parentheses, colored or
            conditional formats, fractions, and scientific notation aren&apos;t reproduced.
          </List.Item>
          <List.Item>
            <strong>Trailing format text is dropped</strong> — a custom format like
            <code> 0.00&quot; USD&quot;</code> shows <code>30.40</code>, not <code>30.40 USD</code>.
          </List.Item>
          <List.Item>
            <strong>Only single-sheet workbooks</strong> are supported; multi-sheet files are
            rejected.
          </List.Item>
          <List.Item>
            <strong>Formulas show their last-saved value</strong> — nothing is recalculated on
            import.
          </List.Item>
          <List.Item>
            <strong>Dates are normalized to ISO 8601</strong>
            (<code>yyyy-mm-ddThh:mm:ss.mmm</code>), regardless of the sheet&apos;s date format.
          </List.Item>
          <List.Item>
            <strong>Numbers stay numeric</strong> — a code like <code>00123</code> stored as a
            number loses its leading zeros unless it was stored as text in Excel. Save as CSV to
            preserve exact text.
          </List.Item>
          <List.Item>
            Styling, colors, merged cells, and hidden rows/columns are ignored — values only.
          </List.Item>
        </List>
      </Popover.Dropdown>
    </Popover>
  );
}
