// src/components/LogViewer.tsx
//
// The technical log, two taps below [11] and nowhere near the main screen.
//
// What changed from the old window: it pulled a THOUSAND lines through IPC
// every 1.5 seconds, forever, and painted the lot orange because every line
// of tun2socks stderr was labelled `warn`. Here nothing polls: the log is
// read once, in pages of 200, and only when the person asks for the next one.
//
// Masking is the core's job and happens before a line is written — by the
// time text reaches this component it must already be free of addresses.

import { useCallback, useEffect, useState } from "react";

import { bridge, type LogLine } from "../bridge";
import { Screen, Spinner, useUi } from "./ui";

const PAGE = 200;

export default function LogViewer({ onClose }: { onClose: () => void }) {
  const { t, toast } = useUi();
  const [lines, setLines] = useState<LogLine[]>([]);
  const [loading, setLoading] = useState(false);
  const [end, setEnd] = useState(false);
  const [failed, setFailed] = useState(false);

  const loadPage = useCallback(async (offset: number) => {
    setLoading(true);
    try {
      const page = await bridge.logs(offset, PAGE);
      setLines((previous) => (offset === 0 ? page : [...previous, ...page]));
      if (page.length < PAGE) setEnd(true);
    } catch {
      setFailed(true);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadPage(0);
  }, [loadPage]);

  const copy = useCallback(async () => {
    const text = lines
      .map((line) => `${new Date(line.tsMs).toISOString()} ${line.level} ${line.source} ${line.message}`)
      .join("\n");
    try {
      await bridge.writeClipboard(text);
      toast(t("common.copied"));
    } catch {
      toast(t("tl.techLog.failed"));
    }
  }, [lines, t, toast]);

  return (
    <Screen
      title={t("tl.techLog")}
      onClose={onClose}
      closeKind="back"
      footer={
        <>
          {!end && lines.length > 0 ? (
            <button
              type="button"
              className="btn btn-outline"
              onClick={() => void loadPage(lines.length)}
              disabled={loading}
            >
              {loading ? <Spinner /> : t("tl.techLog.more")}
            </button>
          ) : null}
          <button
            type="button"
            className="btn btn-quiet"
            onClick={() => void copy()}
            disabled={lines.length === 0}
          >
            {t("common.copy")}
          </button>
        </>
      }
    >
      {failed ? <p className="body danger">{t("tl.techLog.failed")}</p> : null}
      {!failed && lines.length === 0 && !loading ? (
        <p className="body dim">{t("tl.techLog.empty")}</p>
      ) : null}
      {lines.length > 0 ? (
        <div className="mono-box" style={{ flex: "1 1 auto", minHeight: 240 }}>
          {lines.map((line, index) => (
            <div
              key={`${line.tsMs}-${index}`}
              style={{
                color:
                  line.level === "error"
                    ? "var(--danger)"
                    : line.level === "warn"
                      ? "var(--warn)"
                      : undefined,
              }}
            >
              {new Date(line.tsMs).toLocaleTimeString()} {line.source}: {line.message}
            </div>
          ))}
        </div>
      ) : null}
      {loading && lines.length === 0 ? (
        <p className="body dim">
          <Spinner /> {t("common.loading")}
        </p>
      ) : null}
    </Screen>
  );
}
