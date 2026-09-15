import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles/theme.css";

// GUI-verification e2e only. Expose the WebDriver focus hook so
// @wdio/tauri-service's per-command focus poll resolves instantly instead of
// hanging its hardcoded 5s. Gated behind the VITE_E2E build flag (set only by
// `npm run tauri:build:e2e`), so this branch is dead-code-eliminated from the
// production bundle. See src/e2e/exposeWdioCore.ts.
if (import.meta.env.VITE_E2E) {
  void import("./e2e/exposeWdioCore").then((m) => m.installWdioCoreHook());
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
