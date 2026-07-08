import { useEffect, useRef, useState } from "react";
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
import {
  CsvTableView,
  displayHeaders,
  ExcelNotes,
  ITEMS_PER_PAGE,
  LoadedCsv,
  nextSort,
  SortState,
  sortIndicator,
  TablePreview,
  truncatePath,
} from "./shared";

// `filter_csv` / `compare_csv` return a bounded preview plus the backend id of the full result,
// which is passed back to sort/export so they operate on every row, not just the visible 1000.
type FilteredCsv = { id: number; table: TablePreview };
type ComparedCsv = { id: number; result: ComparisonPreview };

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
// A bounded view of a comparison result, mirroring `TablePreview`: at most the first
// `MAX_PREVIEW_ROWS` rows plus the true `totalRows`. `summary` is computed over the full result,
// so its tallies stay accurate even when `rows` is capped.
type ComparisonPreview = {
  rows: ComparisonRow[];
  keyColumn: string;
  valueColumn: string;
  summary: ComparisonSummary;
  totalRows: number;
};

type TablePanelProps = {
  title: string;
  table: TablePreview | null;
  path: string | null;
  loading: boolean;
  page: number;
  onPageChange: (page: number) => void;
  onLoad: () => void;
  sort: SortState | null;
  onSort: (columnIndex: number) => void;
};

