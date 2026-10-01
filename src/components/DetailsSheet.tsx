// src/components/DetailsSheet.tsx
//
// [3] Everything taken off the main screen, one tap away and with no nesting:
// location, where you are, money, the traffic fork, a check, the subscription.
//
// "Куда идёт трафик" is two numbers and one sentence of explanation. That is
// the only honest form of a transparency promise: counts without addresses.
// The sentence speaks for this device only; "we do not record sites" would
// speak for the servers too, and waits for NO_ACTIVITY_LOGS_CONFIRMED.

import { useCallback, useEffect, useState } from "react";

import { bridge, type RoutingState, type TrafficSplit } from "../bridge";
import { IS_APPSTORE } from "../dist";
import { daysLeft, formatDate, formatNumber } from "../i18n";
import { NO_ACTIVITY_LOGS_CONFIRMED } from "../legal";
import type { StatePayload, SubMeta } from "../types";
import { IconChevron, Sheet, Spinner, useUi } from "./ui";

const OK_LINE_MS = 2000;

export default function DetailsSheet({
  state,
  meta,
  onClose,
  onLocations,
  onWhere,
  onSubscription,
  onCheck,
}: {
  state: StatePayload;
  meta: SubMeta;
  onClose: () => void;
  onLocations: () => void;
  onWhere: () => void;
  onSubscription: () => void;
  /** Opens [8] when the quick check found something to show. */
  onCheck: () => void;
}) {
  const { t, lang } = useUi();
  const [routing, setRouting] = useState<RoutingState | null>(null);
  const [split, setSplit] = useState<TrafficSplit | null>(null);
  const [checking, setChecking] = useState(false);
  const [okLine, setOkLine] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    // Each row loads on its own: a slow one must not hold the others back,
    // and every row stays tappable while it loads.
    void bridge
      .routing()
      .then((value) => alive && setRouting(value))
      .catch(() => undefined);
    void bridge
      .trafficSplit()
      .then((value) => alive && setSplit(value))
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, []);

  const runCheck = useCallback(async () => {
    setChecking(true);
    setOkLine(null);
    try {
      const report = await bridge.runCheck();
      if (report.verdict === null) {
        setOkLine(
          t("details.checkOk", { location: state.location ?? t("details.locationAuto") }),
        );
        window.setTimeout(() => setOkLine(null), OK_LINE_MS);
      } else {
        onCheck();
      }
    } catch {
      // The check could not even run — that is itself a finding, and [8] is
      // where findings are explained.
      onCheck();
    } finally {
      setChecking(false);
    }
  }, [onCheck, state.location, t]);

  const expiryLine = (): string => {
    if (!meta.expiresAt) return t("details.expiryUnknown");
    const ms = meta.expiresAt * 1000;
    const left = daysLeft(ms, Date.now());
    // App Store builds state when access ends, not that it was paid for.
    return t(IS_APPSTORE ? "details.appstore.accessUntil" : "details.paidUntil", {
      date: formatDate(lang, ms),
      days: t("time.days", { n: Math.max(0, left) }),
    });
  };

  return (
    <Sheet title={t("details.title")} onClose={onClose}>
      <div className="sheet-body">
        <button type="button" className="row" onClick={onLocations}>
          <span className="row-main">
            <span className="row-title">
              {state.location ?? t("details.locationAuto")}
            </span>
          </span>
          <span className="row-side">
            {t("details.change")}
            <IconChevron />
          </span>
        </button>

        <button type="button" className="row" onClick={onWhere}>
          <span className="row-main">
            <span className="row-title" style={{ fontWeight: 500 }}>
              {routing === null
                ? t("common.loading")
                : routing.inRussia
                  ? t("details.whereRu")
                  : t("details.whereAbroad")}
            </span>
          </span>
          <span className="row-side">
            {t("details.edit")}
            <IconChevron />
          </span>
        </button>

        <div className="row">
          <span className="row-main">
            <span className="row-title" style={{ fontWeight: 500 }}>
              {expiryLine()}
            </span>
          </span>
        </div>

        {split ? (
          <div className="row">
            <span className="row-main">
              <span className="row-title" style={{ fontWeight: 500 }}>
                {t("details.split", {
                  via: formatNumber(lang, split.viaVpn),
                  direct: formatNumber(lang, split.direct),
                })}
              </span>
              <span className="row-sub">
                {t(NO_ACTIVITY_LOGS_CONFIRMED ? "details.splitNote.noLogs" : "details.splitNote")}
              </span>
            </span>
          </div>
        ) : null}

        <button type="button" className="row" onClick={() => void runCheck()} disabled={checking}>
          <span className="row-main">
            <span className="row-title">{t("details.checkNow")}</span>
            {okLine ? <span className="row-sub accent">{okLine}</span> : null}
          </span>
          <span className="row-side">{checking ? <Spinner /> : null}</span>
        </button>

        <button type="button" className="row" onClick={onSubscription}>
          <span className="row-main">
            <span className="row-title">{t("details.sub")}</span>
          </span>
          <span className="row-side">
            <IconChevron />
          </span>
        </button>
      </div>
    </Sheet>
  );
}
