import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { readText } from "@tauri-apps/plugin-clipboard-manager";
import { Power, Zap, AlertCircle, ClipboardPaste, QrCode, Settings, X } from "lucide-react";
import QRCode from "qrcode";
import logo from "./assets/logo.png";
import "./App.css";
import { makeT, type Lang } from "./i18n";
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
  const [showQr, setShowQr] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [qrDataUrl, setQrDataUrl] = useState<string>("");
  const [theme, setTheme] = useState<"light" | "dark">(() => {
    const t = localStorage.getItem("proxysvpn_theme");
    return t === "light" || t === "dark" ? t : "dark";
  });
  const [lang, setLang] = useState<Lang>(() => {
    const l = localStorage.getItem("proxysvpn_lang");
    return l === "en" ? "en" : "ru";
  });
  const t = makeT(lang);
  const [servers, setServers] = useState<{ index: number; remark: string; host: string; port: number; proto: string }[]>([]);
  const [selectedServer, setSelectedServer] = useState<number>(() => {
    const v = localStorage.getItem("proxysvpn_server");
    return v ? parseInt(v, 10) || 0 : 0;
  });
  const pingTimer = useRef<number | null>(null);

  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
    localStorage.setItem("proxysvpn_theme", theme);
  }, [theme]);

  useEffect(() => {
    localStorage.setItem("proxysvpn_lang", lang);
  }, [lang]);

  useEffect(() => {
    const saved = localStorage.getItem(SUB_URL_KEY);
    if (saved) setSubUrl(saved);
    invoke<boolean>("vpn_status").then((running) => {
      if (running) setStatus("connected");
    });
  }, []);

  // Подгрузить список серверов, когда есть ссылка подписки.
  const loadServers = async (url: string) => {
    if (!url.trim()) return;
    try {
      const list = await invoke<typeof servers>("list_servers", { subUrl: url.trim() });
      setServers(list);
      if (selectedServer >= list.length) setSelectedServer(0);
    } catch {
      setServers([]);
    }
  };

  useEffect(() => {
    if (subUrl.trim()) loadServers(subUrl);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [subUrl]);

  useEffect(() => {
    localStorage.setItem("proxysvpn_server", String(selectedServer));
  }, [selectedServer]);

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

  // Connect using whatever sub URL is currently stored in state.
  const connectWith = async (url: string) => {
    setStatus("connecting");
    try {
      localStorage.setItem(SUB_URL_KEY, url);
      const result = await invoke<ConnectResult>("vpn_connect", { subUrl: url, serverIndex: selectedServer });
      setInfo(result);
      setStatus("connected");
    } catch (e) {
      setError(String(e));
      setStatus("disconnected");
    }
  };

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
      setError(t("err.noSub"));
      return;
    }
    await connectWith(subUrl.trim());
  };

  // Read clipboard, validate it looks like a sub link, store + connect.
  const handlePaste = async () => {
    setError(null);
    try {
      const text = (await readText()).trim();
      if (!text || !/^https?:\/\//i.test(text)) {
        setError(t("err.clipboardEmpty"));
        return;
      }
      setSubUrl(text);
      await connectWith(text);
    } catch {
      setError(t("err.clipboardRead"));
    }
  };

  // Заглушка: показываем QR-плейсхолдер. Pairing с сервером пока отключён.
  const handleShowQr = async () => {
    setError(null);
    try {
      const img = await QRCode.toDataURL("proxysvpn-pairing-soon", {
        width: 280,
        margin: 1,
        color: { dark: "#0d1117", light: "#ffffff" },
      });
      setQrDataUrl(img);
      setShowQr(true);
    } catch {
      setError(t("err.qr"));
    }
  };


  const statusLabel = {
    disconnected: t("status.disconnected"),
    connecting: t("status.connecting"),
    connected: t("status.connected"),
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
      {/* Header */}
      <div className="w-full flex items-center justify-between">
        <div className="flex items-center gap-2">
          <img src={logo} alt="ProxysVPN" className="w-6 h-6 rounded" />
          <span className="font-semibold text-sm">ProxysVPN</span>
        </div>
        <button
          onClick={() => setShowSettings(true)}
          className="nm-circle-pressed w-8 h-8 flex items-center justify-center"
          title={t("settings.title")}
        >
          <Settings className="w-4 h-4 text-nm-text-secondary" />
        </button>
      </div>

      {/* Power button + status */}
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
          <div className={`text-lg font-semibold ${statusColor}`}>{statusLabel}</div>
          {status === "connected" && (
            <div className={`text-xs mt-1 flex items-center justify-center gap-1 ${pingColor}`}>
              <Zap className="w-3 h-3" />
              {ping === null ? t("ping.measuring") : `${t("ping.label")}: ${ping} ms`}
            </div>
          )}
        </div>

        {error && (
          <div className="text-xs text-red-400 flex items-start gap-1.5 max-w-[260px]">
            <AlertCircle className="w-3.5 h-3.5 flex-shrink-0 mt-0.5" />
            <span className="break-words">{error}</span>
          </div>
        )}
      </div>

      {/* Bottom: paste + QR buttons (hidden when connected) */}
      {status !== "connected" && servers.length > 1 && (
        <div className="w-full flex items-center justify-center gap-2 mb-1">
          <span className="text-xs text-nm-text-secondary">{t("settings.server")}:</span>
          <select
            value={selectedServer}
            onChange={(e) => setSelectedServer(parseInt(e.target.value, 10))}
            className="nm-pressed-sm text-xs text-nm-text bg-transparent px-2 py-1.5 rounded-lg outline-none"
          >
            {servers.map((srv) => (
              <option key={srv.index} value={srv.index} className="bg-nm-bg text-nm-text">
                {(srv.remark || srv.host) + " · " + srv.proto}
              </option>
            ))}
          </select>
        </div>
      )}

      {status !== "connected" ? (
        <div className="w-full flex gap-3">
          <button
            onClick={handlePaste}
            disabled={status === "connecting"}
            className="nm-raised flex-1 py-3 flex items-center justify-center gap-2 text-sm font-medium disabled:opacity-50"
          >
            <ClipboardPaste className="w-4 h-4 text-nm-accent" />
            {t("btn.paste")}
          </button>
          <button
            onClick={handleShowQr}
            disabled={status === "connecting"}
            className="nm-raised w-14 flex items-center justify-center disabled:opacity-50"
            title={t("btn.qrTitle")}
          >
            <QrCode className="w-5 h-5 text-nm-accent" />
          </button>
        </div>
      ) : (
        <div className="w-full text-center text-xs text-nm-text-secondary">
          {info ? `${info.remark || info.host}` : ""}
        </div>
      )}

      <div className="w-full flex items-center justify-center mt-2">
        <span className="version-label">v0.1.0</span>
      </div>

      <LogViewer />

      {/* QR modal */}
      {showQr && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/70"
          onClick={() => setShowQr(false)}
        >
          <div
            className="nm-raised p-6 flex flex-col items-center gap-4 mx-6"
            style={{ borderRadius: "20px" }}
            onClick={(e) => e.stopPropagation()}
          >
            <div className="w-full flex items-center justify-between">
              <span className="font-semibold text-sm">{t("qr.title")}</span>
              <button
                onClick={() => setShowQr(false)}
                className="nm-circle-pressed w-7 h-7 flex items-center justify-center"
              >
                <X className="w-4 h-4 text-nm-text-secondary" />
              </button>
            </div>
            {qrDataUrl && (
              <img src={qrDataUrl} alt="QR" className="rounded-lg" width={240} height={240} />
            )}
            <p className="text-xs text-nm-text-secondary text-center max-w-[240px]">
              {t("qr.soon")}
            </p>
          </div>
        </div>
      )}

      {/* Settings modal (placeholder) */}
      {showSettings && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/70"
          onClick={() => setShowSettings(false)}
        >
          <div
            className="nm-raised p-6 flex flex-col items-center gap-4 mx-6 min-w-[260px]"
            style={{ borderRadius: "20px" }}
            onClick={(e) => e.stopPropagation()}
          >
            <div className="w-full flex items-center justify-between">
              <span className="font-semibold text-sm">{t("settings.title")}</span>
              <button
                onClick={() => setShowSettings(false)}
                className="nm-circle-pressed w-7 h-7 flex items-center justify-center"
              >
                <X className="w-4 h-4 text-nm-text-secondary" />
              </button>
            </div>
            <div className="w-full flex flex-col gap-4 py-2">
              <div className="flex items-center justify-between">
                <span className="text-sm text-nm-text">{t("settings.theme")}</span>
                <div className="flex gap-2">
                  <button
                    onClick={() => setTheme("light")}
                    className={`px-3 py-1.5 text-xs rounded-lg ${theme === "light" ? "nm-btn-accent text-[#0d1117]" : "nm-pressed-sm text-nm-text-secondary"}`}
                  >
                    {t("settings.theme.light")}
                  </button>
                  <button
                    onClick={() => setTheme("dark")}
                    className={`px-3 py-1.5 text-xs rounded-lg ${theme === "dark" ? "nm-btn-accent text-[#0d1117]" : "nm-pressed-sm text-nm-text-secondary"}`}
                  >
                    {t("settings.theme.dark")}
                  </button>
                </div>
              </div>
              <div className="flex items-center justify-between">
                <span className="text-sm text-nm-text">{t("settings.lang")}</span>
                <div className="flex gap-2">
                  <button
                    onClick={() => setLang("ru")}
                    className={`px-3 py-1.5 text-xs rounded-lg ${lang === "ru" ? "nm-btn-accent text-[#0d1117]" : "nm-pressed-sm text-nm-text-secondary"}`}
                  >
                    RU
                  </button>
                  <button
                    onClick={() => setLang("en")}
                    className={`px-3 py-1.5 text-xs rounded-lg ${lang === "en" ? "nm-btn-accent text-[#0d1117]" : "nm-pressed-sm text-nm-text-secondary"}`}
                  >
                    EN
                  </button>
                </div>
              </div>
              {subUrl.trim() && (
                <button
                  onClick={() => {
                    localStorage.removeItem(SUB_URL_KEY);
                    localStorage.removeItem("proxysvpn_server");
                    setSubUrl("");
                    setServers([]);
                    setSelectedServer(0);
                    setInfo(null);
                    setShowSettings(false);
                  }}
                  className="nm-pressed-sm w-full py-2.5 text-xs text-red-400 rounded-lg mt-1"
                >
                  {t("settings.clearSub")}
                </button>
              )}
              <p className="text-xs text-nm-text-secondary text-center pt-2">
                {t("settings.soon")}
              </p>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

export default App;
