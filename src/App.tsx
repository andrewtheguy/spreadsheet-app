import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Box,
  Button,
  Group,
  Pagination,
  Paper,
  Table,
  Text,
  Title,
} from "@mantine/core";
import { notifications } from "@mantine/notifications";

type CsvTable = { headers: string[]; rows: string[][] };
type LoadedCsv = { table: CsvTable; path: string };

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

function App() {
  const [leftTable, setLeftTable] = useState<CsvTable | null>(null);
  const [leftPath, setLeftPath] = useState<string | null>(null);
  const [leftLoading, setLeftLoading] = useState(false);
  const [leftPage, setLeftPage] = useState(1);

  const [rightTable, setRightTable] = useState<CsvTable | null>(null);
  const [rightPath, setRightPath] = useState<string | null>(null);
  const [rightLoading, setRightLoading] = useState(false);
  const [rightPage, setRightPage] = useState(1);

  const [exporting, setExporting] = useState(false);
  const [merged, setMerged] = useState<CsvTable | null>(null);
  const [mergedPage, setMergedPage] = useState(1);

  // The merge runs in the Rust `sheet-core` crate via the `merge_csv` command. Recompute
  // it whenever either source table changes; a cancellation flag drops a stale response if
  // the inputs change again before it resolves.
  useEffect(() => {
    if (!leftTable || !rightTable) {
      setMerged(null);
      return;
    }
    let cancelled = false;
    invoke<CsvTable>("merge_csv", { left: leftTable, right: rightTable })
      .then((result) => {
        if (cancelled) return;
        setMerged(result);
        setMergedPage(1);
      })
      .catch((err) => {
        if (cancelled) return;
        console.error("Failed to merge:", err);
        notifications.show({
          color: "red",
          title: "Merge failed",
          message: String(err),
        });
        setMerged(null);
      });
    return () => {
      cancelled = true;
    };
  }, [leftTable, rightTable]);

  async function loadCsv(
    setTable: (table: CsvTable) => void,
    setPath: (path: string) => void,
    setLoading: (loading: boolean) => void,
    setPage: (page: number) => void,
  ) {
    setLoading(true);
    try {
      // `load_csv` returns null when the user cancels the dialog; keep the current table.
      const result = await invoke<LoadedCsv | null>("load_csv");
      if (result) {
        setTable(result.table);
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
    setLeftPath(rightPath);
    setRightPath(leftPath);
    setLeftPage(1);
    setRightPage(1);
  }

  async function exportResult() {
    if (!merged || merged.rows.length === 0) return;
    setExporting(true);
    try {
      // `save_csv` returns false when the user cancels the save dialog; nothing to do.
      await invoke<boolean>("save_csv", { table: merged });
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

  const mergedCount = merged?.rows.length ?? 0;

  return (
    // Plain flex column we fully control: a 100vh box whose two row regions (the source
    // panels and the merged panel) each take half the remaining height with `minHeight: 0`
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
          Spreadsheet Merge
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
          onLoad={() =>
            loadCsv(setLeftTable, setLeftPath, setLeftLoading, setLeftPage)
          }
        />
        <TablePanel
          title="Right"
          table={rightTable}
          path={rightPath}
          loading={rightLoading}
          page={rightPage}
          onPageChange={setRightPage}
          onLoad={() =>
            loadCsv(setRightTable, setRightPath, setRightLoading, setRightPage)
          }
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
        <Group justify="space-between" mb="sm">
          <Group gap="xs">
            <Title order={2} size="h4">
              Merged
            </Title>
            {merged && (
              <Text c="dimmed" size="sm">
                {mergedCount} {mergedCount === 1 ? "row" : "rows"}
              </Text>
            )}
          </Group>
          <Button
            onClick={exportResult}
            loading={exporting}
            disabled={mergedCount === 0}
          >
            Export result
          </Button>
        </Group>

        {merged && mergedCount > 0 ? (
          <CsvTableView
            table={merged}
            page={mergedPage}
            onPageChange={setMergedPage}
          />
        ) : (
          <Text c="dimmed" fs="italic">
            {leftTable && rightTable
              ? "No left rows match the right CSV's first column."
              : "Load both CSVs to see matched rows."}
          </Text>
        )}
      </Paper>
    </Box>
  );
}

export default App;
