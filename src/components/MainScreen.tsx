// src/components/MainScreen.tsx
//
// [2] The screen almost everyone sees, and usually the only one.
//
// One state word, one button, no required choices. What is deliberately NOT
// here: the server list, the flag carousel, a traffic counter, a session
// timer, a version label, a log button, a QR placeholder.
//
// The service bar at the top carries the outage notice; the expiry lives in
// the quiet line under the button. Splitting them keeps either one from
// pushing the other off the screen, and the outage is the only line allowed
// to be loud.

import { formatAge, daysLeft, type Translate } from "../i18n";
import type { MetricPayload, StatePayload, SubMeta, VpnStep } from "../types";
import Shield, { type ShieldState } from "./Shield";
import { IconGear, IconGlobe, IconStethoscope, useNow, useUi } from "./ui";
import markUrl from "../assets/logo.svg";

/** Healing whispers up to this point and only then becomes a sentence (М3). */
const HEAL_LOUD_AT_MS = 45_000;
/** The expiry line appears a week out, and asks for money three days out. */
const EXPIRY_NOTICE_DAYS = 7;
const EXPIRY_URGENT_DAYS = 3;

const STEP_KEY: Record<VpnStep, Parameters<Translate>[0]> = {
  fetchingSub: "main.starting.fetchingSub",
  pickingServer: "main.starting.pickingServer",
  startingEngine: "main.starting.startingEngine",
  raisingTun: "main.starting.raisingTun",
  probing: "main.starting.probing",
};

export interface MainScreenProps {
  state: StatePayload;
  step?: VpnStep;
  metric: MetricPayload;
  meta: SubMeta;
  hasLink: boolean;
  busy: boolean;
  onToggle: () => void;
  onAddLink: () => void;
  onWhatHappened: () => void;
  onOpenDetails: () => void;
  onOpenLocations: () => void;
  onOpenCheck: () => void;
  onOpenMore: () => void;
}

