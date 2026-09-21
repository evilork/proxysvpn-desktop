// src/components/OnboardingScreen.tsx
//
// [0] The system barriers, one per screen, in the words the system itself
// will use.
//
// Today the password dialog appears BEFORE any window of ours and explains
// nothing, while the README promises the opposite. Here the explanation comes
// first, the step is counted in words, and the screen disappears without
// congratulating anybody once the barrier is passed.
//
// Nothing here is shown if the permissions are already granted: the screen
// does not exist rather than appearing empty.

import { useCallback, useState } from "react";

import { bridge, type OnboardingStep } from "../bridge";
import type { MsgKey } from "../i18n";
import { Screen, Spinner, useUi } from "./ui";

/** macOS: Login Items, where a blocked helper has to be allowed. */
const MACOS_SETTINGS_URL =
  "x-apple.systempreferences:com.apple.LoginItems-Settings.extension";
/** iOS: the VPN section of Settings. */
const IOS_SETTINGS_URL = "App-prefs:General&path=VPN";

const COPY: Record<OnboardingStep, { title: MsgKey; body: MsgKey; action: MsgKey }> = {
  moveToApplications: {
    title: "ob.move.title",
    body: "ob.move.body",
    action: "ob.move.action",
  },
  password: { title: "ob.pass.title", body: "ob.pass.body", action: "ob.pass.action" },
  iosPermission: { title: "ob.ios.title", body: "ob.ios.body", action: "ob.ios.action" },
};

export default function OnboardingScreen({
  steps,
  onDone,
}: {
  steps: OnboardingStep[];
  onDone: () => void;
}) {
  const { t, openExternal } = useUi();
  const [index, setIndex] = useState(0);
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);

  const step = steps[index];

  const advance = useCallback(() => {
    if (index + 1 >= steps.length) onDone();
    else {
      setIndex(index + 1);
      setFailed(false);
    }
  }, [index, steps.length, onDone]);

  const run = useCallback(async () => {
    if (!step) return;
    setBusy(true);
    setFailed(false);
    try {
      await bridge.onboardingRun(step);
      advance();
    } catch {
      setFailed(true);
    } finally {
      setBusy(false);
    }
  }, [advance, step]);

  if (!step) return null;

  const copy = COPY[step];
  const failureKey: MsgKey = step === "moveToApplications" ? "ob.move.failed" : "ob.denied";

  return (
    <Screen
      title={t("app.name")}
      footer={
        <>
          {failed ? (
            <button
              type="button"
              className="btn btn-primary"
              onClick={() =>
                openExternal(step === "iosPermission" ? IOS_SETTINGS_URL : MACOS_SETTINGS_URL)
              }
            >
              {t("ob.showWhere")}
            </button>
          ) : (
            <button
              type="button"
              className="btn btn-primary"
              onClick={() => void run()}
              disabled={busy}
            >
              {busy ? (
                <>
                  <Spinner /> {t("ob.waiting")}
                </>
              ) : (
                t(copy.action)
              )}
            </button>
          )}
          {step === "moveToApplications" || failed ? (
            <button type="button" className="btn btn-quiet" onClick={advance}>
              {step === "moveToApplications" ? t("ob.move.skip") : t("common.retry")}
            </button>
          ) : null}
        </>
      }
    >
      <div className="section">
        <span className="caps">{t("ob.step", { n: index + 1, total: steps.length })}</span>
        <h1 className="h1">{t(copy.title)}</h1>
        <p className="body dim">{t(copy.body)}</p>
        {failed ? <p className="body danger">{t(failureKey)}</p> : null}
      </div>
    </Screen>
  );
}
