// src/components/ProblemScreen.tsx
//
// [7] One reason, one action.
//
// This screen replaces raw Rust strings like
// "tun start: dns lookup failed for <адрес узла>" — which, on top of being
// unreadable, printed a node address on the screen. Here there are no codes,
// no paths and no addresses: a title, a paragraph, exactly ONE filled button,
// and two quiet lines under it.
//
// Where the wording comes from: our own sentence from i18n, plus — when the
// server wrote one — its own text, verbatim, underneath. The server's notices
// are already proof-read against real tickets and already translated by
// `?lang=`, and the one for a taken link even carries the cure inside it.
//
// Exactly one action is not a stylistic choice. A screen with five
// suggestions reads as "everything is broken" and sends the person to support
// just as reliably as saying nothing.

import { ERROR_ACTION, type AppError, type ErrorAction } from "../types";
import type { MsgKey } from "../i18n";
import { Screen, Spinner, useUi } from "./ui";

const ACTION_KEY: Record<ErrorAction, MsgKey> = {
  addLink: "action.addLink",
  retry: "action.retry",
  openCabinet: "action.openCabinet",
  topUp: "action.topUp",
  diagnose: "action.diagnose",
  waitAndSee: "action.waitAndSee",
  contactSupport: "action.contactSupport",
};

export default function ProblemScreen({
  error,
  busy,
  onAction,
  onClose,
  onCheck,
  onTimeline,
}: {
  error: AppError;
  /** The single button turns into an indicator; the screen does not change. */
  busy: boolean;
  onAction: (action: ErrorAction) => void;
  onClose: () => void;
  onCheck: () => void;
  onTimeline: () => void;
}) {
  const { t } = useUi();
  const action = ERROR_ACTION[error.code];

  return (
    <Screen
      title={t("problem.title")}
      onClose={onClose}
      centered
      footer={
        <>
          <button
            type="button"
            className="btn btn-primary"
            onClick={() => onAction(action)}
            disabled={busy}
          >
            {busy ? (
              <>
                <Spinner /> {t("problem.working")}
              </>
            ) : (
              t(ACTION_KEY[action])
            )}
          </button>
          <div className="btn-row">
            <button type="button" className="btn btn-quiet" onClick={onCheck}>
              {t("problem.check")}
            </button>
            <button type="button" className="btn btn-quiet" onClick={onTimeline}>
              {t("problem.timeline")}
            </button>
          </div>
        </>
      }
    >
      <div className="section">
        <h1 className="h1">{t(`err.${error.code}.title`)}</h1>
        <p className="body dim">{t(`err.${error.code}.body`)}</p>
        {error.detail ? (
          <p className="body" style={{ whiteSpace: "pre-wrap" }}>
            {error.detail}
          </p>
        ) : null}
      </div>
    </Screen>
  );
}