export default function MainScreen(props: MainScreenProps) {
  const { t, openExternal } = useUi();
  const now = useNow(1000);
  const { state, metric, meta, hasLink } = props;
  const announceUrl = meta.announceUrl ?? "";

  const healingFor = state.healingForMs ?? 0;
  const healingIsLoud = state.phase === "healing" && healingFor >= HEAL_LOUD_AT_MS;
  const healingIsWhisper = state.phase === "healing" && !healingIsLoud;

  const shieldState: ShieldState = hasLink ? state.phase : "nolink";

  // Healing under 45 s keeps saying "Защищено" on purpose: the routes are
  // still up and the traffic is still ours. Calling it broken would teach
  // people to press buttons at the exact moment we are fixing it for them.
  const shieldStateShown: ShieldState = healingIsWhisper ? "healing" : shieldState;

  const headline = (): string => {
    if (!hasLink) return t("main.nolink.title");
    switch (state.phase) {
      case "off":
        return t("main.off.title");
      case "starting":
        return t("main.starting.title");
      case "on":
        return t("main.on.title");
      case "unconfirmed":
        return t("main.unconfirmed.title");
      case "healing":
        return healingIsLoud ? t("main.healing.title") : t("main.on.title");
      case "failed":
        return t(`err.${state.error?.code ?? "UNKNOWN"}.title`);
    }
  };

  const hint = (): string | null => {
    if (!hasLink) return t("main.nolink.hint");
    switch (state.phase) {
      case "off":
        return t("main.off.hint");
      case "starting":
        return props.step ? t(STEP_KEY[props.step]) : null;
      case "unconfirmed":
        return t("main.unconfirmed.hint");
      case "healing":
        return healingIsLoud ? t("main.healing.hint") : null;
      case "failed":
        return t(`err.${state.error?.code ?? "UNKNOWN"}.body`);
      case "on":
        return null;
    }
  };

  const action = (): { label: string; onClick: () => void; primary: boolean } => {
    if (!hasLink) {
      return { label: t("main.nolink.action"), onClick: props.onAddLink, primary: true };
    }
    switch (state.phase) {
      case "starting":
        return { label: t("main.starting.action"), onClick: props.onToggle, primary: false };
      case "healing":
        return healingIsLoud
          ? { label: t("common.cancel"), onClick: props.onToggle, primary: false }
          : { label: t("main.on.action"), onClick: props.onToggle, primary: false };
      case "on":
      case "unconfirmed":
        return { label: t("main.on.action"), onClick: props.onToggle, primary: false };
      case "failed":
        return { label: t("main.failed.action"), onClick: props.onWhatHappened, primary: true };
      case "off":
        return { label: t("main.off.action"), onClick: props.onToggle, primary: true };
    }
  };

  const shieldLabel = (): string => {
    if (!hasLink) return t("main.nolink.hint");
    switch (state.phase) {
      case "off":
        return t("main.shield.off");
      case "starting":
        return t("main.shield.starting");
      case "on":
        return t("main.shield.on");
      case "unconfirmed":
        return t("main.shield.unconfirmed");
      case "healing":
        return t("main.shield.healing");
      case "failed":
        return t("main.shield.failed");
    }
  };

  const showsTruth =
    hasLink &&
    (state.phase === "on" || state.phase === "unconfirmed" || state.phase === "healing");

  const ageLabel = metric.lastProofAt
    ? t("main.truth.checked", { age: formatAge(t, metric.lastProofAt, now) })
    : t("main.truth.checking");

  const expiryDays = meta.expiresAt ? daysLeft(meta.expiresAt * 1000, now) : null;
  const quietLine = (): string | null => {
    if (healingIsWhisper) return t("main.healing.quiet");
    if (expiryDays === null) return null;
    if (expiryDays <= 0) return t("bar.expired");
    if (expiryDays <= EXPIRY_URGENT_DAYS) return t("bar.expirySoon", { n: expiryDays });
    if (expiryDays <= EXPIRY_NOTICE_DAYS) return t("bar.expiry", { n: expiryDays });
    return null;
  };

  const current = action();

  return (
    <>
      <div className="topbar">
        <div className="brand">
          {/* Наша марка, а не абстрактный щит: в шапке то же, что в центре
                экрана и на сайте. */}
            <img className="brand-mark" src={markUrl} alt="" draggable={false} />
          <span>{t("app.name")}</span>
        </div>
        <button
          type="button"
          className="icon-btn"
          onClick={props.onOpenMore}
          aria-label={t("more.title")}
        >
          <IconGear />
        </button>
      </div>

      {/* The outage notice is written by the service and printed verbatim —
          we never paraphrase an incident. It is the one line allowed to be
          loud, and it disappears entirely when there is nothing to say. */}
      {meta.announce ? (
        meta.announceUrl ? (
          <button
            type="button"
            className="servicebar"
            data-tone="warn"
            onClick={() => openExternal(announceUrl)}
          >
            <span>{meta.announce}</span>
          </button>
        ) : (
          <div className="servicebar" data-tone="warn" role="status">
            <span>{meta.announce}</span>
          </div>
        )
      ) : null}

      <div className="stage">
        <Shield
          state={shieldStateShown}
          label={shieldLabel()}
          onClick={hasLink && !props.busy ? props.onToggle : undefined}
          disabled={props.busy}
        />

        {healingIsWhisper ? <div className="whisper" aria-hidden="true" /> : null}

        <div className="headline">
          <h1 className="h1">{headline()}</h1>
          {hint() ? <p className="body dim">{hint()}</p> : null}
        </div>

        {showsTruth ? (
          <button
            type="button"
            className="truthline"
            onClick={props.onOpenDetails}
            aria-label={t("main.truth.details")}
          >
            <span>{state.location ?? t("details.locationAuto")}</span>
            {metric.rttMs !== undefined ? (
              <>
                <span className="sep">·</span>
                <span className="nowrap-num">{t("main.truth.rtt", { n: metric.rttMs })}</span>
              </>
            ) : null}
            <span className="sep">·</span>
            <span>{ageLabel}</span>
          </button>
        ) : null}

        <div style={{ width: "100%", maxWidth: 320 }}>
          <button
            type="button"
            className={current.primary ? "btn btn-primary" : "btn btn-outline"}
            onClick={current.onClick}
          >
            {current.label}
          </button>
          <p className="quiet-note" style={{ marginTop: 10 }}>
            {quietLine()}
          </p>
        </div>
      </div>

      <div className="footbar">
        <button type="button" onClick={props.onOpenLocations} disabled={!hasLink}>
          <IconGlobe size={18} />
          {t("main.nav.country")}
        </button>
        <button type="button" onClick={props.onOpenCheck}>
          <IconStethoscope size={18} />
          {t("main.nav.check")}
        </button>
      </div>
    </>
  );
}
