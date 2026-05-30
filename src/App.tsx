import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Badge,
  Box,
  Button,
  Checkbox,
  Group,
  Pagination,
  Paper,
  Select,
  SegmentedControl,
  Table,
  Text,
  Title,
} from "@mantine/core";
import { notifications } from "@mantine/notifications";

type CsvTable = { headers: string[]; rows: string[][] };
// `id` is the backend `TableStore` handle; it's sent to filter/compare/common-columns
// instead of the whole table on every recompute.
type LoadedCsv = { id: number; table: CsvTable; path: string };

// Mirrors `sheet_core::FilterMode` / `FilterOptions`. `exclude` drops left rows whose value
// in the chosen column appears in the right's same-named column; `include` keeps only those.
type FilterMode = "exclude" | "include";
type FilterOptions = { mode: FilterMode; caseInsensitive: boolean };

type OperationMode = "filter" | "compare";

// Mirrors `sheet_core::Comparison*`. A VLOOKUP-style diff classifying each key across the
// two CSVs. Field names are camelCase to match the Rust structs' `serde(rename_all)`.
type ComparisonStatus = "matched" | "diff" | "only-left" | "only-right";
type ComparisonRow = {
  key: string;
  leftValue: string | null;
  rightValue: string | null;
  status: ComparisonStatus;
};
type ComparisonSummary = {
  total: number;
  matched: number;
  diff: number;
  onlyLeft: number;
  onlyRight: number;
};
type ComparisonResult = {
  rows: ComparisonRow[];
  keyColumn: string;
  valueColumn: string;
  summary: ComparisonSummary;
};

const ITEMS_PER_PAGE = 10;

// A row is treated as blank when every cell is empty or whitespace-only. Blank rows are
// hidden in the rendered tables (display-only) — the underlying data keeps them.
function isBlankRow(row: string[]): boolean {
  return row.every((cell) => cell.trim() === "");
}

// Empty/whitespace-only headers are shown as "(Empty column N)" using their 1-based column
// position, so blank columns stay identifiable in the table head.
function displayHeaders(headers: string[]): string[] {
  return headers.map((header, index) =>
    header.trim() ? header : `(Empty column ${index + 1})`,
  );
}

// Middle-truncates a long file path to `maxLength`, keeping the filename intact. Handles
// both POSIX (`/`) and Windows (`\`) separators.
function truncatePath(path: string, maxLength = 50): string {
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
  table: CsvTable;
  page: number;
  onPageChange: (page: number) => void;
};

