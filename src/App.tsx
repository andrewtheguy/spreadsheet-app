import { useState } from "react";
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
        style={{ borderStyle: "dashed", minHeight: 120 }}
      >
        <Text c="dimmed" fs="italic">
          placeholder for now
        </Text>
      </Paper>
    </Stack>
  );
}

export default App;
