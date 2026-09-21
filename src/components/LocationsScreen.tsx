// src/components/LocationsScreen.tsx
//
// [5] Countries — the manual choice, for the minority who need a specific one.
//
// Quality is a WORD, never milliseconds. The same number means three
// different things across our platforms: on macOS the ping is measured beside
// the tunnel, on iOS through it, and on Hysteria2 a TCP probe into a UDP port
// never comes up at all. "быстро / средне / медленно / сейчас не проходит /
// проверим при подключении" is true in all three cases. The only number in
// the whole interface is the round trip of the CURRENT connection, measured
// through the tunnel, where it is comparable with itself.
//
// Nothing here carries an address, a port or a protocol — not in the row, not
// in `aria-label`, not in a tooltip. An unnamed entry becomes "Сервер №3",
// never its host name.

import { useCallback, useEffect, useState } from "react";

import { bridge, type LocationEntry, type LocationQuality } from "../bridge";
import { flagFor } from "../flags";
import { formatTime, type MsgKey } from "../i18n";
import { Screen, Spinner, useLatest, useUi } from "./ui";

const QUALITY_KEY: Record<LocationQuality, MsgKey> = {
  good: "loc.q.good",
  ok: "loc.q.ok",
  poor: "loc.q.poor",
  blocked: "loc.q.blocked",
  unknown: "loc.q.unknown",
};

export default function LocationsScreen({
  currentLabel,
  onClose,
  onPicked,
  onEmpty,
}: {
  /** What the tunnel is using right now, for the "Сейчас: …" line. */
  currentLabel?: string;
  onClose: () => void;
  /** null = back to automatic. The label is only used for the toast. */
  onPicked: (label: string | null) => void;
  /** An empty list always means one of the subscription states — show [7]. */
  onEmpty: () => void;
}) {
  const { t, lang } = useUi();
  const [items, setItems] = useState<LocationEntry[] | null>(null);
  const [refreshing, setRefreshing] = useState(true);
  const [staleAt, setStaleAt] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);

  // The list is fetched once per opening, never on a timer: the subscription
  // has a request budget and this screen is not allowed to eat it.
  const emptyRef = useLatest(onEmpty);
  const pickedRef = useLatest(onPicked);

  const load = useCallback(async () => {
    setRefreshing(true);
    try {
      const list = await bridge.locations();
      if (list.length === 0) {
        emptyRef.current();
        return;
      }
      setItems(list);
      setStaleAt(null);
    } catch {
      // An old list beats an empty screen: it is still mostly right, and the
      // person can still connect somewhere.
      setStaleAt(Date.now());
    } finally {
      setRefreshing(false);
    }
  }, [emptyRef]);

  useEffect(() => {
    void load();
  }, [load]);

  const pick = useCallback(
    async (id: string | null, label: string | null) => {
      setBusy(true);
      try {
        await bridge.selectLocation(id);
        pickedRef.current(label);
      } catch {
        // Selection is the one place where failing silently would leave the
        // person looking at a country they are not on.
        setBusy(false);
      }
    },
    [pickedRef],
  );

  const pinned = items?.find((entry) => entry.selected) ?? null;
  const recent = items?.filter((entry) => entry.recent) ?? [];
  const all = items ?? [];

  const renderRow = (entry: LocationEntry, index: number) => (
    <button
      key={entry.id}
      type="button"
      className="row"
      onClick={() => void pick(entry.id, entry.label)}
      disabled={busy}
      data-selected={entry.selected ? "true" : undefined}
      aria-current={entry.selected ? "true" : undefined}
    >
      <span className="row-flag" aria-hidden="true">
        {entry.flag ?? flagFor(entry.label)}
      </span>
      <span className="row-main">
        <span className="row-title">
          {entry.label || t("loc.unnamed", { n: index + 1 })}
        </span>
        <span className="row-sub">{entry.note ?? t(QUALITY_KEY[entry.quality])}</span>
      </span>
      {entry.selected ? <span className="badge">{t("loc.pinned")}</span> : null}
    </button>
  );

  return (
    <Screen
      title={t("loc.title")}
      onClose={onClose}
      footer={<p className="small faint">{t("loc.pinNote")}</p>}
    >
      {refreshing && !items ? (
        <p className="body dim">
          <Spinner /> {t("loc.refreshing")}
        </p>
      ) : null}

      {staleAt !== null ? (
        <div className="servicebar">
          <span>{t("loc.stale", { time: formatTime(lang, staleAt) })}</span>
          <button type="button" onClick={() => void load()}>
            {t("loc.refresh")}
          </button>
        </div>
      ) : null}

      {items ? (
        <>
          <div className="list">
            <button
              type="button"
              className="row"
              onClick={() => void pick(null, null)}
              disabled={busy}
              data-selected={pinned === null ? "true" : undefined}
              aria-current={pinned === null ? "true" : undefined}
            >
              <span className="row-main">
                <span className="row-title">{t("loc.auto")}</span>
                <span className="row-sub">
                  {t("loc.autoHint")}
                  {currentLabel ? ` · ${t("loc.current", { location: currentLabel })}` : ""}
                </span>
              </span>
            </button>
          </div>

          {recent.length > 0 ? (
            <div className="section">
              <span className="caps">{t("loc.recent")}</span>
              <div className="list">{recent.slice(0, 3).map(renderRow)}</div>
            </div>
          ) : null}

          <div className="section">
            <span className="caps">{t("loc.all")}</span>
            <div className="list">{all.map(renderRow)}</div>
          </div>
        </>
      ) : null}
    </Screen>
  );
}