function CsvTableView({ table, page, onPageChange }: CsvTableViewProps) {
  const headers = displayHeaders(table.headers);
  // Display-only: hide fully-blank rows; pagination counts only the visible ones.
  const visibleRows = table.rows.filter((row) => !isBlankRow(row));
  const totalPages = Math.ceil(visibleRows.length / ITEMS_PER_PAGE);
  const startIndex = (page - 1) * ITEMS_PER_PAGE;
  const endIndex = startIndex + ITEMS_PER_PAGE;
  const pageRows = visibleRows.slice(startIndex, endIndex);

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
                <Table.Th key={i} style={{ whiteSpace: "nowrap" }}>
                  {header}
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

type TablePanelProps = {
  title: string;
  table: CsvTable | null;
  path: string | null;
  loading: boolean;
  page: number;
  onPageChange: (page: number) => void;
  onLoad: () => void;
};

function TablePanel({
  title,
  table,
  path,
  loading,
  page,
  onPageChange,
  onLoad,
}: TablePanelProps) {
  return (
    <Paper
      withBorder
      p="sm"
      style={{
        flex: 1,
        minWidth: 0,
        minHeight: 0,
        overflow: "hidden",
        display: "flex",
        flexDirection: "column",
      }}
    >
      <Group justify="space-between" mb="sm" wrap="nowrap">
        <Box style={{ minWidth: 0 }}>
          <Title order={2} size="h4">
            {title}
          </Title>
          {path && (
            <Text size="xs" c="dimmed" title={path}>
              {truncatePath(path)}
            </Text>
          )}
        </Box>
        <Button onClick={onLoad} loading={loading}>
          Load CSV
        </Button>
      </Group>

      {table ? (
        <CsvTableView table={table} page={page} onPageChange={onPageChange} />
      ) : (
        <Text c="dimmed" fs="italic">
          No spreadsheet loaded.
        </Text>
      )}
    </Paper>
  );
}

// Row tint per comparison status, using Mantine's light color variables.
const STATUS_BG: Record<ComparisonStatus, string | undefined> = {
  matched: undefined,
  diff: "var(--mantine-color-red-light)",
  "only-left": "var(--mantine-color-orange-light)",
  "only-right": "var(--mantine-color-blue-light)",
};

const STATUS_LABEL: Record<ComparisonStatus, string> = {
  matched: "Matched",
  diff: "Diff",
  "only-left": "Only Left",
  "only-right": "Only Right",
};

type ComparisonTableViewProps = {
  result: ComparisonResult;
  page: number;
  onPageChange: (page: number) => void;
};

function ComparisonTableView({
  result,
  page,
  onPageChange,
}: ComparisonTableViewProps) {
  const { rows, keyColumn, valueColumn, summary } = result;
  const totalPages = Math.ceil(rows.length / ITEMS_PER_PAGE);
  const startIndex = (page - 1) * ITEMS_PER_PAGE;
  const endIndex = startIndex + ITEMS_PER_PAGE;
  const pageRows = rows.slice(startIndex, endIndex);

  return (
    <Box
      style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}
    >
      <Group gap="xs" mb="xs">
        <Badge color="gray" variant="light">
          Total {summary.total}
        </Badge>
        <Badge color="green" variant="light">
          Matched {summary.matched}
        </Badge>
        <Badge color="red" variant="light">
          Diff {summary.diff}
        </Badge>
        <Badge color="orange" variant="light">
          Only Left {summary.onlyLeft}
        </Badge>
        <Badge color="blue" variant="light">
          Only Right {summary.onlyRight}
        </Badge>
      </Group>

      {/* Same bounded scroll region as CsvTableView (`minHeight: 0` + `overflow: auto`). */}
      <Box style={{ flex: 1, minHeight: 0, overflow: "auto" }}>
        <Table stickyHeader withTableBorder withColumnBorders>
          <Table.Thead>
            <Table.Tr>
              <Table.Th style={{ whiteSpace: "nowrap" }}>{keyColumn}</Table.Th>
              <Table.Th style={{ whiteSpace: "nowrap" }}>
                {valueColumn} (Left)
              </Table.Th>
              <Table.Th style={{ whiteSpace: "nowrap" }}>
                {valueColumn} (Right)
              </Table.Th>
              <Table.Th style={{ whiteSpace: "nowrap" }}>Status</Table.Th>
            </Table.Tr>
          </Table.Thead>
          <Table.Tbody>
            {pageRows.map((row, r) => (
              <Table.Tr
                key={startIndex + r}
                style={{ backgroundColor: STATUS_BG[row.status] }}
              >
                <Table.Td style={{ whiteSpace: "nowrap" }}>{row.key}</Table.Td>
                <Table.Td style={{ whiteSpace: "nowrap" }}>
                  {row.leftValue ?? ""}
                </Table.Td>
                <Table.Td style={{ whiteSpace: "nowrap" }}>
                  {row.rightValue ?? ""}
                </Table.Td>
                <Table.Td style={{ whiteSpace: "nowrap" }}>
                  {STATUS_LABEL[row.status]}
                </Table.Td>
              </Table.Tr>
            ))}
          </Table.Tbody>
        </Table>
      </Box>

      <Group justify="space-between" mt="xs">
        <Text size="xs" c="dimmed">
          {rows.length === 0
            ? "No rows"
            : `Showing ${startIndex + 1}-${Math.min(
                endIndex,
                rows.length,
              )} of ${rows.length} rows`}
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

function App() {
  const [leftTable, setLeftTable] = useState<CsvTable | null>(null);
  const [leftId, setLeftId] = useState<number | null>(null);
  const [leftPath, setLeftPath] = useState<string | null>(null);
  const [leftLoading, setLeftLoading] = useState(false);
  const [leftPage, setLeftPage] = useState(1);

  const [rightTable, setRightTable] = useState<CsvTable | null>(null);
  const [rightId, setRightId] = useState<number | null>(null);
  const [rightPath, setRightPath] = useState<string | null>(null);
  const [rightLoading, setRightLoading] = useState(false);
  const [rightPage, setRightPage] = useState(1);

  const [exporting, setExporting] = useState(false);
  // The column to filter by, stored as its index (as a string) into the right table's
  // headers — indices stay unique even when headers are empty or duplicated.
  const [selectedColumn, setSelectedColumn] = useState<string | null>(null);
  const [filterMode, setFilterMode] = useState<FilterMode>("exclude");
  const [caseInsensitive, setCaseInsensitive] = useState(false);
  const [filtered, setFiltered] = useState<CsvTable | null>(null);
  const [filteredPage, setFilteredPage] = useState(1);

  // Filter vs Compare. The case-insensitive toggle is shared across both modes.
  const [operationMode, setOperationMode] = useState<OperationMode>("filter");
  const [commonCols, setCommonCols] = useState<string[]>([]);
  const [keyColumn, setKeyColumn] = useState<string | null>(null);
  const [valueColumn, setValueColumn] = useState<string | null>(null);
  const [comparison, setComparison] = useState<ComparisonResult | null>(null);
  const [comparisonPage, setComparisonPage] = useState(1);

  const columnIndex = selectedColumn === null ? null : Number(selectedColumn);
  const columnValid =
    columnIndex !== null &&
    !!rightTable &&
    columnIndex < rightTable.headers.length;

  // The filter runs in the Rust `sheet-core` crate via the `filter_csv` command. Recompute
  // it whenever the inputs change; a cancellation flag drops a stale response if the inputs
  // change again before it resolves.
  useEffect(() => {
    if (
      operationMode !== "filter" ||
      !leftTable ||
      !rightTable ||
      leftId === null ||
      rightId === null ||
      !columnValid
    ) {
      setFiltered(null);
      return;
    }
    const options: FilterOptions = { mode: filterMode, caseInsensitive };
    let cancelled = false;
    invoke<CsvTable>("filter_csv", {
      leftId,
      rightId,
      column: rightTable.headers[columnIndex],
      options,
    })
      .then((result) => {
        if (cancelled) return;
        setFiltered(result);
        setFilteredPage(1);
      })
      .catch((err) => {
        if (cancelled) return;
        console.error("Failed to filter:", err);
        notifications.show({
          color: "red",
          title: "Filter failed",
          message: String(err),
        });
        setFiltered(null);
      });
    return () => {
      cancelled = true;
    };
  }, [
    operationMode,
    leftTable,
    rightTable,
    leftId,
    rightId,
    columnIndex,
    columnValid,
    filterMode,
    caseInsensitive,
  ]);

  // The candidate compare columns (header names in both tables) come from Rust. Recompute
  // when either table changes and clear any stale key/value selection.
  useEffect(() => {
    if (!leftTable || !rightTable || leftId === null || rightId === null) {
      setCommonCols([]);
      return;
    }
    let cancelled = false;
    invoke<string[]>("common_columns", { leftId, rightId })
      .then((cols) => {
        if (cancelled) return;
        setCommonCols(cols);
        setKeyColumn(null);
        setValueColumn(null);
      })
      .catch((err) => {
        if (cancelled) return;
        console.error("Failed to load common columns:", err);
        setCommonCols([]);
      });
    return () => {
      cancelled = true;
    };
  }, [leftTable, rightTable, leftId, rightId]);

  const keyIndex = keyColumn === null ? null : Number(keyColumn);
  const valueIndex = valueColumn === null ? null : Number(valueColumn);
  const compareValid =
    keyIndex !== null &&
    valueIndex !== null &&
    keyIndex < commonCols.length &&
    valueIndex < commonCols.length;

  // The compare runs in Rust via `compare_csv`. Recompute when the inputs change while in
  // compare mode; a cancellation flag drops a stale response.
  useEffect(() => {
    if (
      operationMode !== "compare" ||
      !leftTable ||
      !rightTable ||
      leftId === null ||
      rightId === null ||
      !compareValid
    ) {
      setComparison(null);
      return;
    }
    let cancelled = false;
    invoke<ComparisonResult>("compare_csv", {
      leftId,
      rightId,
      keyColumn: commonCols[keyIndex],
      valueColumn: commonCols[valueIndex],
      caseInsensitive,
    })
      .then((result) => {
        if (cancelled) return;
        setComparison(result);
        setComparisonPage(1);
      })
      .catch((err) => {
        if (cancelled) return;
        console.error("Failed to compare:", err);
        notifications.show({
          color: "red",
          title: "Compare failed",
          message: String(err),
        });
        setComparison(null);
      });
    return () => {
      cancelled = true;
    };
  }, [
    operationMode,
    leftTable,
    rightTable,
    leftId,
    rightId,
    keyIndex,
    valueIndex,
    compareValid,
    caseInsensitive,
    commonCols,
  ]);

  async function loadCsv(side: "left" | "right") {
    const isLeft = side === "left";
    const setTable = isLeft ? setLeftTable : setRightTable;
    const setId = isLeft ? setLeftId : setRightId;
    const setPath = isLeft ? setLeftPath : setRightPath;
    const setLoading = isLeft ? setLeftLoading : setRightLoading;
    const setPage = isLeft ? setLeftPage : setRightPage;
    // The side's current table is superseded by this load; pass its id so the backend
    // store evicts it (null on first load → `None`, nothing to evict).
    const replace = isLeft ? leftId : rightId;

    setLoading(true);
    try {
      // `load_csv` returns null when the user cancels the dialog; keep the current table.
      const result = await invoke<LoadedCsv | null>("load_csv", { replace });
      if (result) {
        setTable(result.table);
        setId(result.id);
        setPath(result.path);
        setPage(1);
      }
    } catch (err) {
      console.error("Failed to load CSV:", err);
      notifications.show({
        color: "red",
        title: "Failed to load CSV",
        message: String(err),
      });
    } finally {
      setLoading(false);
    }
  }

  function swapSides() {
    setLeftTable(rightTable);
    setRightTable(leftTable);
    // The tables stay in the backend store under their existing ids; just swap which side
    // points at which — no reload needed.
    setLeftId(rightId);
    setRightId(leftId);
    setLeftPath(rightPath);
    setRightPath(leftPath);
    setLeftPage(1);
    setRightPage(1);
    // Selections reference the now-swapped tables — clear them.
    setSelectedColumn(null);
    setFilteredPage(1);
    setKeyColumn(null);
    setValueColumn(null);
    setComparisonPage(1);
  }

  async function exportResult() {
    if (!filtered || filtered.rows.length === 0) return;
    setExporting(true);
    try {
      // `save_csv` returns false when the user cancels the save dialog; nothing to do.
      const saved = await invoke<boolean>("save_csv", { table: filtered });
      if (saved) {
        notifications.show({
          color: "green",
          title: "Export complete",
          message: "Filtered CSV saved.",
        });
      }
    } catch (err) {
      console.error("Failed to export result:", err);
      notifications.show({
        color: "red",
        title: "Export failed",
        message: String(err),
      });
    } finally {
      setExporting(false);
    }
  }

  // Compare results export through the Rust `comparison_to_table` renderer, then `save_csv`.
  async function exportComparison() {
    if (!comparison || comparison.rows.length === 0) return;
    setExporting(true);
    try {
      const table = await invoke<CsvTable>("comparison_to_table", {
        result: comparison,
      });
      const saved = await invoke<boolean>("save_csv", { table });
      if (saved) {
        notifications.show({
          color: "green",
          title: "Export complete",
          message: "Comparison CSV saved.",
        });
      }
    } catch (err) {
      console.error("Failed to export comparison:", err);
      notifications.show({
        color: "red",
        title: "Export failed",
        message: String(err),
      });
    } finally {
      setExporting(false);
    }
  }

  const canExport = !!filtered && filtered.rows.length > 0;
  const canExportComparison = !!comparison && comparison.rows.length > 0;
  const columnOptions =
    rightTable?.headers.map((_, index) => ({
      value: String(index),
      label: displayHeaders(rightTable.headers)[index],
    })) ?? [];
  const commonColOptions = commonCols.map((name, index) => ({
    value: String(index),
    label: name.trim() ? name : "(empty header)",
  }));

  return (
    // Plain flex column we fully control: a 100vh box whose two row regions (the source
    // panels and the filter panel) each take half the remaining height with `minHeight: 0`
    // so they shrink and scroll internally instead of overflowing the viewport.
    <Box
      style={{
        height: "100vh",
        boxSizing: "border-box",
        padding: "var(--mantine-spacing-md)",
        display: "flex",
        flexDirection: "column",
        gap: "var(--mantine-spacing-md)",
      }}
    >
      <Group justify="center" pos="relative">
        <Title order={1} ta="center">
          CSV Filter &amp; Compare
        </Title>
        <Button
          variant="default"
          onClick={swapSides}
          disabled={!leftTable && !rightTable}
          style={{ position: "absolute", right: 0 }}
        >
          Swap Left &amp; Right
        </Button>
      </Group>

      <Box
        style={{
          display: "flex",
          gap: "var(--mantine-spacing-md)",
          flex: 1,
          minHeight: 0,
        }}
      >
        <TablePanel
          title="Left"
          table={leftTable}
          path={leftPath}
          loading={leftLoading}
          page={leftPage}
          onPageChange={setLeftPage}
          onLoad={() => loadCsv("left")}
        />
        <TablePanel
          title="Right"
          table={rightTable}
          path={rightPath}
          loading={rightLoading}
          page={rightPage}
          onPageChange={setRightPage}
          onLoad={() => loadCsv("right")}
        />
      </Box>

      <Paper
        withBorder
        p="sm"
        style={{
          flex: 1,
          minHeight: 0,
          overflow: "hidden",
          display: "flex",
          flexDirection: "column",
        }}
      >
        <SegmentedControl
          value={operationMode}
          onChange={(value) => setOperationMode(value as OperationMode)}
          data={[
            { value: "filter", label: "Filter" },
            { value: "compare", label: "Compare" },
          ]}
          size="sm"
          mb="sm"
          style={{ alignSelf: "flex-start" }}
        />

        {operationMode === "filter" ? (
          <>
            <Group justify="space-between" mb="sm" wrap="nowrap" align="flex-end">
              <Group gap="md" align="flex-end" wrap="wrap">
                <Select
                  label="Column (from Right)"
                  placeholder="Select a column"
                  data={columnOptions}
                  value={selectedColumn}
                  onChange={setSelectedColumn}
                  disabled={!rightTable}
                  size="sm"
                  w={220}
                  comboboxProps={{ withinPortal: true }}
                />
                <SegmentedControl
                  value={filterMode}
                  onChange={(value) => setFilterMode(value as FilterMode)}
                  data={[
                    { value: "exclude", label: "Exclude" },
                    { value: "include", label: "Include" },
                  ]}
                  size="sm"
                />
                <Checkbox
                  label="Case insensitive"
                  checked={caseInsensitive}
                  onChange={(event) =>
                    setCaseInsensitive(event.currentTarget.checked)
                  }
                />
              </Group>
              <Button
                onClick={exportResult}
                loading={exporting}
                disabled={!canExport}
              >
                Export result
              </Button>
            </Group>

            {filtered ? (
              <CsvTableView
                table={filtered}
                page={filteredPage}
                onPageChange={setFilteredPage}
              />
            ) : (
              <Text c="dimmed" fs="italic">
                {leftTable && rightTable
                  ? "Pick a column from the Right CSV to filter the Left CSV."
                  : "Load both CSVs to filter."}
              </Text>
            )}
          </>
        ) : (
          <>
            <Group justify="space-between" mb="sm" wrap="nowrap" align="flex-end">
              <Group gap="md" align="flex-end" wrap="wrap">
                <Select
                  label="Key column"
                  placeholder="Select a column"
                  data={commonColOptions}
                  value={keyColumn}
                  onChange={setKeyColumn}
                  disabled={commonCols.length === 0}
                  size="sm"
                  w={200}
                  comboboxProps={{ withinPortal: true }}
                />
                <Select
                  label="Value column"
                  placeholder="Select a column"
                  data={commonColOptions}
                  value={valueColumn}
                  onChange={setValueColumn}
                  disabled={commonCols.length === 0}
                  size="sm"
                  w={200}
                  comboboxProps={{ withinPortal: true }}
                />
                <Checkbox
                  label="Case insensitive"
                  checked={caseInsensitive}
                  onChange={(event) =>
                    setCaseInsensitive(event.currentTarget.checked)
                  }
                />
              </Group>
              <Button
                onClick={exportComparison}
                loading={exporting}
                disabled={!canExportComparison}
              >
                Export result
              </Button>
            </Group>

            {comparison ? (
              <ComparisonTableView
                result={comparison}
                page={comparisonPage}
                onPageChange={setComparisonPage}
              />
            ) : (
              <Text c="dimmed" fs="italic">
                {!leftTable || !rightTable
                  ? "Load both CSVs to compare."
                  : commonCols.length === 0
                    ? "The two CSVs share no columns to compare."
                    : "Pick key and value columns to compare."}
              </Text>
            )}
          </>
        )}
      </Paper>
    </Box>
  );
}

export default App;
