// src/components/MoreScreen.tsx
//
// [12] Seven rows, nothing nested deeper than one level, and not a single
// entry that says "soon".
//
// "Обновить настройки подписки" is named as an ACTION, not as a gesture:
// there is no pull-to-refresh in any of our clients, and the wrong wording
// for it has already reached four places in the support rules.
//
// "Включаться автоматически" is absent rather than greyed out. On macOS we
// cannot do it without a privileged helper, and a disabled row is a promise
// with the sound turned off.

import { useCallback, useEffect, useState } from "react";

import { bridge, type AppInfo, type RoutingState, type SubState } from "../bridge";
import { LANGS, LANG_NAME, formatTime, type Lang } from "../i18n";
import { IconChevron, Screen, Spinner, useUi, type ThemePref } from "./ui";

const THEME_OPTIONS: [ThemePref, "more.theme.system" | "more.theme.light" | "more.theme.dark"][] = [
  ["system", "more.theme.system"],
  ["light", "more.theme.light"],
  ["dark", "more.theme.dark"],
];

export default function MoreScreen({
  info,
  sub,
  theme,
  onTheme,
  onLang,
  onClose,
  onSubscription,
  onWhere,
  onTimeline,
  onUnlinked,
  onOpenCabinet,
}: {
  info: AppInfo | null;
  sub: SubState | null;
  theme: ThemePref;
  onTheme: (value: ThemePref) => void;
  onLang: (value: Lang) => void;
  onClose: () => void;
  onSubscription: () => void;
  onWhere: () => void;
  onTimeline: () => void;
  /** The link was removed from this device; the app returns to [1]. */
  onUnlinked: () => void;
  onOpenCabinet: () => void;
}) {
  const { t, lang, toast } = useUi();
  const [routing, setRouting] = useState<RoutingState | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [updatedAt, setUpdatedAt] = useState<number | undefined>(sub?.lastUpdatedAt);
  const [confirmUnlink, setConfirmUnlink] = useState(false);

  useEffect(() => {
    let alive = true;
    void bridge
      .routing()
      .then((value) => alive && setRouting(value))
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, []);

  const refresh = useCallback(async () => {
    setRefreshing(true);
    try {
      await bridge.refreshSub();
      const next = await bridge.subState();
      setUpdatedAt(next.lastUpdatedAt);
    } catch {
      toast(t("more.refreshFailed"));
    } finally {
      setRefreshing(false);
    }
  }, [t, toast]);

  const unlink = useCallback(async () => {
    try {
      await bridge.clearSubscription();
      onUnlinked();
    } catch {
      toast(t("more.refreshFailed"));
    }
  }, [onUnlinked, t, toast]);

  const whereValue = routing
    ? routing.inRussia
      ? t("where.short.ru")
      : t("where.short.abroad")
    : "…";

  return (
    <Screen
      title={t("more.title")}
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn btn-quiet" onClick={onOpenCabinet}>
            {t("app.name")} {info?.version ?? ""} · {t("more.cabinet")}
          </button>
          <p className="small faint" style={{ textAlign: "center" }}>
            {t("more.geoNote")}
          </p>
        </>
      }
    >
      <div className="list">
        <button type="button" className="row" onClick={onSubscription}>
          <span className="row-main">
            <span className="row-title">{t("sub.title")}</span>
          </span>
          <span className="row-side">
            <IconChevron />
          </span>
        </button>

        <button type="button" className="row" onClick={onWhere}>
          <span className="row-main">
            <span className="row-title">{t("more.where", { value: whereValue })}</span>
          </span>
          <span className="row-side">
            <IconChevron />
          </span>
        </button>

        <button
          type="button"
          className="row"
          onClick={() => void refresh()}
          disabled={refreshing}
        >
          <span className="row-main">
            <span className="row-title">{t("more.refresh")}</span>
            <span className="row-sub">
              {updatedAt
                ? t("more.updatedAt", { time: formatTime(lang, updatedAt) })
                : t("more.updatedNever")}
            </span>
          </span>
          <span className="row-side">{refreshing ? <Spinner /> : null}</span>
        </button>

        <div className="row">
          <span className="row-main">
            <span className="row-title">{t("more.lang")}</span>
          </span>
          <span className="row-side">
            <span className="segmented">
              {LANGS.map((code) => (
                <button
                  key={code}
                  type="button"
                  aria-pressed={lang === code}
                  onClick={() => onLang(code)}
                >
                  {LANG_NAME[code]}
                </button>
              ))}
            </span>
          </span>
        </div>

        <div className="row">
          <span className="row-main">
            <span className="row-title">{t("more.theme")}</span>
          </span>
        </div>
        <div className="row">
          <span className="segmented">
            {THEME_OPTIONS.map(([value, key]) => (
              <button
                key={value}
                type="button"
                aria-pressed={theme === value}
                onClick={() => onTheme(value)}
              >
                {t(key)}
              </button>
            ))}
          </span>
        </div>

        <button type="button" className="row" onClick={onTimeline}>
          <span className="row-main">
            <span className="row-title">{t("tl.title")}</span>
          </span>
          <span className="row-side">
            <IconChevron />
          </span>
        </button>
      </div>

      <div className="section">
        {confirmUnlink ? (
          <>
            <p className="small dim">{t("more.unlinkConfirm")}</p>
            <div className="btn-row">
              <button type="button" className="btn btn-outline" onClick={() => setConfirmUnlink(false)}>
                {t("common.cancel")}
              </button>
              <button type="button" className="btn btn-danger" onClick={() => void unlink()}>
                {t("more.unlinkYes")}
              </button>
            </div>
          </>
        ) : (
          <button
            type="button"
            className="btn btn-quiet danger"
            onClick={() => setConfirmUnlink(true)}
            disabled={!sub?.hasLink}
          >
            {t("more.unlink")}
          </button>
        )}
      </div>
    </Screen>
  );
}
