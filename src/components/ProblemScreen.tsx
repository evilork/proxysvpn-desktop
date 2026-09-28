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
//
// App Store builds (src/dist.ts) say none of it with money in it: their own
// title and paragraph for the codes that name a top-up, the cabinet or the
// bot, "Проверить снова" where the direct build says "Пополнить", and the
// server's text only when it passes the payment-word filter — a server line
// saying "Пополните баланс" is exactly the purchase prompt guideline 3.1.1
// rules out, and the service can send one after review.

import { IS_APPSTORE } from "../dist";
import { errorBodyKey, errorTitleKey, hasStoreCopy, shownServerText } from "../storeCopy";
import { ERROR_ACTION, type AppError, type ErrorAction, type ErrorCode } from "../types";
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

/** The button's words for this build. */
function actionLabelKey(code: ErrorCode, action: ErrorAction): MsgKey {
  if (IS_APPSTORE) {
    // Adding the link is signing in to the account, never entering a key.
    if (action === "addLink") return "action.appstore.addLink";
    // A remapped "topUp"/"openCabinet": the person fixes it elsewhere, then
    // comes back — "Повторить" would read as "we failed, press again".
    if (action === "retry" && hasStoreCopy(code)) return "action.checkAgain";
  }
  return ACTION_KEY[action];
}

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
  const detail = shownServerText(error.detail, IS_APPSTORE);

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
              t(actionLabelKey(error.code, action))
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
        <h1 className="h1">{t(errorTitleKey(error.code, IS_APPSTORE))}</h1>
        <p className="body dim">{t(errorBodyKey(error.code, IS_APPSTORE))}</p>
        {detail ? (
          <p className="body" style={{ whiteSpace: "pre-wrap" }}>
            {detail}
          </p>
        ) : null}
      </div>
    </Screen>
  );
}
