// src/App.tsx
//
// The shell: it owns the truth about the tunnel, the navigation stack and the
// two preferences (language, appearance). Every screen below is a pure view
// over what arrives here.
//
// Where the truth comes from: `vpn:*` events for liveness and `vpn_snapshot`
// for correctness. Events can be missed — a reloaded window, a backgrounded
// app — so the snapshot is re-read on mount and whenever the window comes
// back into view. The old screen asked once at mount and never again, which
// is why a dead tunnel could stay green forever.
//
// Depth rule from DESIGN.md: no more than two taps from the main screen to
// any action. The stack below is therefore mostly one level deep.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import "./App.css";
import {
  IS_TAURI,
  bridge,
  toAppError,
  type AppInfo,
  type OnboardingStep,
  type SubState,
} from "./bridge";
import { makeT, type Lang } from "./i18n";
import {
  LANG_KEY,
  THEME_KEY,
  applyTheme,
  readStored,
  readThemePref,
  watchSystemTheme,
  writeStored,
} from "./prefs";
import type {
  AppError,
  ErrorAction,
  MetricPayload,
  StatePayload,
  SubMeta,
  VpnStep,
} from "./types";

import CheckScreen from "./components/CheckScreen";
import DemoStrip from "./components/DemoStrip";
import DetailsSheet from "./components/DetailsSheet";
import HealingSheet from "./components/HealingSheet";
import LocationsScreen from "./components/LocationsScreen";
import LogViewer from "./components/LogViewer";
import MainScreen from "./components/MainScreen";
import MoreScreen from "./components/MoreScreen";
import TunnelScreen from "./components/TunnelScreen";
import OnboardingScreen from "./components/OnboardingScreen";
import PairScreen from "./components/PairScreen";
import ProblemScreen from "./components/ProblemScreen";
import ReportScreen from "./components/ReportScreen";
import SubscriptionScreen from "./components/SubscriptionScreen";
import TimelineScreen from "./components/TimelineScreen";
import WhereScreen from "./components/WhereScreen";
import { Toast, UiProvider, type ThemePref, type UiContextValue } from "./components/ui";

type Route =
  | "pair"
  | "locations"
  | "check"
  | "problem"
  | "subscription"
  | "more"
  | "timeline"
  | "techlog"
  | "report"
  | "where"
  | "tunnel";

const TOAST_MS = 4000;
/** Recovery is silent below this; past it the sheet may be opened. */
const HEAL_SHEET_AT_MS = 8000;

function initialLang(): Lang {
  const stored = readStored(LANG_KEY);
  if (stored === "ru" || stored === "en") return stored;
  return typeof navigator !== "undefined" && navigator.language.startsWith("en") ? "en" : "ru";
}

