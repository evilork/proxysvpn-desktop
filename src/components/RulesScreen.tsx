// src/components/RulesScreen.tsx
//
// «Свои правила»: два списка доменов, которые человек вписывает вручную -
// «Всегда напрямую» (в обход туннеля) и «Всегда через VPN». Оба идут в
// маршрутизацию ПЕРЕД профилем подписки (subscription.rs), так что домен из
// этих списков не может быть переопределён профилем.
//
// В отличие от TunnelScreen, здесь не применяется каждое нажатие клавиши:
// список - это вставленный текст, и проверять/переподключать на каждую букву
// значило бы дёргать туннель на каждый символ вставки. Поэтому - кнопка
// «Сохранить», а не onChange. Строки, которые не прошли проверку (адрес,
// ссылка, звёздочка, порт - то, что xray прочтёт не как домен) или оказались
// сразу в обоих списках, показаны рядом с полем, а не потеряны молча.

import { useCallback, useEffect, useState } from "react";

import { bridge, type TunnelPrefs } from "../bridge";
import { Screen, Spinner, useUi } from "./ui";

export default function RulesScreen({ onClose }: { onClose: () => void }) {
  const { t, toast } = useUi();
  const [direct, setDirect] = useState<string | null>(null);
  const [proxy, setProxy] = useState<string | null>(null);
  const [directIgnored, setDirectIgnored] = useState<string[]>([]);
  const [proxyIgnored, setProxyIgnored] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [reconnecting, setReconnecting] = useState(false);

  useEffect(() => {
    let alive = true;
    void bridge
      .tunnelPrefs()
      .then((prefs: TunnelPrefs) => {
        if (!alive) return;
        setDirect(prefs.directDomains.join("\n"));
        setProxy(prefs.proxyDomains.join("\n"));
      })
      .catch(() => {
        if (alive) {
          setDirect("");
          setProxy("");
        }
      });
    return () => {
      alive = false;
    };
  }, []);

  const save = useCallback(async () => {
    setBusy(true);
    try {
      const outcome = await bridge.setCustomRules(direct ?? "", proxy ?? "");
      setDirectIgnored(outcome.directIgnored);
      setProxyIgnored(outcome.proxyIgnored);
      setReconnecting(outcome.reconnected);
      toast(t("rules.saved"));
    } catch {
      toast(t("rules.saveFailed"));
    } finally {
      setBusy(false);
    }
  }, [direct, proxy, t, toast]);

  if (direct === null || proxy === null) {
    return (
      <Screen title={t("rules.title")} onClose={onClose}>
        <p className="body dim">
          <Spinner /> {t("common.loading")}
        </p>
      </Screen>
    );
  }

  return (
    <Screen
      title={t("rules.title")}
      onClose={onClose}
      footer={
        reconnecting ? (
          <p className="small faint" style={{ textAlign: "center" }}>
            {t("rules.reconnecting")}
          </p>
        ) : (
          <p className="small faint" style={{ textAlign: "center" }}>
            {t("rules.note")}
          </p>
        )
      }
    >
      <div className="list">
        <div className="row">
          <span className="row-main">
            <span className="row-title">{t("rules.direct")}</span>
            <span className="row-sub">{t("rules.directHint")}</span>
          </span>
        </div>
        <div className="row">
          <span className="row-main">
            <textarea
              className="field"
              rows={5}
              value={direct}
              placeholder={t("rules.placeholder")}
              spellCheck={false}
              autoCapitalize="off"
              autoCorrect="off"
              disabled={busy}
              onChange={(e) => setDirect(e.target.value)}
            />
            <span className="row-sub">{t("rules.limit")}</span>
            {directIgnored.length > 0 ? (
              <span className="row-sub accent">{t("rules.ignored", { list: directIgnored.join(", ") })}</span>
            ) : null}
          </span>
        </div>

        <div className="row">
          <span className="row-main">
            <span className="row-title">{t("rules.proxy")}</span>
            <span className="row-sub">{t("rules.proxyHint")}</span>
          </span>
        </div>
        <div className="row">
          <span className="row-main">
            <textarea
              className="field"
              rows={5}
              value={proxy}
              placeholder={t("rules.placeholder")}
              spellCheck={false}
              autoCapitalize="off"
              autoCorrect="off"
              disabled={busy}
              onChange={(e) => setProxy(e.target.value)}
            />
            <span className="row-sub">{t("rules.limit")}</span>
            {proxyIgnored.length > 0 ? (
              <span className="row-sub accent">{t("rules.ignored", { list: proxyIgnored.join(", ") })}</span>
            ) : null}
          </span>
        </div>

        <div className="row">
          <button type="button" className="btn btn-primary" disabled={busy} onClick={() => void save()}>
            {busy ? <Spinner /> : t("rules.save")}
          </button>
        </div>
      </div>
    </Screen>
  );
}
