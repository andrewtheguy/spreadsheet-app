import { useState } from "react";
import { Box, Tabs, Title } from "@mantine/core";
import FilterCompareView from "./FilterCompareView";
import ConvertView from "./ConvertView";

// The app's use cases. Each tab renders its own self-contained view; they share nothing but
// the backend `TableStore` (each view manages its own ids there).
type UseCase = "filter-compare" | "convert";

function App() {
  const [useCase, setUseCase] = useState<UseCase>("filter-compare");

  return (
    // Full-height flex column: a fixed header + tab bar, then the active view fills the rest
    // with `minHeight: 0` so its internal tables scroll instead of overflowing the viewport.
    <Box
      style={{
        height: "100vh",
        boxSizing: "border-box",
        padding: "var(--mantine-spacing-md)",
        display: "flex",
        flexDirection: "column",
        gap: "var(--mantine-spacing-sm)",
      }}
    >
      <Title order={1} ta="center">
        Spreadsheet Tools
      </Title>

      <Tabs
        value={useCase}
        onChange={(value) => setUseCase(value as UseCase)}
        style={{
          flex: 1,
          minHeight: 0,
          display: "flex",
          flexDirection: "column",
        }}
      >
        <Tabs.List>
          <Tabs.Tab value="filter-compare">Filter &amp; Compare</Tabs.Tab>
          <Tabs.Tab value="convert">Convert to CSV</Tabs.Tab>
        </Tabs.List>

        {/* Both views stay mounted so loaded files persist across tab switches; only the
            active one is displayed. Managing visibility here (rather than via `Tabs.Panel`)
            lets the active view be a flex container that fills the remaining height without
            an inline `display` fighting Mantine's `display:none` on the hidden panel. */}
        <Box
          style={{
            flex: 1,
            minHeight: 0,
            paddingTop: "var(--mantine-spacing-md)",
            display: useCase === "filter-compare" ? "flex" : "none",
          }}
        >
          <FilterCompareView />
        </Box>
        <Box
          style={{
            flex: 1,
            minHeight: 0,
            paddingTop: "var(--mantine-spacing-md)",
            display: useCase === "convert" ? "flex" : "none",
          }}
        >
          <ConvertView />
        </Box>
      </Tabs>
    </Box>
  );
}

export default App;
