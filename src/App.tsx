import { useState, type MouseEvent } from "react";
import reactLogo from "./assets/react.svg";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

// The embedded Chromium (CEF) webview would otherwise navigate external links
// inside the app, so route them to the OS default browser via a Rust command.
async function openExternal(e: MouseEvent<HTMLAnchorElement>) {
  e.preventDefault();
  // Capture the href before awaiting: React resets currentTarget after the handler.
  const url = e.currentTarget.href;
  try {
    await invoke("open_external", { url });
  } catch (err) {
    console.error(`Failed to open ${url}:`, err);
    alert(`Failed to open ${url}: ${err}`);
  }
}

function App() {
  const [greetMsg, setGreetMsg] = useState("");
  const [name, setName] = useState("");

  async function greet() {
    // Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
    setGreetMsg(await invoke("greet", { name }));
  }

  return (
    <main className="container">
      <h1>Welcome to Tauri + React</h1>

      <div className="row">
        <a href="https://vite.dev" target="_blank" onClick={openExternal}>
          <img src="/vite.svg" className="logo vite" alt="Vite logo" />
        </a>
        <a href="https://tauri.app" target="_blank" onClick={openExternal}>
          <img src="/tauri.svg" className="logo tauri" alt="Tauri logo" />
        </a>
        <a href="https://react.dev" target="_blank" onClick={openExternal}>
          <img src={reactLogo} className="logo react" alt="React logo" />
        </a>
      </div>
      <p>Click on the Tauri, Vite, and React logos to learn more.</p>

      <form
        className="row"
        onSubmit={(e) => {
          e.preventDefault();
          greet();
        }}
      >
        <input
          id="greet-input"
          onChange={(e) => setName(e.currentTarget.value)}
          placeholder="Enter a name..."
        />
        <button type="submit">Greet</button>
      </form>
      <p>{greetMsg}</p>
    </main>
  );
}

export default App;
