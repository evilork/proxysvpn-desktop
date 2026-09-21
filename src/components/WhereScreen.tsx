// src/components/WhereScreen.tsx
//
// [6] "Где вы сейчас?" — the word "routing" replaced by a question a person
// can actually answer. Not one occurrence of "split", "маршруты" or "гео".
//
// This is the one win only our own client can deliver: we build the routing
// ourselves, so a person abroad has nowhere else to turn it off.

import { useCallback, useEffect, useState } from "react";

import { bridge, type RoutingState } from "../bridge";
import { Screen, Spinner, useUi } from "./ui";

export default function WhereScreen({ onClose }: { onClose: () => void }) {
  const { t } = useUi();
  const [routing, setRouting] = useState<RoutingState | null>(null);
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let alive = true;
    void bridge
      .routing()
      .then((value) => alive && setRouting(value))
      .catch(() => alive && setRouting({ inRussia: true, guess: null }));
    return () => {
      alive = false;
    };
  }, []);

  const choose = useCallback(
    async (inRussia: boolean) => {
      if (!routing || routing.inRussia === inRussia) return;
      const previous = routing;
      setRouting({ ...routing, inRussia });
      setBusy(true);
      setFailed(false);
      try {
        await bridge.setRouting(inRussia);
      } catch {
        // Leaving the switch where the person put it while the core kept the
        // old setting would be a lie the app never notices. Roll back loudly.
        setRouting(previous);
        setFailed(true);
      } finally {
        setBusy(false);
      }
    },
    [routing],
  );

  const option = (inRussia: boolean, title: string, body: string) => (
    <button
      type="button"
      className="radio"
      role="radio"
      aria-checked={routing?.inRussia === inRussia}
      disabled={busy || routing === null}
      onClick={() => void choose(inRussia)}
    >
      <span className="radio-dot" aria-hidden="true" />
      <span className="row-main">
        <span className="row-title">{title}</span>
        <span className="row-sub">{body}</span>
      </span>
    </button>
  );

  return (
    <Screen title={t("where.title")} onClose={onClose} closeKind="back">
      <div className="section" role="radiogroup" aria-label={t("where.title")}>
        {option(true, t("where.ru"), t("where.ruBody"))}
        {option(false, t("where.abroad"), t("where.abroadBody"))}
      </div>

      {routing?.guess ? (
        <p className="small faint">
          {routing.guess === "ru" ? t("where.guessRu") : t("where.guessAbroad")}
        </p>
      ) : null}

      {busy ? (
        <p className="small dim">
          <Spinner /> {t("common.loading")}
        </p>
      ) : null}
      {failed ? <p className="body danger">{t("where.failed")}</p> : null}
    </Screen>
  );
}
