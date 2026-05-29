import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

type CsvTable = { headers: string[]; rows: string[][] };

function App() {
  const [table, setTable] = useState<CsvTable | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);

  async function loadCsv() {
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
      <h1>Spreadsheet</h1>

      <div className="row">
        <button onClick={loadCsv} disabled={loading}>
          {loading ? "Loading…" : "Load CSV"}
        </button>
      </div>

      {error && <p className="error">{error}</p>}

      {table && (
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
      )}
    </main>
  );
}

export default App;
