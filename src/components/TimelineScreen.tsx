// src/components/TimelineScreen.tsx
//
// [11] A feed of things that happened, instead of the orange wall of raw
// engine output the (i) button used to open.
//
// Only product events land here — an ordinary connection is not an event.
// Nothing in this list can contain an address, a port or a UUID, because the
// core sends a CODE and this file turns it into a sentence.

import { useCallback, useEffect, useState } from "react";

import { bridge, type TimelineEntry } from "../bridge";
import { formatTime, type MsgKey, type TParams } from "../i18n";
import { Screen, Spinner, useUi } from "./ui";

const LIMIT = 500;

const EVENT_KEY: Record<TimelineEntry["code"], MsgKey> = {
  connected: "tl.ev.connected",
  disconnected: "tl.ev.disconnected",
  healed: "tl.ev.healed",
  trafficStopped: "tl.ev.trafficStopped",
  networkChanged: "tl.ev.networkChanged",
  subRefreshed: "tl.ev.subRefreshed",
  subRefreshedFallback: "tl.ev.subRefreshedFallback",
  locationSwitched: "tl.ev.locationSwitched",
  locationUnreadable: "tl.ev.locationUnreadable",
  failed: "tl.ev.failed",
};

/** Midnight-based day index, so "yesterday" survives a clock near midnight. */
function dayIndex(ms: number): number {
  const date = new Date(ms);
  date.setHours(0, 0, 0, 0);
  return Math.round(date.getTime() / 86_400_000);
}

export default function TimelineScreen({
  onClose,
  onReport,
  onTechLog,
}: {
  onClose: () => void;
  onReport: () => void;
  onTechLog: () => void;
}) {
  const { t, lang } = useUi();
  const [entries, setEntries] = useState<TimelineEntry[] | null>(null);

  useEffect(() => {
    let alive = true;
    void bridge
      .timeline(LIMIT)
      .then((list) => alive && setEntries(list))
      .catch(() => alive && setEntries([]));
    return () => {
      alive = false;
    };
  }, []);

  const groupTitle = useCallback(
    (index: number, today: number): string => {
      if (index === today) return t("tl.today");
      if (index === today - 1) return t("tl.yesterday");
      return t("tl.earlier");
    },
    [t],
  );

  const today = dayIndex(Date.now());
  const groups = new Map<string, TimelineEntry[]>();
  for (const entry of entries ?? []) {
    const title = groupTitle(dayIndex(entry.atMs), today);
    const bucket = groups.get(title);
    if (bucket) bucket.push(entry);
    else groups.set(title, [entry]);
  }

  const line = (entry: TimelineEntry): string => {
    const params: TParams = {};
    if (entry.location) params.location = entry.location;
    // The core tells us the network changed, not what it changed TO — so the
    // sentence naming the network is only used when that word actually
    // arrived. Picking the key here rather than letting the placeholder fall
    // through is what keeps a literal "{network}" off the screen.
    let key = EVENT_KEY[entry.code];
    if (entry.network) {
      params.network = t(entry.network === "wifi" ? "tl.net.wifi" : "tl.net.mobile");
      if (entry.code === "networkChanged") key = "tl.ev.networkChangedTo";
    }
    return t(key, params);
  };

  return (
    <Screen
      title={t("tl.title")}
      onClose={onClose}
      closeKind="back"
      footer={
        <>
          <button type="button" className="btn btn-quiet" onClick={onReport}>
            {t("tl.report")}
          </button>
          {/* Two taps away from the main screen, exactly as far as it should be. */}
          <button type="button" className="btn btn-quiet" onClick={onTechLog}>
            {t("tl.techLog")}
          </button>
        </>
      }
    >
      {entries === null ? (
        <p className="body dim">
          <Spinner /> {t("common.loading")}
        </p>
      ) : entries.length === 0 ? (
        <p className="body dim">{t("tl.empty")}</p>
      ) : (
        [...groups.entries()].map(([title, list]) => (
          <div className="section" key={title}>
            <span className="caps">{title}</span>
            <div className="list">
              {list.map((entry) => (
                <div className="row" key={`${entry.atMs}-${entry.code}`}>
                  <span className="row-side nowrap-num" style={{ minWidth: 44 }}>
                    {formatTime(lang, entry.atMs)}
                  </span>
                  <span className="row-main">
                    <span className="row-title" style={{ fontWeight: 500 }}>
                      {line(entry)}
                    </span>
                  </span>
                </div>
              ))}
            </div>
          </div>
        ))
      )}
    </Screen>
  );
}
