// src/components/SubscriptionScreen.tsx
//
// [10] Money and time shown as a STATE of the service, plus the one rule a
// person comes here looking for: one link, one device.
//
// What is not here, on purpose:
//   • no "Delete device" button — the most expensive action in the system
//     (PRO is 135–169 ₽, no refund) and the most common reflex when something
//     breaks. It lives in the bot and the cabinet, behind a session.
//   • no PRO banner and no filled "buy" button. The only line about money is
//     a quiet one, last.
//   • no rouble balance. The balance comes from `/api/balance`, which needs a
//     cabinet session the app does not have, and copying the money formula
//     into the client has already cost people half of a paid period once.
//     A line we cannot source is a line we do not print.
//
// The profile name comes from the `profile-title` header, where the service
// deliberately puts the DOMAIN rather than a brand: weeks later that is how a
// person works out where to renew.
//
// App Store builds (src/dist.ts) show the state and nothing about paying for
// more: no "Пополнить на …" line, no cabinet or bot buttons (the app ships
// without in-app purchases, guideline 3.1.1), "Доступ до" instead of
// "оплачен до", and the service's title and info text only when they pass the
// payment-word filter.

import { IS_APPSTORE } from "../dist";
import { daysLeft, formatDate, formatTime } from "../i18n";
import { shownServerText } from "../storeCopy";
import type { SubMeta } from "../types";
import type { AppInfo, SubState } from "../bridge";
import { Screen, useUi } from "./ui";

export default function SubscriptionScreen({
  meta,
  info,
  sub,
  onClose,
  onOpenCabinet,
  onOpenBot,
}: {
  meta: SubMeta;
  info: AppInfo | null;
  sub: SubState | null;
  onClose: () => void;
  onOpenCabinet: () => void;
  onOpenBot: () => void;
}) {
  const { t, lang } = useUi();

  const expiresMs = meta.expiresAt ? meta.expiresAt * 1000 : null;
  const host = info ? info.cabinetUrl.replace(/^https?:\/\//, "") : "proxysvpn.com";
  // `??` as before: a direct build still prints an empty title as empty.
  const title = IS_APPSTORE ? (shownServerText(meta.title, true) ?? host) : (meta.title ?? host);
  const infoText = shownServerText(meta.infoText, IS_APPSTORE);

  return (
    <Screen
      title={t("sub.title")}
      onClose={onClose}
      closeKind="back"
      footer={
        IS_APPSTORE ? null : (
          <>
            <button type="button" className="btn btn-outline" onClick={onOpenCabinet}>
              {t("sub.openCabinet")}
            </button>
            <button type="button" className="btn btn-quiet" onClick={onOpenBot}>
              {t("sub.orBot")}
            </button>
          </>
        )
      }
    >
      <div className="section">
        <h2 className="h2">{title}</h2>
        {expiresMs ? (
          <p className="body">
            {t(IS_APPSTORE ? "sub.appstore.accessUntil" : "sub.paidUntil", {
              date: formatDate(lang, expiresMs),
              days: t("time.days", { n: Math.max(0, daysLeft(expiresMs, Date.now())) }),
            })}
          </p>
        ) : (
          <p className="body dim">{t("details.expiryUnknown")}</p>
        )}
        {infoText ? <p className="body dim">{infoText}</p> : null}
        {/* "as of 14:20" instead of a zero: a zero reads as "no money left". */}
        {sub?.lastUpdatedAt ? (
          <p className="small faint">
            {t("sub.asOf", { time: formatTime(lang, sub.lastUpdatedAt) })}
          </p>
        ) : null}
      </div>

      <hr className="divider" />

      <div className="section">
        <span className="caps">{t("sub.device")}</span>
        <p className="body">
          {info?.deviceLinkedAt
            ? t("sub.deviceLine", {
                name: info.deviceName,
                date: formatDate(lang, info.deviceLinkedAt),
              })
            : (info?.deviceName ?? "—")}
        </p>
        <p className="small dim">{t(IS_APPSTORE ? "sub.appstore.oneLink" : "sub.oneLink")}</p>
      </div>

      {IS_APPSTORE ? null : <p className="small faint">{t("sub.topUp", { host })}</p>}
    </Screen>
  );
}
