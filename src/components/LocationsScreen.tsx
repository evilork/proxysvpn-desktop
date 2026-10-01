// src/components/LocationsScreen.tsx
//
// [5] Countries — the manual choice, for the minority who need a specific one.
//
// Миллисекунды ЕСТЬ - по решению владельца от 22.09.2026. Раньше здесь были
// только слова, и довод был такой: одно и то же число значит на наших
// платформах разное, а на Hysteria2 рукопожатия TCP не бывает вовсе.
//
// Первая половина довода снята тем, ЧТО именно мы теперь меряем: это
// рукопожатие TCP до самого узла, снятое одинаково для всех строк списка, а
// не смесь замеров с разных путей. Такие числа сравнимы между собой - ровно
// для этого список и открывают.
//
// Вторая половина довода осталась верной и соблюдена: у локаций на Hysteria2
// числа НЕТ, и на их месте по-прежнему слово. Ноль, прочерк и «ошибка» там
// недопустимы - именно так когда-то и родился вечный спиннер.
//
// С включённым VPN список показывал 1-5 мс у всех стран (02.10.2026): обычное
// рукопожатие уходило в наш же туннель, и отвечал на него tun2socks. Теперь
// ядро меряет мимо туннеля, через физический интерфейс, а где не может - не
// меряет вовсе и оставляет прежнее число, снятое мимо туннеля. Такое число
// показывается с возрастом («234 мс · замер 12 минут назад», src/latency.ts).
//
// Nothing here carries an address or a port — not in the row, not in
// `aria-label`, not in a tooltip. An unnamed entry becomes "Сервер №3", never
// its host name.
//
// The protocol IS shown, since 27.09.2026 (owner): «Британия» on Vision and
// «Британия · XHTTP» were two identical rows. The title is the whole name the
// service wrote (the part after «·» included), and the line under it says the
// protocol next to the milliseconds.

import { useCallback, useEffect, useState } from "react";

import { bridge, type LocationEntry, type LocationQuality } from "../bridge";
import { flagFor, rendersFlagEmoji } from "../flags";
import { displayLocation } from "../locationName";
import { formatAge, formatTime, type MsgKey } from "../i18n";
import { rttMeasuredAtToShow } from "../latency";
import { Screen, Spinner, useLatest, useUi } from "./ui";

const QUALITY_KEY: Record<LocationQuality, MsgKey> = {
  good: "loc.q.good",
  ok: "loc.q.ok",
  poor: "loc.q.poor",
  blocked: "loc.q.blocked",
  unknown: "loc.q.unknown",
};

