// src/components/HealingSheet.tsx
//
// [4] Recovery, shown only once it has dragged on past eight seconds.
//
// Why it exists: the most expensive action in the whole system is deleting a
// device (PRO is 135–169 ₽, no refund), and deleting the device is the first
// reflex when something breaks. A person who can see that the app is working
// through a list does not go looking for that button.
//
// Which rungs are shown is derived from `healingForMs` and the budgets in М3,
// not from a separate event stream. The core walks the same ladder on the
// same clock, so the two cannot disagree, and events.rs did not have to grow
// a channel for it.
//
// Finished rungs get a tick, the current one gets a spinner, FUTURE RUNGS ARE
// NOT SHOWN — a visible list of everything still to try reads as "it is all
// broken". There is no seconds counter either: a ticking number is pressure.

import { useEffect, useState } from "react";

import type { MsgKey } from "../i18n";
import type { StatePayload } from "../types";
import { Sheet, Spinner, useLatest, useUi } from "./ui";

/** [starts at ms, line]. Mirrors the ladder in DESIGN.md М3. */
const RUNGS: [number, MsgKey][] = [
  [0, "heal.step.checkNode"],
  [3_000, "heal.step.restartEngine"],
  [8_000, "heal.step.otherNode"],
  [18_000, "heal.step.otherProto"],
  [28_000, "heal.step.refreshSub"],
  [40_000, "heal.step.askService"],
];

const OUTCOME_PAUSE_MS = 1500;

export default function HealingSheet({
  state,
  onClose,
  onFailed,
  onCancel,
}: {
  state: StatePayload;
  /** Recovered: the sheet says so and leaves. */
  onClose: () => void;
  /** The ladder ran out: [7] takes over. */
  onFailed: () => void;
  onCancel: () => void;
}) {
  const { t } = useUi();
  const [outcome, setOutcome] = useState<"ok" | "fail" | null>(null);
  const closeRef = useLatest(onClose);
  const failedRef = useLatest(onFailed);

  const healing = state.phase === "healing";
  const elapsed = state.healingForMs ?? 0;

  useEffect(() => {
    if (state.phase === "healing") return;
    // The phase moved on while the sheet was open. Say what happened, then
    // get out of the way — silently vanishing looks like a glitch.
    const recovered = state.phase === "on" || state.phase === "unconfirmed";
    setOutcome(recovered ? "ok" : "fail");
    const id = window.setTimeout(
      () => (recovered ? closeRef.current() : failedRef.current()),
      OUTCOME_PAUSE_MS,
    );
    return () => window.clearTimeout(id);
  }, [state.phase, closeRef, failedRef]);

  const reached = RUNGS.filter(([at]) => elapsed >= at);
  const visible = reached.length > 0 ? reached : [RUNGS[0]];

  return (
    <Sheet title={t("heal.title")} onClose={onCancel}>
      <div className="sheet-body">
        <h2 className="h2">{t("heal.title")}</h2>
        <p className="body dim">{t("heal.body")}</p>

        <div style={{ margin: "10px 0 6px" }}>
          {visible.map(([at, key], index) => {
            const isCurrent = healing && index === visible.length - 1;
            return (
              <div className="heal-step" key={at} data-current={isCurrent ? "true" : "false"}>
                <span className="mark" aria-hidden="true">
                  {isCurrent ? <Spinner /> : "✓"}
                </span>
                <span>{t(key)}</span>
              </div>
            );
          })}
        </div>

        {outcome === "ok" ? <p className="body accent">{t("heal.ok")}</p> : null}
        {outcome === "fail" ? <p className="body">{t("heal.fail")}</p> : null}

        {outcome === null ? (
          <button type="button" className="btn btn-quiet" onClick={onCancel}>
            {t("heal.cancel")}
          </button>
        ) : null}
      </div>
    </Sheet>
  );
}
