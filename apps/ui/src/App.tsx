import { useState } from "react";

import "./App.css";
import { hostAdapter } from "./host-adapter";

function App() {
  const [checkingHost, setCheckingHost] = useState(false);
  const [hostStatus, setHostStatus] = useState("Host connection has not been checked.");

  async function checkHost() {
    setCheckingHost(true);
    setHostStatus("Checking host connection…");

    try {
      await hostAdapter.ping();
      setHostStatus("Wallpaper host connected.");
    } catch (error) {
      setHostStatus(
        error instanceof Error
          ? error.message
          : "Could not check the wallpaper host.",
      );
    } finally {
      setCheckingHost(false);
    }
  }

  return (
    <main className="app-shell">
      <h1>wallpaper-lite</h1>
      <p>Desktop application shell</p>
      <button className="host-check" disabled={checkingHost} onClick={checkHost}>
        {checkingHost ? "Checking…" : "Check wallpaper host"}
      </button>
      <p aria-live="polite" className="host-status" role="status">
        {hostStatus}
      </p>
    </main>
  );
}

export default App;