function TablePanel({
  title,
  table,
  path,
  loading,
  page,
  onPageChange,
  onLoad,
  sort,
  onSort,
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
          Load CSV / Excel
        </Button>
      </Group>

      {table ? (
        <CsvTableView
          table={table}
          page={page}
          onPageChange={onPageChange}
          sort={sort}
          onSort={onSort}
        />
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
  result: ComparisonPreview;
  page: number;
  onPageChange: (page: number) => void;
  // Sort runs in Rust; this view renders the already-sorted `result` and emits header
  // clicks. Column indices: 0=key, 1=left value, 2=right value, 3=status.
  sort?: SortState | null;
  onSort?: (columnIndex: number) => void;
};

function ComparisonTableView({
  result,
  page,
  onPageChange,
  sort,
  onSort,
}: ComparisonTableViewProps) {
  const { rows, keyColumn, valueColumn, summary } = result;
  const columnLabels = [
    keyColumn,
    `${valueColumn} (Left)`,
    `${valueColumn} (Right)`,
    "Status",
  ];
  const totalPages = Math.ceil(rows.length / ITEMS_PER_PAGE);
  const startIndex = (page - 1) * ITEMS_PER_PAGE;
  const endIndex = startIndex + ITEMS_PER_PAGE;
  const pageRows = rows.slice(startIndex, endIndex);
  // The backend caps previews at the first 1000 rows; note when more rows exist server-side.
  const capped = result.totalRows > rows.length;

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
              {columnLabels.map((label, i) => (
                <Table.Th
                  key={i}
                  onClick={onSort ? () => onSort(i) : undefined}
                  style={{
                    whiteSpace: "nowrap",
                    cursor: onSort ? "pointer" : undefined,
                    userSelect: "none",
                  }}
                >
                  {label}
                  {sortIndicator(sort, i)}
                </Table.Th>
              ))}
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
          {capped &&
            ` · preview of first ${rows.length.toLocaleString()} of ${result.totalRows.toLocaleString()} rows`}
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

export default function FilterCompareView() {
  const [leftTable, setLeftTable] = useState<TablePreview | null>(null);
  const [leftId, setLeftId] = useState<number | null>(null);
  const [leftPath, setLeftPath] = useState<string | null>(null);
  const [leftLoading, setLeftLoading] = useState(false);
  const [leftPage, setLeftPage] = useState(1);
  // Display-only sorted view of the left table (null = show it unsorted) plus its sort
  // state. Kept separate from `leftTable` so sorting for display never re-triggers the
  // filter/compare effects or the backend operations (which still use the original order).
  const [leftView, setLeftView] = useState<TablePreview | null>(null);
  const [leftSort, setLeftSort] = useState<SortState | null>(null);

  const [rightTable, setRightTable] = useState<TablePreview | null>(null);
  const [rightId, setRightId] = useState<number | null>(null);
  const [rightPath, setRightPath] = useState<string | null>(null);
  const [rightLoading, setRightLoading] = useState(false);
  const [rightPage, setRightPage] = useState(1);
  const [rightView, setRightView] = useState<TablePreview | null>(null);
  const [rightSort, setRightSort] = useState<SortState | null>(null);

  const [exporting, setExporting] = useState(false);
  // The column to filter by, stored as its index (as a string) into the right table's
  // headers — indices stay unique even when headers are empty or duplicated.
  const [selectedColumn, setSelectedColumn] = useState<string | null>(null);
  const [filterMode, setFilterMode] = useState<FilterMode>("exclude");
  const [caseInsensitive, setCaseInsensitive] = useState(false);
  const [filtered, setFiltered] = useState<TablePreview | null>(null);
  // The backend `FilteredStore` id of the full filter result, passed to sort/export so they act
  // on every row, not just the visible preview. Null when there's no current filter result.
  const [filteredId, setFilteredId] = useState<number | null>(null);
  const [filteredPage, setFilteredPage] = useState(1);
  // `filtered`/`comparison` stay in their original (computed) order; the sorted display lives
  // in a separate view (null = unsorted) so a third header click can drop back to it.
  // Recomputing the result clears both the view and its sort descriptor.
  const [filteredView, setFilteredView] = useState<TablePreview | null>(null);
  const [filteredSort, setFilteredSort] = useState<SortState | null>(null);

  // Filter vs Compare. The case-insensitive toggle is shared across both modes.
  const [operationMode, setOperationMode] = useState<OperationMode>("filter");
  const [commonCols, setCommonCols] = useState<string[]>([]);
  const [keyColumn, setKeyColumn] = useState<string | null>(null);
  const [valueColumn, setValueColumn] = useState<string | null>(null);
  const [comparison, setComparison] = useState<ComparisonPreview | null>(null);
  // The backend `ComparisonStore` id of the full compare result, passed to sort/export.
  const [comparisonId, setComparisonId] = useState<number | null>(null);
  const [comparisonPage, setComparisonPage] = useState(1);
  const [comparisonView, setComparisonView] =
    useState<ComparisonPreview | null>(null);
  const [comparisonSort, setComparisonSort] = useState<SortState | null>(null);

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
      setFilteredId(null);
      return;
    }
    const options: FilterOptions = { mode: filterMode, caseInsensitive };
    let cancelled = false;
    invoke<FilteredCsv>("filter_csv", {
      leftId,
      rightId,
      column: rightTable.headers[columnIndex],
      options,
    })
      .then((result) => {
        if (cancelled) return;
        setFiltered(result.table);
        setFilteredId(result.id);
        setFilteredPage(1);
        setFilteredView(null);
        setFilteredSort(null);
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
        setFilteredId(null);
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
      setComparisonId(null);
      return;
    }
    let cancelled = false;
    invoke<ComparedCsv>("compare_csv", {
      leftId,
      rightId,
      keyColumn: commonCols[keyIndex],
      valueColumn: commonCols[valueIndex],
      caseInsensitive,
    })
      .then((result) => {
        if (cancelled) return;
        setComparison(result.result);
        setComparisonId(result.id);
        setComparisonPage(1);
        setComparisonView(null);
        setComparisonSort(null);
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
        setComparisonId(null);
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
    const setView = isLeft ? setLeftView : setRightView;
    const setSort = isLeft ? setLeftSort : setRightSort;
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
        // The new file replaces any sorted view of the previous one.
        setView(null);
        setSort(null);
      }
    } catch (err) {
      console.error("Failed to load file:", err);
      notifications.show({
        color: "red",
        title: "Failed to load file",
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
    // The sorted views and their sort state follow their tables to the other side.
    setLeftView(rightView);
    setRightView(leftView);
    setLeftSort(rightSort);
    setRightSort(leftSort);
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

  // Guards against out-of-order sort responses: every handler bumps its target's token at
  // the start, and a resolved (or cleared) response is only applied if its token is still
  // current — so a stale `sort_*` reply can't clobber a newer sort or clear.
  const leftSortReq = useRef(0);
  const rightSortReq = useRef(0);
  const filteredSortReq = useRef(0);
  const comparisonSortReq = useRef(0);

  // Reports a sort failure; the handlers below each call this from their catch block.
  function sortFailed(err: unknown) {
    console.error("Failed to sort:", err);
    notifications.show({
      color: "red",
      title: "Sort failed",
      message: String(err),
    });
  }

  // Sorting runs in Rust. Each header click cycles ascending → descending → unsorted. Source
  // tables sort by id (the store copy is untouched) into a display-only view; the
  // filter/compare results sort their original-order copy into a separate view. In every case
  // returning to unsorted just drops the view and shows the original order.
  async function sortSide(side: "left" | "right", columnIndex: number) {
    const isLeft = side === "left";
    const id = isLeft ? leftId : rightId;
    if (id === null) return;
    const setView = isLeft ? setLeftView : setRightView;
    const setSort = isLeft ? setLeftSort : setRightSort;
    const setPage = isLeft ? setLeftPage : setRightPage;
    const reqRef = isLeft ? leftSortReq : rightSortReq;
    const requestId = ++reqRef.current;
    const next = nextSort(isLeft ? leftSort : rightSort, columnIndex);
    setPage(1);
    if (!next) {
      setView(null);
      setSort(null);
      return;
    }
    try {
      const sorted = await invoke<TablePreview>("sort_csv", {
        id,
        column: columnIndex,
        ascending: next.ascending,
      });
      if (requestId !== reqRef.current) return;
      setView(sorted);
      setSort(next);
    } catch (err) {
      sortFailed(err);
    }
  }

  async function sortFiltered(columnIndex: number) {
    if (filteredId === null) return;
    const requestId = ++filteredSortReq.current;
    const next = nextSort(filteredSort, columnIndex);
    setFilteredPage(1);
    if (!next) {
      setFilteredView(null);
      setFilteredSort(null);
      return;
    }
    try {
      // Sort the full filter result server-side (by id), not the visible preview.
      const sorted = await invoke<TablePreview>("sort_filtered", {
        id: filteredId,
        column: columnIndex,
        ascending: next.ascending,
      });
      if (requestId !== filteredSortReq.current) return;
      setFilteredView(sorted);
      setFilteredSort(next);
    } catch (err) {
      sortFailed(err);
    }
  }

  async function sortComparisonResult(columnIndex: number) {
    if (comparisonId === null) return;
    const requestId = ++comparisonSortReq.current;
    const next = nextSort(comparisonSort, columnIndex);
    setComparisonPage(1);
    if (!next) {
      setComparisonView(null);
      setComparisonSort(null);
      return;
    }
    try {
      // Sort the full compare result server-side (by id), not the visible preview.
      const sorted = await invoke<ComparisonPreview>("sort_comparison", {
        id: comparisonId,
        column: columnIndex,
        ascending: next.ascending,
      });
      if (requestId !== comparisonSortReq.current) return;
      setComparisonView(sorted);
      setComparisonSort(next);
    } catch (err) {
      sortFailed(err);
    }
  }

  // Maps a display `SortState` to the backend `SortSpec` (or null when unsorted), so an export
  // writes the full data in the same order shown on screen.
  function sortSpec(sort: SortState | null): {
    column: number;
    ascending: boolean;
  } | null {
    return sort ? { column: sort.index, ascending: sort.ascending } : null;
  }

  async function exportResult() {
    if (filteredId === null || !filtered || filtered.totalRows === 0) return;
    setExporting(true);
    try {
      // Export the full filter result server-side (by id), in the current on-screen sort order.
      // `export_filtered` returns false when the user cancels the save dialog; nothing to do.
      const saved = await invoke<boolean>("export_filtered", {
        id: filteredId,
        sort: sortSpec(filteredSort),
      });
      if (saved) {
        notifications.show({
          color: "green",
          title: "Export complete",
          message: "Filtered spreadsheet saved.",
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

  async function exportComparison() {
    if (comparisonId === null || !comparison || comparison.totalRows === 0)
      return;
    setExporting(true);
    try {
      // Export the full compare result server-side (by id), in the current on-screen sort order.
      const saved = await invoke<boolean>("export_comparison", {
        id: comparisonId,
        sort: sortSpec(comparisonSort),
      });
      if (saved) {
        notifications.show({
          color: "green",
          title: "Export complete",
          message: "Comparison spreadsheet saved.",
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

  const canExport = !!filtered && filtered.totalRows > 0;
  const canExportComparison = !!comparison && comparison.totalRows > 0;
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
    // Plain flex column we fully control: the two row regions (the source panels and the
    // filter panel) each take half the remaining height with `minHeight: 0` so they shrink
    // and scroll internally instead of overflowing.
    <Box
      style={{
        flex: 1,
        minHeight: 0,
        display: "flex",
        flexDirection: "column",
        gap: "var(--mantine-spacing-md)",
      }}
    >
      <Group justify="space-between">
        <ExcelNotes />
        <Button
          variant="default"
          onClick={swapSides}
          disabled={!leftTable && !rightTable}
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
          table={leftView ?? leftTable}
          path={leftPath}
          loading={leftLoading}
          page={leftPage}
          onPageChange={setLeftPage}
          onLoad={() => loadCsv("left")}
          sort={leftSort}
          onSort={(columnIndex) => sortSide("left", columnIndex)}
        />
        <TablePanel
          title="Right"
          table={rightView ?? rightTable}
          path={rightPath}
          loading={rightLoading}
          page={rightPage}
          onPageChange={setRightPage}
          onLoad={() => loadCsv("right")}
          sort={rightSort}
          onSort={(columnIndex) => sortSide("right", columnIndex)}
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
                table={filteredView ?? filtered}
                page={filteredPage}
                onPageChange={setFilteredPage}
                sort={filteredSort}
                onSort={sortFiltered}
              />
            ) : (
              <Text c="dimmed" fs="italic">
                {leftTable && rightTable
                  ? "Pick a column from the right spreadsheet to filter the left spreadsheet."
                  : "Load both CSV / Excel files to filter."}
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
                result={comparisonView ?? comparison}
                page={comparisonPage}
                onPageChange={setComparisonPage}
                sort={comparisonSort}
                onSort={sortComparisonResult}
              />
            ) : (
              <Text c="dimmed" fs="italic">
                {!leftTable || !rightTable
                  ? "Load both CSV / Excel files to compare."
                  : commonCols.length === 0
                    ? "The two spreadsheets share no columns to compare."
                    : "Pick key and value columns to compare."}
              </Text>
            )}
          </>
        )}
      </Paper>
    </Box>
  );
}
