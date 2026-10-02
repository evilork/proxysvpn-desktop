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
//
// The data notice (guideline 5.4) is a step too, and the core always puts it
// first: it is what a person reads before the app talks to the service at
// all. It has its own screen (DataNoticeScreen) because it is a page to read,
// not a barrier with one button.
//
// Where a barrier was not passed, the app says where to fix it in words.
// iOS gets no link: the only address that opens the VPN page of Settings is a
// private URL scheme, which guideline 2.5.1 rejects. The macOS Login Items
// link stays in the direct build and is never rendered in an App Store build.
//
// Gaming Mode on SteamOS is a question, not a barrier: two answers of equal
// standing, and the core grants the passwordless start only for "allow"
// (pvpn-platform helper/install.rs). It is said before the system's password
// window, which can only quote a shortened command line. App.tsx asks the
// core for the steps again before every connect of the AppImage, so after
// "Remove system files" the question comes back before the next setup.

import { useCallback, useState } from "react";

import { bridge, type AppInfo, type GamingModeChoice, type OnboardingStep } from "../bridge";
import { IS_APPSTORE } from "../dist";
import type { MsgKey } from "../i18n";
import DataNoticeScreen from "./DataNoticeScreen";
import { Screen, Spinner, useUi } from "./ui";

/** macOS: Login Items, where a blocked helper has to be allowed. */
const MACOS_SETTINGS_URL =
  "x-apple.systempreferences:com.apple.LoginItems-Settings.extension";

type BarrierStep = Exclude<OnboardingStep, "dataNotice" | "gamingMode">;

const COPY: Record<BarrierStep, { title: MsgKey; body: MsgKey; action: MsgKey }> = {
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
  platform,
  onDone,
}: {
  steps: OnboardingStep[];
  /** Which engine's resolvers the data notice names; undefined until known. */
  platform: AppInfo["platform"] | undefined;
  onDone: () => void;
}) {
  const { t, openExternal } = useUi();
  const [index, setIndex] = useState(0);
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const [answering, setAnswering] = useState<GamingModeChoice | null>(null);

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

  // Gaming Mode: the answer is the step. Nothing moves on without one.
  const answerGamingMode = useCallback(
    async (choice: GamingModeChoice) => {
      setAnswering(choice);
      setFailed(false);
      try {
        await bridge.onboardingRun("gamingMode", choice);
        advance();
      } catch {
        setFailed(true);
      } finally {
        setAnswering(null);
      }
    },
    [advance],
  );

  // "Продолжить" on the notice moves on even when the core could not store
  // the answer: the person has read it, which is what 5.4 asks for, and an
  // unwritable disk must not lock them out of the app. The only cost is the
  // notice showing again on the next launch.
  const acceptNotice = useCallback(async () => {
    setBusy(true);
    try {
      await bridge.onboardingRun("dataNotice");
    } catch {
      console.warn({ event: "data_notice_not_recorded" });
    } finally {
      setBusy(false);
    }
    advance();
  }, [advance]);

  if (!step) return null;

  const stepLabel = t("ob.step", { n: index + 1, total: steps.length });

  if (step === "dataNotice") {
    return (
      <DataNoticeScreen
        mode="consent"
        platform={platform}
        stepLabel={steps.length > 1 ? stepLabel : undefined}
        busy={busy}
        onContinue={() => void acceptNotice()}
      />
    );
  }

  if (step === "gamingMode") {
    return (
      <Screen
        title={t("app.name")}
        footer={
          <>
            <button
              type="button"
              className="btn btn-primary"
              onClick={() => void answerGamingMode("allow")}
              disabled={answering !== null}
            >
              {answering === "allow" ? <Spinner /> : t("ob.gaming.allow")}
            </button>
            <button
              type="button"
              className="btn btn-outline"
              onClick={() => void answerGamingMode("decline")}
              disabled={answering !== null}
            >
              {answering === "decline" ? <Spinner /> : t("ob.gaming.decline")}
            </button>
          </>
        }
      >
        <div className="section">
          <span className="caps">{stepLabel}</span>
          <h1 className="h1">{t("ob.gaming.title")}</h1>
          <p className="body dim">{t("ob.gaming.body")}</p>
          <p className="body dim">{t("ob.gaming.declineBody")}</p>
          {failed ? <p className="body danger">{t("ob.gaming.failed")}</p> : null}
        </div>
      </Screen>
    );
  }

  const copy = COPY[step];
  const failureKey: MsgKey = step === "moveToApplications" ? "ob.move.failed" : "ob.denied";
  // The Login Items link: direct macOS builds only, never on iOS (no public
  // address for that page) and never in an App Store build.
  const settingsLink = failed && !IS_APPSTORE && step !== "iosPermission";

  return (
    <Screen
      title={t("app.name")}
      footer={
        <>
          {settingsLink ? (
            <button
              type="button"
              className="btn btn-primary"
              onClick={() => openExternal(MACOS_SETTINGS_URL)}
            >
              {t("ob.showWhere")}
            </button>
          ) : failed ? (
            <button type="button" className="btn btn-primary" onClick={advance}>
              {t("common.retry")}
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
          {step === "moveToApplications" || settingsLink ? (
            <button type="button" className="btn btn-quiet" onClick={advance}>
              {step === "moveToApplications" ? t("ob.move.skip") : t("common.retry")}
            </button>
          ) : null}
        </>
      }
    >
      <div className="section">
        <span className="caps">{stepLabel}</span>
        <h1 className="h1">{t(copy.title)}</h1>
        <p className="body dim">{t(copy.body)}</p>
        {failed ? <p className="body danger">{t(failureKey)}</p> : null}
        {failed && step === "iosPermission" ? (
          <p className="body">{t("ob.ios.where")}</p>
        ) : null}
      </div>
    </Screen>
  );
}
