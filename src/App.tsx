import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

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
    <section className="panel">
      <div className="panel-header">
        <h2>{title}</h2>
        <button onClick={onLoad} disabled={loading}>
          {loading ? "Loading…" : "Load CSV"}
        </button>
      </div>

      {error && <p className="error">{error}</p>}

      {table ? (
        <div className="table-wrapper">
          <table className="csv-table">
            <thead>
              <tr>
                {table.headers.map((header, i) => (
                  <th key={i}>{header}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {table.rows.map((row, r) => (
                <tr key={r}>
                  {row.map((cell, c) => (
                    <td key={c}>{cell}</td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : (
        <p className="placeholder">No spreadsheet loaded.</p>
      )}
    </section>
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
    <main className="container">
      <h1>Spreadsheet Merge</h1>

      <div className="panels">
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
      </div>

      <section className="merged-panel">
        <p className="placeholder">placeholder for now</p>
      </section>
    </main>
  );
}

export default App;