export default function App() {
  const [lang, setLang] = useState<Lang>(initialLang);
  const [theme, setTheme] = useState<ThemePref>(readThemePref);

  const [state, setState] = useState<StatePayload>({ phase: "off" });
  const [step, setStep] = useState<VpnStep | undefined>(undefined);
  const [metric, setMetric] = useState<MetricPayload>({ rxBytes: 0, txBytes: 0 });
  const [meta, setMeta] = useState<SubMeta>({});
  const [sub, setSub] = useState<SubState | null>(null);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [onboarding, setOnboarding] = useState<OnboardingStep[] | null>(null);

  const [stack, setStack] = useState<Route[]>([]);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [healOpen, setHealOpen] = useState(false);
  const [toastText, setToastText] = useState<string | null>(null);
  const [forcedError, setForcedError] = useState<AppError | null>(null);
  const [busy, setBusy] = useState(false);
  const toastTimer = useRef<number | null>(null);

  const t = useMemo(() => makeT(lang), [lang]);

  // ── preferences ──────────────────────────────────────────────────────────

  useEffect(() => {
    writeStored(LANG_KEY, lang);
    document.documentElement.setAttribute("lang", lang);
  }, [lang]);

  useEffect(() => {
    writeStored(THEME_KEY, theme);
    applyTheme(theme);
    // "Как в системе" is a standing subscription, not a one-off read: the
    // person can flip macOS to light while the window is open, and before
    // this the window kept whatever it had resolved at startup.
    if (theme !== "system") return undefined;
    return watchSystemTheme(() => applyTheme("system"));
  }, [theme]);

  // ── toast ────────────────────────────────────────────────────────────────

  const toast = useCallback((message: string) => {
    setToastText(message);
    if (toastTimer.current !== null) window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToastText(null), TOAST_MS);
  }, []);

  useEffect(
    () => () => {
      if (toastTimer.current !== null) window.clearTimeout(toastTimer.current);
    },
    [],
  );

  const openExternal = useCallback(
    (url: string) => {
      void bridge.openExternal(url).catch(() => toast(t("err.UNKNOWN.title")));
    },
    [t, toast],
  );

  const ui: UiContextValue = useMemo(
    () => ({ t, lang, toast, openExternal }),
    [t, lang, toast, openExternal],
  );

  // ── truth from the core ──────────────────────────────────────────────────

  const pullSnapshot = useCallback(async () => {
    try {
      const snapshot = await bridge.snapshot();
      setState(snapshot.state);
      setStep(snapshot.step);
      setMetric(snapshot.metric);
      setMeta(snapshot.meta);
    } catch (err) {
      // A core that cannot answer at all is itself a failure worth naming.
      setState({ phase: "failed", error: toAppError(err) });
    }
  }, []);

  useEffect(() => {
    const unsubscribe = bridge.subscribe({
      onState: (next) => {
        setState(next);
        if (next.phase !== "starting") setStep(undefined);
      },
      onStep: setStep,
      onMetric: setMetric,
      onMeta: setMeta,
      onEvent: (event) => {
        // `handled` means the core is already fixing it and the person is not
        // being asked for anything — those stay silent by design.
        if (!event.handled) toast(t(`err.${event.error.code}.title`));
      },
    });
    return unsubscribe;
  }, [t, toast]);

  useEffect(() => {
    void pullSnapshot();
    void bridge
      .subState()
      .then(setSub)
      .catch(() => setSub({ hasLink: false }));
    void bridge
      .appInfo()
      .then(setInfo)
      .catch(() => setInfo(null));
    void bridge
      .onboarding()
      .then((value) => setOnboarding(value.steps))
      .catch(() => setOnboarding([]));
  }, [pullSnapshot]);

  // Events are missed while the window is hidden; the snapshot is the cure.
  useEffect(() => {
    const onVisible = () => {
      if (document.visibilityState === "visible") void pullSnapshot();
    };
    document.addEventListener("visibilitychange", onVisible);
    window.addEventListener("focus", onVisible);
    return () => {
      document.removeEventListener("visibilitychange", onVisible);
      window.removeEventListener("focus", onVisible);
    };
  }, [pullSnapshot]);

  // ── navigation ───────────────────────────────────────────────────────────

  const push = useCallback((route: Route) => {
    setDetailsOpen(false);
    setStack((previous) => (previous[previous.length - 1] === route ? previous : [...previous, route]));
  }, []);

  const pop = useCallback(() => setStack((previous) => previous.slice(0, -1)), []);
  const reset = useCallback(() => setStack([]), []);

  const top = stack[stack.length - 1];

  // ── the one button ───────────────────────────────────────────────────────

  const connect = useCallback(async () => {
    setBusy(true);
    try {
      await bridge.connect();
    } catch (err) {
      // `connect` can fail before the core emits anything (no link, malformed
      // link), so the rejection has to become a phase by itself.
      setState({ phase: "failed", error: toAppError(err) });
    } finally {
      setBusy(false);
    }
  }, []);

  const disconnect = useCallback(async () => {
    setBusy(true);
    try {
      await bridge.disconnect();
    } catch (err) {
      setState({ phase: "failed", error: toAppError(err) });
    } finally {
      setBusy(false);
    }
  }, []);

  const toggle = useCallback(() => {
    if (state.phase === "off" || state.phase === "failed") void connect();
    else void disconnect();
  }, [state.phase, connect, disconnect]);

  const refreshSubState = useCallback(async () => {
    try {
      setSub(await bridge.subState());
    } catch {
      // Keep the previous answer: guessing "no link" would throw the person
      // back onto the pairing screen for no reason.
    }
  }, []);

  // ── recovery sheet ───────────────────────────────────────────────────────

  useEffect(() => {
    if (state.phase === "healing" && (state.healingForMs ?? 0) >= HEAL_SHEET_AT_MS) {
      setHealOpen(true);
    }
  }, [state]);

  // ── what a failure offers ────────────────────────────────────────────────

  const runAction = useCallback(
    (action: ErrorAction) => {
      switch (action) {
        case "addLink":
          reset();
          void refreshSubState();
          break;
        case "retry":
          setForcedError(null);
          reset();
          void connect();
          break;
        case "openCabinet":
          if (info) openExternal(info.cabinetUrl);
          break;
        case "topUp":
          if (info) openExternal(info.cabinetUrl);
          break;
        case "diagnose":
          setStack(["check"]);
          break;
        case "waitAndSee":
          setForcedError(null);
          reset();
          break;
        case "contactSupport":
          setStack(["report"]);
          break;
      }
    },
    [connect, info, openExternal, refreshSubState, reset],
  );

  // A reason the SCREENS name themselves, for the cases the core has no phase
  // for — an empty location list is always one of the subscription states, and
  // "UNKNOWN" would send the person to support for a billing problem.
  const problemError: AppError = forcedError ?? state.error ?? { code: "UNKNOWN" };
  const cabinetUrl = info?.cabinetUrl ?? "https://proxysvpn.com";
  // `support-url` is what the SERVICE says support is today; the bundled bot
  // address is only the fallback for a subscription that never answered.
  const botUrl = meta.supportUrl ?? info?.botUrl ?? cabinetUrl;

  // ── render ───────────────────────────────────────────────────────────────

  // Until `sub_state` answers we assume there IS a link: that is true for
  // everyone but a first run, and guessing the other way makes every launch
  // flash "Не настроено" for a frame.
  const hasLink = sub?.hasLink ?? true;
  const needsOnboarding = onboarding !== null && onboarding.length > 0;

  const screen = () => {
    if (needsOnboarding) {
      return (
        <OnboardingScreen
          steps={onboarding}
          onDone={() => {
            setOnboarding([]);
            void refreshSubState();
          }}
        />
      );
    }

    // No link: [1] is the whole app until there is one. It has no close
    // button, because there is nowhere to close it to.
    if (sub !== null && !hasLink && top !== "pair") {
      return (
        <PairScreen
          onDone={() => {
            void refreshSubState().then(() => void connect());
          }}
        />
      );
    }

    switch (top) {
      case "pair":
        return (
          <PairScreen
            onClose={pop}
            onDone={() => {
              pop();
              void refreshSubState().then(() => void connect());
            }}
          />
        );
      case "locations":
        return (
          <LocationsScreen
            currentLabel={state.location}
            onClose={pop}
            onPicked={(label) => {
              pop();
              toast(label ? t("loc.switched", { location: label }) : t("loc.auto"));
            }}
            onEmpty={() => {
              setForcedError({ code: "SUB_EMPTY" });
              setStack(["problem"]);
            }}
          />
        );
      case "check":
        return (
          <CheckScreen
            onClose={pop}
            onReport={() => push("report")}
            onOpenCabinet={() => openExternal(cabinetUrl)}
          />
        );
      case "problem":
        return (
          <ProblemScreen
            error={problemError}
            busy={busy}
            onAction={runAction}
            onClose={() => {
              setForcedError(null);
              pop();
            }}
            onCheck={() => push("check")}
            onTimeline={() => push("timeline")}
          />
        );
      case "subscription":
        return (
          <SubscriptionScreen
            meta={meta}
            info={info}
            sub={sub}
            onClose={pop}
            onOpenCabinet={() => openExternal(cabinetUrl)}
            onOpenBot={() => openExternal(botUrl)}
          />
        );
      case "tunnel":
        return <TunnelScreen onClose={pop} />;
      case "more":
        return (
          <MoreScreen
            info={info}
            sub={sub}
            theme={theme}
            onTheme={setTheme}
            onLang={setLang}
            onClose={pop}
            onSubscription={() => push("subscription")}
            onWhere={() => push("where")}
            onTunnel={() => push("tunnel")}
            onTimeline={() => push("timeline")}
            onUnlinked={() => {
              reset();
              void refreshSubState();
            }}
            onOpenCabinet={() => openExternal(cabinetUrl)}
          />
        );
      case "timeline":
        return (
          <TimelineScreen
            onClose={pop}
            onReport={() => push("report")}
            onTechLog={() => push("techlog")}
          />
        );
      case "techlog":
        return <LogViewer onClose={pop} />;
      case "report":
        return <ReportScreen onClose={pop} onOpenBot={() => openExternal(botUrl)} />;
      case "where":
        return <WhereScreen onClose={pop} />;
      default:
        return null;
    }
  };

  return (
    <UiProvider value={ui}>
      <div className="app">
        <MainScreen
          state={state}
          step={step}
          metric={metric}
          meta={meta}
          hasLink={hasLink}
          busy={busy}
          onToggle={toggle}
          onAddLink={() => push("pair")}
          onWhatHappened={() => push("problem")}
          onOpenDetails={() => setDetailsOpen(true)}
          onOpenLocations={() => push("locations")}
          onOpenCheck={() => push("check")}
          onOpenMore={() => push("more")}
        />
      </div>

      {detailsOpen ? (
        <DetailsSheet
          state={state}
          meta={meta}
          onClose={() => setDetailsOpen(false)}
          onLocations={() => push("locations")}
          onWhere={() => push("where")}
          onSubscription={() => push("subscription")}
          onCheck={() => push("check")}
        />
      ) : null}

      {healOpen ? (
        <HealingSheet
          state={state}
          onClose={() => setHealOpen(false)}
          onFailed={() => {
            setHealOpen(false);
            setStack(["problem"]);
          }}
          onCancel={() => {
            setHealOpen(false);
            void disconnect();
          }}
        />
      ) : null}

      {screen()}

      {toastText ? <Toast message={toastText} /> : null}
      {IS_TAURI ? null : <DemoStrip />}
    </UiProvider>
  );
}
