// src/components/DataNoticeScreen.tsx
//
// "Какие данные мы используем" — the declaration guideline 5.4 wants from a
// VPN app, on a screen of its own, before the service is used at all.
//
// Two ways in, one text:
//   • "consent" — the first first-run step (the core puts it before pairing,
//     so no request reaches the service before this screen was seen);
//     "Продолжить" tells the core to remember it (`onboarding_run`).
//   • "readOnly" — from More → О приложении, to read again. Nothing is
//     written: reading it twice is not a second consent.
//
// Every line is something the app or the service actually does, checked
// against the code that does it — the device id (device_id.rs), the headers
// on each request, the resolvers in the engine configs. A sentence this
// screen cannot back up does not go on it: "we keep no logs" waits for
// NO_ACTIVITY_LOGS_CONFIRMED (src/legal.ts).
//
// The resolvers differ by engine, so the DNS line does too: the Apple engine
// (xray_apple.rs) asks Cloudflare over HTTPS through the server, sends every
// name routed direct (Russian ones, the person's "Всегда напрямую") to
// Yandex's public DNS, and looks up the server's own name with the network's
// resolver; the desktop engine asks Cloudflare and Google through the server,
// or whatever the person set in Tunnel settings.

import { useEffect, useRef } from "react";

import type { AppInfo } from "../bridge";
import { IS_APPSTORE } from "../dist";
import type { MsgKey } from "../i18n";
import { NO_ACTIVITY_LOGS_CONFIRMED, privacyUrl, termsUrl } from "../legal";
import { Screen, Spinner, useUi } from "./ui";

type Platform = AppInfo["platform"];

/**
 * Which engine's resolvers to name. Before `app_info` answers, the build
 * decides: every App Store build today is iOS, the direct build is a desktop
 * one (macOS, Windows or Linux), and all three run the same xray config.
 */
function dnsKey(platform: Platform | undefined): MsgKey {
  if (platform === "ios") return "notice.dns.apple";
  if (platform === "macos" || platform === "windows" || platform === "linux") {
    return "notice.dns.desktop";
  }
  return IS_APPSTORE ? "notice.dns.apple" : "notice.dns.desktop";
}

function bulletKeys(platform: Platform | undefined): MsgKey[] {
  const keys: MsgKey[] = [
    "notice.token",
    "notice.installId",
    "notice.version",
    "notice.ip",
    "notice.traffic",
    dnsKey(platform),
  ];
  if (NO_ACTIVITY_LOGS_CONFIRMED) keys.push("notice.noActivityLogs");
  keys.push("notice.log", "notice.promise");
  return keys;
}

// A plain block list: as flex items the bullets lose their markers.
const LIST_STYLE = { margin: "4px 0 0", paddingLeft: 20, listStyleType: "disc" } as const;
const ITEM_STYLE = { marginBottom: 10 } as const;

export default function DataNoticeScreen({
  mode,
  platform,
  stepLabel,
  busy = false,
  onContinue,
  onClose,
}: {
  mode: "consent" | "readOnly";
  platform: Platform | undefined;
  /** "Шаг 1 из 3" when the notice is one of several first-run steps. */
  stepLabel?: string;
  busy?: boolean;
  /** consent: "Продолжить". */
  onContinue?: () => void;
  /** readOnly: back to More. */
  onClose?: () => void;
}) {
  const { t, lang, openExternal } = useUi();
  const heading = useRef<HTMLHeadingElement | null>(null);

  // VoiceOver and the keyboard start at the heading, not at whatever the
  // previous screen had focused: the whole point is that this gets read.
  useEffect(() => {
    heading.current?.focus();
  }, []);

  const consent = mode === "consent";

  return (
    <Screen
      title={consent ? t("app.name") : t("about.title")}
      onClose={consent ? undefined : onClose}
      closeKind="back"
      footer={
        consent ? (
          <button
            type="button"
            className="btn btn-primary"
            onClick={onContinue}
            disabled={busy || !onContinue}
          >
            {busy ? (
              <>
                <Spinner /> {t("notice.continue")}
              </>
            ) : (
              t("notice.continue")
            )}
          </button>
        ) : undefined
      }
    >
      <div className="section">
        {stepLabel ? <span className="caps">{stepLabel}</span> : null}
        <h1 className="h1" ref={heading} tabIndex={-1}>
          {t("notice.title")}
        </h1>
        <p className="body dim">{t("notice.intro")}</p>
        <ul className="body" style={LIST_STYLE}>
          {bulletKeys(platform).map((key) => (
            <li key={key} style={ITEM_STYLE}>
              {t(key)}
            </li>
          ))}
        </ul>
      </div>

      {/* Stacked, not side by side: at twice the text size two half-width
          buttons break their names over three lines each. */}
      <div className="section">
        <button
          type="button"
          className="btn btn-quiet"
          onClick={() => openExternal(privacyUrl(lang))}
        >
          {t("notice.privacy")}
        </button>
        <button
          type="button"
          className="btn btn-quiet"
          onClick={() => openExternal(termsUrl(lang))}
        >
          {t("notice.terms")}
        </button>
      </div>
    </Screen>
  );
}
