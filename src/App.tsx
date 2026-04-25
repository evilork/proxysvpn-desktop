import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Power, Zap, AlertCircle } from "lucide-react";
import logo from "./assets/logo.png";
import "./App.css";
import ThemeToggle from "./components/ThemeToggle";
import LogViewer from "./components/LogViewer";

type Status = "disconnected" | "connecting" | "connected";

interface ConnectResult {
  ok: boolean;
  remark: string;
  host: string;
  port: number;
  socks_port: number;
  http_port: number;
}

const SUB_URL_KEY = "proxysvpn_sub_url";
const PING_INTERVAL_MS = 5000;

function App() {
  const [status, setStatus] = useState<Status>("disconnected");
  const [subUrl, setSubUrl] = useState<string>("");
  const [info, setInfo] = useState<ConnectResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [ping, setPing] = useState<number | null>(null);
  const pingTimer = useRef<number | null>(null);

  useEffect(() => {
    const saved = localStorage.getItem(SUB_URL_KEY);
    if (saved) setSubUrl(saved);
    invoke<boolean>("vpn_status").then((running) => {
      if (running) setStatus("connected");
    });
  }, []);

  // Periodically measure true TCP-ping through the tunnel while connected.
  useEffect(() => {
    if (status !== "connected") {
      if (pingTimer.current) {
        clearInterval(pingTimer.current);
        pingTimer.current = null;
      }
      setPing(null);
      return;
    }

    const measure = async () => {
      try {
        const ms = await invoke<number>("vpn_ping");
        setPing(ms);
      } catch {
        setPing(null);
      }
    };

    measure();
    pingTimer.current = window.setInterval(measure, PING_INTERVAL_MS);
    return () => {
      if (pingTimer.current) {
        clearInterval(pingTimer.current);
        pingTimer.current = null;
      }
    };
  }, [status]);

  const handleToggle = async () => {
    setError(null);
    if (status === "connected") {
      try {
        await invoke("vpn_disconnect");
        setStatus("disconnected");
        setInfo(null);
      } catch (e) {
        setError(String(e));
      }
      return;
    }

    if (!subUrl.trim()) {
      setError("Введите ссылку на подписку");
      return;
    }

    setStatus("connecting");
    try {
      localStorage.setItem(SUB_URL_KEY, subUrl.trim());
      const result = await invoke<ConnectResult>("vpn_connect", {
        subUrl: subUrl.trim(),
      });
      setInfo(result);
      setStatus("connected");
    } catch (e) {
      setError(String(e));
      setStatus("disconnected");
    }
  };

  const statusLabel = {
    disconnected: "Не подключено",
    connecting: "Подключение...",
    connected: "Подключено",
  }[status];

  const statusColor = {
    disconnected: "text-nm-text-secondary",
    connecting: "text-yellow-400",
    connected: "text-nm-accent",
  }[status];

  const pingColor =
    ping === null
      ? "text-nm-text-secondary"
      : ping < 100
      ? "text-nm-accent"
      : ping < 200
      ? "text-yellow-400"
      : "text-red-400";

  return (
    <div className="h-screen w-screen flex flex-col items-center justify-between p-6 bg-nm-bg">
      <div className="w-full flex items-center justify-between">
        <div className="flex items-center gap-2">
          <img src={logo} alt="ProxysVPN" className="w-6 h-6 rounded" />
          <span className="font-semibold text-sm">ProxysVPN</span>
        </div>
      </div>

      {status !== "connected" && (
        <div className="w-full">
          <label className="text-xs text-nm-text-secondary mb-1.5 block">
            Ссылка на подписку
          </label>
          <input
            type="text"
            value={subUrl}
            onChange={(e) => setSubUrl(e.target.value)}
            placeholder="https://proxysvpn.com/api/sub/..."
            disabled={status === "connecting"}
            className="nm-pressed-sm w-full px-3 py-2.5 text-xs text-nm-text bg-transparent outline-none disabled:opacity-50"
          />
          {error && (
            <div className="mt-2 text-xs text-red-400 flex items-start gap-1.5">
              <AlertCircle className="w-3.5 h-3.5 flex-shrink-0 mt-0.5" />
              <span className="break-words">{error}</span>
            </div>
          )}
        </div>
      )}

      <div className="flex flex-col items-center gap-6">
        <button
          onClick={handleToggle}
          disabled={status === "connecting"}
          className={`w-40 h-40 flex items-center justify-center transition-all ${
            status === "connected" ? "nm-btn-accent" : "nm-raised"
          }`}
          style={{ borderRadius: "24px" }}
        >
          <Power
            className={`w-16 h-16 ${
              status === "connected" ? "text-[#0d1117]" : "text-nm-text-secondary"
            } ${status === "connecting" ? "animate-pulse" : ""}`}
          />
        </button>

        <div className="text-center">
          <div className={`text-lg font-semibold ${statusColor}`}>
            {statusLabel}
          </div>
          {status === "connected" && (
            <div className={`text-xs mt-1 flex items-center justify-center gap-1 ${pingColor}`}>
              <Zap className="w-3 h-3" />
              {ping === null ? "измерение..." : `Пинг: ${ping} ms`}
            </div>
          )}
        </div>
      </div>

      <div className="w-full text-center text-xs text-nm-text-secondary">
        {status === "connected" && info
          ? `${info.remark || info.host}`
          : "Введите ссылку подписки и нажмите кнопку"}
      </div>
          <span className="version-label">v0.1.0</span>
          <ThemeToggle />
      <LogViewer />
    </div>
  );
}

export default App;
