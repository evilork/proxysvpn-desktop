// src/components/LogViewer.tsx
import { useEffect, useState, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AlertCircle, X, Copy, Download, Trash2, RefreshCw } from "lucide-react";

interface LogLine {
  ts_ms: number;
  level: "info" | "warn" | "error";
  source: string;
  message: string;
}

const REFRESH_INTERVAL_MS = 1500;

function formatTimestamp(ts_ms: number): string {
  const d = new Date(ts_ms);
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  const ss = String(d.getSeconds()).padStart(2, "0");
  const ms = String(d.getMilliseconds()).padStart(3, "0");
  return `${hh}:${mm}:${ss}.${ms}`;
}

export default function LogViewer() {
  const [open, setOpen] = useState(false);
  const [logs, setLogs] = useState<LogLine[]>([]);
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState(false);
  const [logFilePath, setLogFilePath] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const lines = await invoke<LogLine[]>("get_logs", { limit: 1000 });
      setLogs(lines);
    } catch (e) {
      // logger missing or backend not registered yet — silently ignore
      console.warn("get_logs failed:", e);
    }
  }, []);

  useEffect(() => {
    if (!open) return;
    refresh();
    invoke<string | null>("get_log_file_path")
      .then((p) => setLogFilePath(p))
      .catch(() => setLogFilePath(null));
    const id = window.setInterval(refresh, REFRESH_INTERVAL_MS);
    return () => clearInterval(id);
  }, [open, refresh]);

  const onCopy = async () => {
    setBusy(true);
    try {
      const text = await invoke<string>("export_logs", {
        includeSystemInfo: true,
      });
      await navigator.clipboard.writeText(text);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch (e) {
      console.error(e);
    } finally {
      setBusy(false);
    }
  };

  const onDownload = async () => {
    setBusy(true);
    try {
      const text = await invoke<string>("export_logs", {
        includeSystemInfo: true,
      });
      const blob = new Blob([text], { type: "text/plain;charset=utf-8" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      const ts = new Date().toISOString().replace(/[:.]/g, "-").split("T");
      a.href = url;
      a.download = `proxysvpn-logs-${ts[0]}.txt`;
      a.click();
      window.setTimeout(() => URL.revokeObjectURL(url), 1000);
    } catch (e) {
      console.error(e);
    } finally {
      setBusy(false);
    }
  };

  const onClear = async () => {
    if (!confirm("Очистить логи?")) return;
    setBusy(true);
    try {
      await invoke("clear_logs");
      await refresh();
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        className="log-toggle nm-btn-sm"
        title="Логи приложения"
        aria-label="Открыть логи"
      >
        <AlertCircle size={14} strokeWidth={2.2} />
      </button>

      {open && (
        <div
          className="log-overlay"
          onClick={(e) => {
            if (e.target === e.currentTarget) setOpen(false);
          }}
        >
          <div className="log-modal nm-raised">
            <div className="log-modal-header">
              <span className="log-modal-title">Логи</span>
              <button
                onClick={() => setOpen(false)}
                className="log-icon-btn"
                aria-label="Закрыть"
              >
                <X size={16} />
              </button>
            </div>

            <div className="log-modal-actions">
              <button
                onClick={onCopy}
                disabled={busy}
                className="log-action-btn nm-btn-sm"
              >
                <Copy size={12} />
                <span>{copied ? "Скопировано" : "Копировать"}</span>
              </button>
              <button
                onClick={onDownload}
                disabled={busy}
                className="log-action-btn nm-btn-sm"
              >
                <Download size={12} />
                <span>Скачать .txt</span>
              </button>
              <button
                onClick={refresh}
                disabled={busy}
                className="log-action-btn nm-btn-sm"
              >
                <RefreshCw size={12} />
                <span>Обновить</span>
              </button>
              <button
                onClick={onClear}
                disabled={busy}
                className="log-action-btn nm-btn-sm log-action-danger"
              >
                <Trash2 size={12} />
                <span>Очистить</span>
              </button>
            </div>

            <div className="log-modal-body nm-pressed-sm">
              {logs.length === 0 ? (
                <div className="log-empty">пусто</div>
              ) : (
                logs.map((l, i) => (
                  <div key={i} className={`log-line log-${l.level}`}>
                    <span className="log-time">{formatTimestamp(l.ts_ms)}</span>{" "}
                    <span className="log-source">[{l.source}]</span>{" "}
                    <span className="log-msg">{l.message}</span>
                  </div>
                ))
              )}
            </div>

            {logFilePath && (
              <div className="log-modal-footer">
                Файл: <code>{logFilePath}</code>
              </div>
            )}
          </div>
        </div>
      )}
    </>
  );
}
