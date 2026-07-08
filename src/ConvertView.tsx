import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Box, Button, Group, Paper, Text } from "@mantine/core";
import { notifications } from "@mantine/notifications";
import {
  CsvTableView,
  ExcelNotes,
  LoadedCsv,
  TablePreview,
  truncatePath,
} from "./shared";

// Derives the default CSV filename for the save dialog from the loaded source path: the
// basename with its extension swapped for `.csv` (e.g. `.../sales.xlsx` → `sales.csv`).
// Handles both POSIX (`/`) and Windows (`\`) separators.
function csvNameFrom(path: string): string {
  const sepIndex = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  const base = sepIndex >= 0 ? path.slice(sepIndex + 1) : path;
  const dot = base.lastIndexOf(".");
  const stem = dot > 0 ? base.slice(0, dot) : base;
  return `${stem}.csv`;
}

// The Excel → CSV converter. Loads a spreadsheet via the shared `load_csv` command (which
// parses .xlsx/.xls through `sheet_core::parse_excel`), previews how it was interpreted, then
// writes it out as CSV via `export_csv`. The preview + Excel notes matter here: the user
// confirms the parse before saving, since Excel formatting is only partially reconstructed.
export default function ConvertView() {
  const [table, setTable] = useState<TablePreview | null>(null);
  const [id, setId] = useState<number | null>(null);
  const [path, setPath] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [page, setPage] = useState(1);

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

  async function exportCsv() {
    if (id === null || !path) return;
    setExporting(true);
    try {
      // `export_csv` writes the full stored table (not just the preview) and returns false
      // when the user cancels the save dialog.
      const saved = await invoke<boolean>("export_csv", {
        id,
        defaultName: csvNameFrom(path),
      });
      if (saved) {
        notifications.show({
          color: "green",
          title: "Export complete",
          message: "CSV file saved.",
        });
      }
    } catch (err) {
      console.error("Failed to export CSV:", err);
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
        <Group justify="space-between" mb="sm" wrap="nowrap" align="flex-end">
          <Group gap="xs" align="center" wrap="nowrap" style={{ minWidth: 0 }}>
            <Button onClick={loadFile} loading={loading}>
              Load Excel file
            </Button>
            <ExcelNotes />
            {path && (
              <Text size="xs" c="dimmed" title={path} truncate>
                {truncatePath(path)}
              </Text>
            )}
          </Group>
          <Button
            onClick={exportCsv}
            loading={exporting}
            disabled={id === null}
          >
            Export as CSV
          </Button>
        </Group>

        {table ? (
          <CsvTableView table={table} page={page} onPageChange={setPage} />
        ) : (
          <Text c="dimmed" fs="italic">
            Load an Excel (.xlsx / .xls) file to preview it, then export it as CSV.
          </Text>
        )}
      </Paper>
    </Box>
  );
}