export default function LocationsScreen({
  currentLabel,
  onClose,
  onPicked,
  onEmpty,
}: {
  /** What the tunnel is using right now, for the "Сейчас: …" line. */
  currentLabel?: string;
  onClose: () => void;
  /** null = back to automatic. The label is only used for the toast. */
  onPicked: (label: string | null) => void;
  /** An empty list always means one of the subscription states — show [7]. */
  onEmpty: () => void;
}) {
  const { t, lang } = useUi();
  const [items, setItems] = useState<LocationEntry[] | null>(null);
  const [refreshing, setRefreshing] = useState(true);
  const [staleAt, setStaleAt] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  /** This opening's latency round is back, so a number's age is worth saying. */
  const [measured, setMeasured] = useState(false);

  // The list is fetched once per opening, never on a timer: the subscription
  // has a request budget and this screen is not allowed to eat it.
  const emptyRef = useLatest(onEmpty);
  const pickedRef = useLatest(onPicked);

  const load = useCallback(async () => {
    setRefreshing(true);
    try {
      const list = await bridge.locations();
      if (list.length === 0) {
        emptyRef.current();
        return;
      }
      setItems(list);
      setStaleAt(null);
    } catch {
      // An old list beats an empty screen: it is still mostly right, and the
      // person can still connect somewhere.
      setStaleAt(Date.now());
    } finally {
      setRefreshing(false);
    }
  }, [emptyRef]);

  useEffect(() => {
    void load();
  }, [load]);

  // Числа приезжают ВТОРЫМ заходом, уже поверх открытого списка.
  //
  // Слить это с `load` нельзя: рукопожатие до десятка узлов занимает секунды,
  // и человек всё это время смотрел бы на пустой экран вместо готового
  // списка, по которому уже можно ткнуть.
  useEffect(() => {
    if (!items) return;
    let alive = true;
    void bridge
      .measureLocations()
      .then((list) => {
        // Экран мог закрыться или список - обновиться, пока мы мерили.
        if (alive && list.length > 0) setItems(list);
      })
      .catch(() => {
        // Замер - украшение: без него список остаётся со словами и полностью
        // рабочим. Ошибку показывать не за что.
      })
      .finally(() => {
        if (alive) setMeasured(true);
      });
    return () => {
      alive = false;
    };
    // Намеренно один раз за открытие: перемер на каждое изменение списка
    // превратил бы экран в метроном по узлам.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [items !== null]);

  const pick = useCallback(
    async (id: string | null, label: string | null) => {
      setBusy(true);
      try {
        await bridge.selectLocation(id);
        pickedRef.current(label);
      } catch {
        // Selection is the one place where failing silently would leave the
        // person looking at a country they are not on.
        setBusy(false);
      }
    },
    [pickedRef],
  );

  // Windows draws flag emoji as two boxed letters (src/flags.ts).
  const showFlags = rendersFlagEmoji(navigator.userAgent);
  const pinned = items?.find((entry) => entry.selected) ?? null;
  const recent = items?.filter((entry) => entry.recent) ?? [];
  const all = items ?? [];

  /**
   * The whole name the service wrote, «Британия · XHTTP». The badge after «·»
   * goes through `displayLocation` too: «Амстердам · резерв» is
   * "Amsterdam · reserve" in an English window, not "Amsterdam · резерв".
   */
  const titleOf = (entry: LocationEntry, index: number) => {
    const base = entry.label ? displayLocation(entry.label, lang) : t("loc.unnamed", { n: index + 1 });
    return entry.note ? `${base} · ${displayLocation(entry.note, lang)}` : base;
  };

  /**
   * The number, or the word when there is none. A number the core kept from
   * an earlier round (it could not measure around the tunnel this time)
   * carries its age.
   */
  const rttText = (entry: LocationEntry) => {
    if (entry.rttMs === undefined) return t(QUALITY_KEY[entry.quality]);
    const now = Date.now();
    // Until this opening's round is back, every cached number looks old and
    // would flash "measured N minutes ago" for the second the round takes.
    const at = measured ? rttMeasuredAtToShow(entry, now) : null;
    return at === null
      ? t("loc.ms", { ms: entry.rttMs })
      : t("loc.msAged", { ms: entry.rttMs, ago: formatAge(t, at, now) });
  };

  const renderRow = (entry: LocationEntry, index: number) => (
    <button
      key={entry.id}
      type="button"
      className="row"
      onClick={() => void pick(entry.id, titleOf(entry, index))}
      disabled={busy}
      data-selected={entry.selected ? "true" : undefined}
      aria-current={entry.selected ? "true" : undefined}
    >
      {showFlags ? (
        <span className="row-flag" aria-hidden="true">
          {entry.flag ?? flagFor(entry.label)}
        </span>
      ) : null}
      <span className="row-main">
        <span className="row-title">{titleOf(entry, index)}</span>
        <span className="row-sub">
          {rttText(entry)}
          {entry.protocol ? ` · ${entry.protocol}` : ""}
        </span>
      </span>
      {entry.selected ? <span className="badge">{t("loc.pinned")}</span> : null}
    </button>
  );

  return (
    <Screen
      title={t("loc.title")}
      onClose={onClose}
      footer={<p className="small faint">{t("loc.pinNote")}</p>}
    >
      {refreshing && !items ? (
        <p className="body dim">
          <Spinner /> {t("loc.refreshing")}
        </p>
      ) : null}

      {staleAt !== null ? (
        <div className="servicebar">
          <span>{t("loc.stale", { time: formatTime(lang, staleAt) })}</span>
          <button type="button" onClick={() => void load()}>
            {t("loc.refresh")}
          </button>
        </div>
      ) : null}

      {items ? (
        <>
          <div className="list">
            <button
              type="button"
              className="row"
              onClick={() => void pick(null, null)}
              disabled={busy}
              data-selected={pinned === null ? "true" : undefined}
              aria-current={pinned === null ? "true" : undefined}
            >
              <span className="row-main">
                <span className="row-title">{t("loc.auto")}</span>
                <span className="row-sub">
                  {t("loc.autoHint")}
                  {currentLabel ? ` · ${t("loc.current", { location: displayLocation(currentLabel, lang) })}` : ""}
                </span>
              </span>
            </button>
          </div>

          {recent.length > 0 ? (
            <div className="section">
              <span className="caps">{t("loc.recent")}</span>
              <div className="list">{recent.slice(0, 3).map(renderRow)}</div>
            </div>
          ) : null}

          <div className="section">
            <span className="caps">{t("loc.all")}</span>
            <div className="list">{all.map(renderRow)}</div>
          </div>
        </>
      ) : null}
    </Screen>
  );
}
