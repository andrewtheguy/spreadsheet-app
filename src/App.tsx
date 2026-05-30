import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Alert,
  Box,
  Button,
  Group,
  Paper,
  Table,
  Text,
  Title,
} from "@mantine/core";

type CsvTable = { headers: string[]; rows: string[][] };

function CsvTableView({ table }: { table: CsvTable }) {
  return (
    // A bounded, scrollable region inside the panel's flex column: `minHeight: 0` lets it
    // shrink below content size so `overflow: auto` scrolls instead of overflowing the
    // panel. `stickyHeader` pins the header row against this scroll container.
    <Box style={{ flex: 1, minHeight: 0, overflow: "auto" }}>
      <Table stickyHeader striped withTableBorder withColumnBorders>
        <Table.Thead>
          <Table.Tr>
            {table.headers.map((header, i) => (
              <Table.Th key={i} style={{ whiteSpace: "nowrap" }}>
                {header}
              </Table.Th>
            ))}
          </Table.Tr>
        </Table.Thead>
        <Table.Tbody>
          {table.rows.map((row, r) => (
            <Table.Tr key={r}>
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
  );
}

type TablePanelProps = {
  title: string;
  table: CsvTable | null;
  loading: boolean;
  error: string;
  onLoad: () => void;
};

function TablePanel({ title, table, loading, error, onLoad }: TablePanelProps) {
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
      <Group justify="space-between" mb="sm">
        <Title order={2} size="h4">
          {title}
        </Title>
        <Button onClick={onLoad} loading={loading}>
          Load CSV
        </Button>
      </Group>

      {error && (
        <Alert color="red" mb="sm">
          {error}
        </Alert>
      )}

      {table ? (
        <CsvTableView table={table} />
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
  const [leftLoading, setLeftLoading] = useState(false);
  const [leftError, setLeftError] = useState("");

  const [rightTable, setRightTable] = useState<CsvTable | null>(null);
  const [rightLoading, setRightLoading] = useState(false);
  const [rightError, setRightError] = useState("");

  const [exporting, setExporting] = useState(false);
  const [mergedError, setMergedError] = useState("");
  const [merged, setMerged] = useState<CsvTable | null>(null);

  // The merge runs in the Rust `sheet-core` crate via the `merge_csv` command. Recompute
  // it whenever either source table changes; a cancellation flag drops a stale response if
  // the inputs change again before it resolves.
  useEffect(() => {
    if (!leftTable || !rightTable) {
      setMerged(null);
      setMergedError("");
      return;
    }
    let cancelled = false;
    setMergedError("");
    invoke<CsvTable>("merge_csv", { left: leftTable, right: rightTable })
      .then((result) => {
        if (!cancelled) setMerged(result);
      })
      .catch((err) => {
        if (cancelled) return;
        console.error("Failed to merge:", err);
        setMergedError(String(err));
        setMerged(null);
      });
    return () => {
      cancelled = true;
    };
  }, [leftTable, rightTable]);

  async function loadCsv(
    setTable: (table: CsvTable) => void,
    setLoading: (loading: boolean) => void,
    setError: (error: string) => void,
  ) {
    setLoading(true);
    setError("");
    try {
      // `load_csv` returns null when the user cancels the dialog; keep the current table.
      const result = await invoke<CsvTable | null>("load_csv");
      if (result) setTable(result);
    } catch (err) {
      console.error("Failed to load CSV:", err);
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }

  async function exportResult() {
    if (!merged || merged.rows.length === 0) return;
    setExporting(true);
    setMergedError("");
    try {
      // `save_csv` returns false when the user cancels the save dialog; nothing to do.
      await invoke<boolean>("save_csv", { table: merged });
    } catch (err) {
      console.error("Failed to export result:", err);
      setMergedError(String(err));
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
      <Title order={1} ta="center">
        Spreadsheet Merge
      </Title>

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
          loading={leftLoading}
          error={leftError}
          onLoad={() => loadCsv(setLeftTable, setLeftLoading, setLeftError)}
        />
        <TablePanel
          title="Right"
          table={rightTable}
          loading={rightLoading}
          error={rightError}
          onLoad={() => loadCsv(setRightTable, setRightLoading, setRightError)}
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

        {mergedError && (
          <Alert color="red" mb="sm">
            {mergedError}
          </Alert>
        )}

        {merged && mergedCount > 0 ? (
          <CsvTableView table={merged} />
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
