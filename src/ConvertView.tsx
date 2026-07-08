import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Box,
  Button,
  Group,
  List,
  Paper,
  Popover,
  SegmentedControl,
  Text,
} from "@mantine/core";
import { notifications } from "@mantine/notifications";
import {
  CsvTableView,
  ExcelNotes,
  LoadedCsv,
  TablePreview,
  truncatePath,
} from "./shared";

// Which way the converter runs. The source is loaded the same way either direction (the picker
// accepts both), so this only selects the export command, the output extension, and the labels.
type Direction = "excel-to-csv" | "csv-to-excel";

// Returns the basename of `path` with its extension swapped for `ext` (e.g.
// `.../sales.xlsx` → `sales.csv`). Handles POSIX (`/`) and Windows (`\`) separators.
function withExtension(path: string, ext: string): string {
  const sepIndex = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  const base = sepIndex >= 0 ? path.slice(sepIndex + 1) : path;
  const dot = base.lastIndexOf(".");
  const stem = dot > 0 ? base.slice(0, dot) : base;
  return `${stem}.${ext}`;
}

// Explains that the CSV→Excel export writes every cell as text — the deliberate behavior that
// avoids Excel corrupting leading zeros, long IDs, and date-like strings on open.
function TextOutputNotes() {
  return (
    <Popover width={420} position="bottom-start" withArrow shadow="md">
      <Popover.Target>
        <Button variant="subtle" size="compact-sm" color="gray">
          Excel output notes
        </Button>
      </Popover.Target>
      <Popover.Dropdown>
        <Text fw={600} size="sm" mb={4}>
          Every cell is written as text
        </Text>
        <Text size="sm" mb="xs">
          CSV has no cell types, so Excel guesses one for each value on open — and those guesses
          corrupt data. To prevent that, every cell (headers included) is written with Excel&apos;s
          <strong> text</strong> format, preserving the CSV value exactly.
        </Text>
        <List size="sm" spacing={4}>
          <List.Item>
            <strong>Leading zeros are kept</strong> — a zip code or ID like <code>00123</code>
            stays <code>00123</code> instead of becoming <code>123</code>.
          </List.Item>
          <List.Item>
            <strong>Long digit strings stay literal</strong> — a 16-digit account number
            won&apos;t be shown in scientific notation or rounded.
          </List.Item>
          <List.Item>
            <strong>Date-like text isn&apos;t reinterpreted</strong> — <code>3/4</code> stays
            <code> 3/4</code>, not a converted date.
          </List.Item>
          <List.Item>
            Excel may show a green &quot;number stored as text&quot; marker on numeric-looking
            cells — that is expected and keeps the value intact.
          </List.Item>
        </List>
      </Popover.Dropdown>
    </Popover>
  );
}

// The spreadsheet converter. Loads a CSV or Excel file via the shared `load_csv` command (both
// parse into the same table), previews how it was interpreted, then writes it back out in the
// other format. Excel→CSV normalizes Excel's formatting (see ExcelNotes); CSV→Excel writes
// every cell as text (see TextOutputNotes) so numeric-looking values survive intact.
export default function ConvertView() {
  const [direction, setDirection] = useState<Direction>("excel-to-csv");
  const [table, setTable] = useState<TablePreview | null>(null);
  const [id, setId] = useState<number | null>(null);
  const [path, setPath] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [page, setPage] = useState(1);

  const toCsv = direction === "excel-to-csv";
  const loadLabel = toCsv ? "Load Excel file" : "Load CSV file";
  const exportLabel = toCsv ? "Export as CSV" : "Export as Excel";
  const emptyHint = toCsv
    ? "Load an Excel (.xlsx / .xls) file to preview it, then export it as CSV."
    : "Load a CSV file to preview it, then export it as Excel (all cells written as text).";

  async function loadFile() {
    setLoading(true);
    try {
      // Pass the current id as `replace` so the backend store evicts the previous table.
      // `load_csv` returns null when the user cancels the dialog; keep the current table.
      const result = await invoke<LoadedCsv | null>("load_csv", {
        replace: id,
      });
      if (result) {
        setTable(result.table);
        setId(result.id);
        setPath(result.path);
        setPage(1);
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

  async function exportFile() {
    if (id === null || !path) return;
    setExporting(true);
    try {
      // Both commands write the full stored table (not just the preview) and return false when
      // the user cancels the save dialog.
      const command = toCsv ? "export_csv" : "export_xlsx";
      const saved = await invoke<boolean>(command, {
        id,
        defaultName: withExtension(path, toCsv ? "csv" : "xlsx"),
      });
      if (saved) {
        notifications.show({
          color: "green",
          title: "Export complete",
          message: toCsv ? "CSV file saved." : "Excel file saved.",
        });
      }
    } catch (err) {
      console.error("Failed to export:", err);
      notifications.show({
        color: "red",
        title: "Export failed",
        message: String(err),
      });
    } finally {
      setExporting(false);
    }
  }

  return (
    <Box
      style={{
        flex: 1,
        minHeight: 0,
        display: "flex",
        flexDirection: "column",
        gap: "var(--mantine-spacing-md)",
      }}
    >
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
          value={direction}
          onChange={(value) => setDirection(value as Direction)}
          data={[
            { value: "excel-to-csv", label: "Excel → CSV" },
            { value: "csv-to-excel", label: "CSV → Excel" },
          ]}
          size="sm"
          mb="sm"
          style={{ alignSelf: "flex-start" }}
        />

        <Group justify="space-between" mb="sm" wrap="nowrap" align="flex-end">
          <Group gap="xs" align="center" wrap="nowrap" style={{ minWidth: 0 }}>
            <Button onClick={loadFile} loading={loading}>
              {loadLabel}
            </Button>
            {toCsv ? <ExcelNotes /> : <TextOutputNotes />}
            {path && (
              <Text size="xs" c="dimmed" title={path} truncate>
                {truncatePath(path)}
              </Text>
            )}
          </Group>
          <Button
            onClick={exportFile}
            loading={exporting}
            disabled={id === null}
          >
            {exportLabel}
          </Button>
        </Group>

        {table ? (
          <CsvTableView table={table} page={page} onPageChange={setPage} />
        ) : (
          <Text c="dimmed" fs="italic">
            {emptyHint}
          </Text>
        )}
      </Paper>
    </Box>
  );
}
