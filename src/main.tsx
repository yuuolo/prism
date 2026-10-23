import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles.css";

const render = () =>
  ReactDOM.createRoot(document.getElementById("root")!).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );

// In a plain browser during development, preview the UI with sample data.
if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
  import("./devmock").then((m) => {
    m.installDevMock();
    render();
  });
} else {
  render();
}
