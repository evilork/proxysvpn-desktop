// src/components/TunnelScreen.tsx
//
// Настройки туннеля: то, что меняет саму работу соединения.
//
// Правило экрана: здесь только переключатели, которые ДЕЙСТВИТЕЛЬНО что-то
// делают. Тумблер, который ничего не меняет, хуже отсутствующего - человек
// щёлкает его и вправе считать, что настройка применилась.
//
// Поэтому мультиплексирование показано, но недоступно, и рядом написано
// почему: наши узлы идут с XTLS Vision, а он с мультиплексированием
// несовместим. Спрятать его было бы проще, но тогда вопрос «а почему у вас
// нет mux» остался бы без ответа.
//
// App Store builds (src/dist.ts) drop it entirely: there a switch that is
// off forever is a non-working feature under guideline 2.1, explanation or
// not. For the same reason iOS does not offer the IP-family and DNS choices:
// the Apple engine (xray_apple.rs) resolves by one fixed policy, the one the
// data notice states, and replaces whatever these two would set. "XHTTP only"
// is offered everywhere since iOS runs Xray-core too.
//
// Каждая правка применяется СРАЗУ. Если туннель поднят, ядро переподключается
// само - иначе настройка молчала бы до следующего включения, и человек об этом
// не узнал бы.

import { useCallback, useEffect, useState } from "react";

import {
  bridge,
  type AppInfo,
  type DnsChoice,
  type IpKind,
  type TransportPref,
  type TunnelPrefs,
} from "../bridge";
import { IS_APPSTORE } from "../dist";
import type { MsgKey } from "../i18n";
import { tunnelHintKeys } from "../platformCopy";
import { Screen, Spinner, useUi } from "./ui";

const IP_KINDS: [IpKind, MsgKey][] = [
  ["ipv4", "tun.ip.v4"],
  ["ipv6", "tun.ip.v6"],
  ["both", "tun.ip.both"],
];

const DNS_CHOICES: [DnsChoice, MsgKey][] = [
  ["internal", "tun.dns.internal"],
  ["system", "tun.dns.system"],
  ["custom", "tun.dns.custom"],
];

const TRANSPORTS: [TransportPref, MsgKey][] = [
  ["auto", "tun.transport.auto"],
  ["xhttpOnly", "tun.transport.xhttp"],
  ["visionOnly", "tun.transport.vision"],
];

