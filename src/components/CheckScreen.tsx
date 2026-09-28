// src/components/CheckScreen.tsx
//
// [8] What we can see from the device, and what only the service can see.
//
// The device half works WITHOUT the internet and without our servers, on
// purpose: `proxysvpn.com` is on the direct list, so our own API is
// unreachable exactly when the person needs this screen most.
//
// The service half needs a backend change that does not exist yet
// (`/api/vpn/check` authorises a cabinet session or an internal key, and the
// app only holds a subscription token). Until it does, the column honestly
// collapses into one line pointing at the cabinet. Drawing a column that
// cannot work is the one thing this screen must not do.
//
// The two groups are stacked rather than set side by side: the window is
// 480 pt wide, and two columns at that width turn every row into two lines.
//
// App Store builds do not open the cabinet (it sells top-ups, and the app
// ships without in-app purchases), so there the line says the check is not
// available yet and no button points anywhere.

import { useCallback, useState } from "react";

import { bridge, type CheckReport, type CheckRow, type CheckRowId } from "../bridge";
import { IS_APPSTORE } from "../dist";
import type { MsgKey } from "../i18n";
import { Screen, Spinner, useUi } from "./ui";

const DEVICE_ROWS: CheckRowId[] = [
  "clock",
  "internet",
  "dns",
  "sub",
  "handshake",
  "traffic",
  "network",
];

const SERVICE_ROWS: CheckRowId[] = ["balance", "profile", "freshness", "nodes"];

const ROW_KEY: Record<CheckRowId, MsgKey> = {
  clock: "check.row.clock",
  internet: "check.row.internet",
  dns: "check.row.dns",
  sub: "check.row.sub",
  handshake: "check.row.handshake",
  traffic: "check.row.traffic",
  network: "check.row.network",
  balance: "check.row.balance",
  profile: "check.row.profile",
  freshness: "check.row.freshness",
  nodes: "check.row.nodes",
};

const VERDICT_KEY: Record<CheckRowId, MsgKey> = {
  clock: "check.verdict.clock",
  internet: "check.verdict.internet",
  dns: "check.verdict.dns",
  sub: "check.verdict.sub",
  handshake: "check.verdict.handshake",
  traffic: "check.verdict.traffic",
  network: "check.verdict.network",
  balance: "check.verdict.balance",
  profile: "check.verdict.profile",
  freshness: "check.verdict.freshness",
  nodes: "check.verdict.nodes",
};

type Stage = "idle" | "running" | "fixing" | "done";

export default function CheckScreen({
  onClose,
  onReport,
  onOpenCabinet,
}: {
  onClose: () => void;
  onReport: () => void;
  onOpenCabinet: () => void;
}) {
  const { t } = useUi();
  const [stage, setStage] = useState<Stage>("idle");
  const [report, setReport] = useState<CheckReport | null>(null);
  const [serviceFailed, setServiceFailed] = useState(false);

  const run = useCallback(async () => {
    setStage("running");
    setServiceFailed(false);
    try {
      setReport(await bridge.runCheck());
    } catch {
      // The local half is what the verdict is built from; when even that
      // cannot run we say so rather than inventing a green screen.
      setServiceFailed(true);
      setReport(null);
    } finally {
      setStage("done");
    }
  }, []);

  const fix = useCallback(async () => {
    setStage("fixing");
    try {
      setReport(await bridge.runFix());
    } catch {
      setServiceFailed(true);
    } finally {
      setStage("done");
    }
  }, []);

  const rowState = (id: CheckRowId, rows: CheckRow[] | null): CheckRow => {
    const found = rows?.find((row) => row.id === id);
    if (found) return found;
    return { id, state: stage === "running" || stage === "fixing" ? "pending" : "idle" };
  };

  const renderRow = (id: CheckRowId, rows: CheckRow[] | null) => {
    const row = rowState(id, rows);
    const mark =
      row.state === "pass" ? "✓" : row.state === "fail" ? "✕" : row.state === "pending" ? "·" : "·";
    return (
      <div className="check-row" key={id} data-state={row.state}>
        <span className="mark" aria-hidden="true">
          {mark}
        </span>
        <span>{t(ROW_KEY[id])}</span>
        {row.viaFallback ? <span className="badge">{t("check.note.fallback")}</span> : null}
        {row.state === "idle" ? <span className="small faint">{t("check.notChecked")}</span> : null}
      </div>
    );
  };

  const service = report?.service;
  const canFix = service?.available === true && service.canFix;
  const verdictId = report?.verdict ?? null;
  const allGood = stage === "done" && report !== null && verdictId === null;

  return (
    <Screen
      title={t("check.title")}
      onClose={onClose}
      footer={
        <>
          {canFix ? (
            <button
              type="button"
              className="btn btn-primary"
              onClick={() => void fix()}
              disabled={stage === "fixing"}
            >
              {stage === "fixing" ? (
                <>
                  <Spinner /> {t("check.fixing")}
                </>
              ) : (
                t("check.fix")
              )}
            </button>
          ) : (
            <button
              type="button"
              className="btn btn-primary"
              onClick={() => void run()}
              disabled={stage === "running" || stage === "fixing"}
            >
              {stage === "running" ? (
                <>
                  <Spinner /> {t("check.running")}
                </>
              ) : stage === "idle" ? (
                t("check.run")
              ) : (
                t("check.again")
              )}
            </button>
          )}
          <button type="button" className="btn btn-quiet" onClick={onReport}>
            {t("check.report")}
          </button>
        </>
      }
    >
      <div className="section">
        <span className="caps">{t("check.device")}</span>
        <div className="check-grid">{DEVICE_ROWS.map((id) => renderRow(id, report?.device ?? null))}</div>
      </div>

      <div className="section">
        <span className="caps">{t("check.service")}</span>
        {service?.available ? (
          <div className="check-grid">{SERVICE_ROWS.map((id) => renderRow(id, service.rows))}</div>
        ) : (
          <div className="check-grid">
            <div className="check-row" data-state="idle">
              <span className="mark" aria-hidden="true">
                ·
              </span>
              <span>
                {serviceFailed || service?.reason === "failed"
                  ? t("check.serviceFailed")
                  : IS_APPSTORE
                    ? t("check.appstore.serviceUnavailable")
                    : t("check.serviceInCabinet")}
              </span>
            </div>
            {IS_APPSTORE ? null : (
              <button type="button" className="btn btn-quiet" onClick={onOpenCabinet}>
                {t("check.openCabinet")}
              </button>
            )}
          </div>
        )}
      </div>

      {stage === "done" ? (
        <div className="verdict" data-bad={verdictId !== null ? "true" : "false"}>
          {allGood ? (
            <>
              <p className="body">{t("check.allGood")}</p>
              <p className="small dim" style={{ marginTop: 6 }}>
                {t("check.allGoodNote")}
              </p>
            </>
          ) : verdictId !== null ? (
            <p className="body">{t("check.verdict", { text: t(VERDICT_KEY[verdictId]) })}</p>
          ) : (
            <p className="body">{t("check.serviceFailed")}</p>
          )}
        </div>
      ) : null}
    </Screen>
  );
}
