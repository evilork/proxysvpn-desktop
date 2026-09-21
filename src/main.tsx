import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { applyTheme, readThemePref } from "./prefs";

// Before the first render, not inside an effect: an effect runs after the
// first paint, so the window would show one frame of the dark palette to a
// person whose system — or stored choice — is light.
applyTheme(readThemePref());

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