export default function TunnelScreen({
  onClose,
  platform,
}: {
  onClose: () => void;
  /** Unknown until `app_info` answers; treated as "not iOS" meanwhile. */
  platform?: AppInfo["platform"];
}) {
  const { t } = useUi();
  const [prefs, setPrefs] = useState<TunnelPrefs | null>(null);
  const [busy, setBusy] = useState(false);
  const [reconnecting, setReconnecting] = useState(false);

  useEffect(() => {
    let alive = true;
    void bridge
      .tunnelPrefs()
      .then((p) => {
        if (alive) setPrefs(p);
      })
      .catch(() => {
        // Не прочиталось - покажем умолчания: экран без настроек бесполезен,
        // а ядро в этом случае и само работает по умолчаниям.
        if (alive) {
          setPrefs({
            fragment: false,
            ipKind: "ipv4",
            dns: "internal",
            customDns: "",
            transport: "auto",
            directDomains: [],
            proxyDomains: [],
          });
        }
      });
    return () => {
      alive = false;
    };
  }, []);

  const apply = useCallback(
    async (next: TunnelPrefs) => {
      // Показываем новое состояние СРАЗУ: тумблер, который думает секунду,
      // читается как сломанный.
      setPrefs(next);
      setBusy(true);
      try {
        const live = await bridge.setTunnelPrefs(next);
        setReconnecting(live);
      } catch {
        // Не записалось - возвращаем то, что в ядре, а не то, что нажали.
        const actual = await bridge.tunnelPrefs().catch(() => null);
        if (actual) setPrefs(actual);
      } finally {
        setBusy(false);
      }
    },
    [],
  );

  if (!prefs) {
    return (
      <Screen title={t("tun.title")} onClose={onClose}>
        <p className="body dim">
          <Spinner /> {t("common.loading")}
        </p>
      </Screen>
    );
  }

  const customMissing = prefs.dns === "custom" && prefs.customDns.trim() === "";
  // See the header: on iOS these two choices would change nothing.
  const engineOwnsDns = platform === "ios";
  // On Windows they reach the engine only (src/platformCopy.ts), and the
  // hints say so instead of promising that lookups stay in the tunnel.
  const hints = tunnelHintKeys(platform);

  return (
    <Screen
      title={t("tun.title")}
      onClose={onClose}
      footer={
        reconnecting ? (
          <p className="small faint" style={{ textAlign: "center" }}>
            {t("tun.reconnecting")}
          </p>
        ) : (
          <p className="small faint" style={{ textAlign: "center" }}>
            {t("tun.note")}
          </p>
        )
      }
    >
      <div className="list">
        <label className="row">
          <span className="row-main">
            <span className="row-title">{t("tun.fragment")}</span>
            <span className="row-sub">{t("tun.fragmentHint")}</span>
          </span>
          <span className="row-side">
            <input
              type="checkbox"
              role="switch"
              aria-checked={prefs.fragment}
              checked={prefs.fragment}
              disabled={busy}
              onChange={(e) => void apply({ ...prefs, fragment: e.target.checked })}
            />
          </span>
        </label>

        {/* Показан, но недоступен - и причина написана рядом, а не спрятана.
            Not in App Store builds: there it is a non-working feature (2.1). */}
        {IS_APPSTORE ? null : (
          <div className="row" aria-disabled="true">
            <span className="row-main">
              <span className="row-title dim">{t("tun.mux")}</span>
              <span className="row-sub">{t("tun.muxWhy")}</span>
            </span>
            <span className="row-side">
              <input
                type="checkbox"
                role="switch"
                aria-checked={false}
                checked={false}
                disabled
                readOnly
              />
            </span>
          </div>
        )}

        <div className="row">
          <span className="row-main">
            <span className="row-title">{t("tun.transport")}</span>
            <span className="row-sub">{t("tun.transportHint")}</span>
          </span>
        </div>
        <div className="row">
          <span className="segmented">
            {TRANSPORTS.map(([value, key]) => (
              <button
                key={value}
                type="button"
                aria-pressed={prefs.transport === value}
                disabled={busy}
                onClick={() => void apply({ ...prefs, transport: value })}
              >
                {t(key)}
              </button>
            ))}
          </span>
        </div>

        {engineOwnsDns ? null : (
          <>
            <div className="row">
              <span className="row-main">
                <span className="row-title">{t("tun.ip")}</span>
                <span className="row-sub">{t(hints.ip)}</span>
              </span>
            </div>
            <div className="row">
              <span className="segmented">
                {IP_KINDS.map(([value, key]) => (
                  <button
                    key={value}
                    type="button"
                    aria-pressed={prefs.ipKind === value}
                    disabled={busy}
                    onClick={() => void apply({ ...prefs, ipKind: value })}
                  >
                    {t(key)}
                  </button>
                ))}
              </span>
            </div>

            <div className="row">
              <span className="row-main">
                <span className="row-title">{t("tun.dns")}</span>
                <span className="row-sub">{t(hints.dns)}</span>
              </span>
            </div>
            <div className="row">
              <span className="segmented">
                {DNS_CHOICES.map(([value, key]) => (
                  <button
                    key={value}
                    type="button"
                    aria-pressed={prefs.dns === value}
                    disabled={busy}
                    onClick={() => void apply({ ...prefs, dns: value })}
                  >
                    {t(key)}
                  </button>
                ))}
              </span>
            </div>

            {prefs.dns === "custom" ? (
              <div className="row">
                <span className="row-main">
                  <input
                    type="text"
                    className="field"
                    value={prefs.customDns}
                    placeholder={t("tun.dnsPlaceholder")}
                    spellCheck={false}
                    autoCapitalize="off"
                    autoCorrect="off"
                    disabled={busy}
                    onChange={(e) => setPrefs({ ...prefs, customDns: e.target.value })}
                    onBlur={() => void apply(prefs)}
                  />
                  <span className="row-sub">
                    {customMissing ? t("tun.dnsMissing") : t("tun.dnsOwnHint")}
                  </span>
                </span>
              </div>
            ) : null}
          </>
        )}
      </div>
    </Screen>
  );
}
