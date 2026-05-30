import { useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Alert,
  Button,
  Group,
  Paper,
  Stack,
  Table,
  Text,
  Title,
} from "@mantine/core";

type CsvTable = { headers: string[]; rows: string[][] };

function CsvTableView({ table }: { table: CsvTable }) {
  return (
    <Table.ScrollContainer minWidth={0} style={{ flex: 1, minHeight: 0 }}>
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
    </Table.ScrollContainer>
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
      style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column" }}
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

  // Left rows whose (trimmed) first-column value also appears, trimmed, in the right
  // table's first column. Blank/whitespace-only keys are skipped on both sides; the match
  // is case-sensitive. Original (untrimmed) cells are preserved in the output.
  const merged = useMemo<CsvTable | null>(() => {
    if (!leftTable || !rightTable) return null;
    const rightKeys = new Set(
      rightTable.rows
        .map((row) => row[0]?.trim() ?? "")
        .filter((key) => key !== ""),
    );
    const rows = leftTable.rows.filter((row) => {
      const key = row[0]?.trim() ?? "";
      return key !== "" && rightKeys.has(key);
    });
    return { headers: leftTable.headers, rows };
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
    <Stack h="100vh" p="md" gap="md">
      <Title order={1} ta="center">
        Spreadsheet Merge
      </Title>

      <Group grow align="stretch" style={{ flex: 1, minHeight: 0 }}>
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
      </Group>

      <Paper
        withBorder
        p="sm"
        style={{
          flex: 1,
          minHeight: 0,
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
            {merged
              ? "No left rows match the right CSV's first column."
              : "Load both CSVs to see matched rows."}
          </Text>
        )}
      </Paper>
    </Stack>
  );
}

export default App;
