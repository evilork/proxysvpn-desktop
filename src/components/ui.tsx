// src/components/ui.tsx
//
// The handful of parts every screen is built from: a full-window screen, a
// bottom sheet, a toast, the icons, and the context that carries `t` so no
// component has to be handed the dictionary through five layers of props.
//
// Icons are drawn here instead of pulled from an icon package: there are six
// of them, they have to line up with a 44 pt target and follow `currentColor`
// in both themes, and a dependency is a poor trade for that.

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";

import { createEscapeStack } from "../escapeStack";
import type { Lang, Translate } from "../i18n";

/** What [12] offers under "Оформление". "system" is the default and the norm. */
export type ThemePref = "system" | "light" | "dark";

export interface UiContextValue {
  t: Translate;
  lang: Lang;
  /** One short line at the bottom of the window, gone in a couple of seconds. */
  toast: (message: string) => void;
  /** Opens a link outside the app; never navigates the window itself. */
  openExternal: (url: string) => void;
}

const UiContext = createContext<UiContextValue | null>(null);

export function UiProvider({
  value,
  children,
}: {
  value: UiContextValue;
  children: ReactNode;
}) {
  return <UiContext.Provider value={value}>{children}</UiContext.Provider>;
}

export function useUi(): UiContextValue {
  const value = useContext(UiContext);
  if (!value) {
    // A screen rendered outside the provider would silently lose every string;
    // failing loudly in development is the cheaper outcome.
    throw new Error("useUi() outside <UiProvider>");
  }
  return value;
}

/**
 * A callback an effect can call without listing it as a dependency.
 *
 * Handlers arrive from App as fresh arrows on every render, and App re-renders
 * on every metric event. An effect that depends on one of them would re-arm
 * its timer a few times a second — which means a 2 s poll never fires and a
 * 1.5 s "we are done" timeout never elapses. This keeps the latest handler
 * reachable while the effect itself runs exactly once.
 */
export function useLatest<T>(value: T): { readonly current: T } {
  const ref = useRef(value);
  ref.current = value;
  return ref;
}

/** Re-renders on a timer so "проверено 9 секунд назад" actually ages. */
export function useNow(intervalMs: number): number {
  const [now, setNow] = useState<number>(() => Date.now());
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), intervalMs);
    return () => window.clearInterval(id);
  }, [intervalMs]);
  return now;
}

/** One stack for the whole window: see src/escapeStack.ts. */
const ESCAPE = createEscapeStack();
let escapeListening = false;

function onEscapeKey(event: KeyboardEvent): void {
  if (ESCAPE.handle(event.key)) event.stopPropagation();
}

/**
 * Esc closes whatever is on top — only that. Registered when an overlay
 * mounts, removed on unmount; the layer keeps its place in the stack while
 * its `onClose` changes identity between renders. A screen with nowhere to go
 * back to passes `undefined` and keeps the key.
 */
function useEscape(onClose: (() => void) | undefined): void {
  const latest = useRef(onClose);
  useEffect(() => {
    latest.current = onClose;
  });
  const enabled = onClose !== undefined;
  useEffect(() => {
    if (!enabled) return;
    if (!escapeListening) {
      window.addEventListener("keydown", onEscapeKey);
      escapeListening = true;
    }
    return ESCAPE.push(() => latest.current?.());
  }, [enabled]);
}

// ── Icons ───────────────────────────────────────────────────────────────────

interface IconProps {
  size?: number;
}

function svgProps(size: number) {
  return {
    width: size,
    height: size,
    viewBox: "0 0 24 24",
    fill: "none",
    stroke: "currentColor",
    strokeWidth: 2,
    strokeLinecap: "round" as const,
    strokeLinejoin: "round" as const,
    "aria-hidden": true,
    focusable: false,
  };
}

export function IconClose({ size = 20 }: IconProps) {
  return (
    <svg {...svgProps(size)}>
      <path d="M6 6l12 12M18 6L6 18" />
    </svg>
  );
}

export function IconBack({ size = 20 }: IconProps) {
  return (
    <svg {...svgProps(size)}>
      <path d="M15 5l-7 7 7 7" />
    </svg>
  );
}

export function IconChevron({ size = 18 }: IconProps) {
  return (
    <svg {...svgProps(size)}>
      <path d="M9 5l7 7-7 7" />
    </svg>
  );
}

export function IconGear({ size = 20 }: IconProps) {
  return (
    <svg {...svgProps(size)}>
      <circle cx="12" cy="12" r="3" />
      {/* Teeth drawn as a ring with gaps rather than spokes: spokes at this
          size read as a sun, which is the icon for "light theme". */}
      <path d="M19.4 15a1.6 1.6 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.6 1.6 0 0 0-1.8-.3 1.6 1.6 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1A1.6 1.6 0 0 0 9 19.4a1.6 1.6 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.6 1.6 0 0 0 .3-1.8 1.6 1.6 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1A1.6 1.6 0 0 0 4.6 9a1.6 1.6 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.6 1.6 0 0 0 1.8.3H9a1.6 1.6 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.6 1.6 0 0 0 1 1.5 1.6 1.6 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.6 1.6 0 0 0-.3 1.8V9a1.6 1.6 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.6 1.6 0 0 0-1.5 1z" />
    </svg>
  );
}

export function IconGlobe({ size = 20 }: IconProps) {
  return (
    <svg {...svgProps(size)}>
      <circle cx="12" cy="12" r="9" />
      <path d="M3 12h18M12 3c2.5 2.6 3.8 5.7 3.8 9S14.5 18.4 12 21c-2.5-2.6-3.8-5.7-3.8-9S9.5 5.6 12 3z" />
    </svg>
  );
}

export function IconStethoscope({ size = 20 }: IconProps) {
  return (
    <svg {...svgProps(size)}>
      <path d="M6 3v5a4 4 0 0 0 8 0V3" />
      <path d="M10 12v2a5 5 0 0 0 5 5 4 4 0 0 0 4-4v-2" />
      <circle cx="19" cy="11" r="1.6" />
    </svg>
  );
}

// ── Screen ──────────────────────────────────────────────────────────────────

export function Screen({
  title,
  onClose,
  closeKind = "close",
  children,
  footer,
  right,
  centered = false,
}: {
  title: string;
  /** Absent when this screen is the only thing the person can do right now.
      A close button that closes nothing is worse than no close button. */
  onClose?: () => void;
  /** "back" inside a chain of screens, "close" for a leaf. */
  closeKind?: "close" | "back";
  children: ReactNode;
  footer?: ReactNode;
  right?: ReactNode;
  /** Centre short content vertically.
      A screen that is one sentence and one button leaves half the window
      empty between them when the body is top-aligned and the footer is
      pinned — it reads as a screen that failed to load. Lists and forms
      must NOT set this: they belong at the top. */
  centered?: boolean;
}) {
  const { t } = useUi();
  useEscape(onClose);

  return (
    <div className="screen" role="dialog" aria-modal="true" aria-label={title}>
      <div className="screen-inner">
        <div className="screen-head">
          {onClose ? (
            <button
              type="button"
              className="icon-btn"
              onClick={onClose}
              aria-label={closeKind === "back" ? t("common.back") : t("common.close")}
            >
              {closeKind === "back" ? <IconBack /> : <IconClose />}
            </button>
          ) : null}
          <h2 className="h2">{title}</h2>
          {right}
        </div>
        <div className={centered ? "screen-body screen-body--center" : "screen-body"}>
          {children}
        </div>
        {footer ? <div className="screen-foot">{footer}</div> : null}
      </div>
    </div>
  );
}

// ── Sheet ───────────────────────────────────────────────────────────────────

/**
 * Bottom sheet. Closes on Esc, on a tap outside, and on a drag down — the
 * three gestures a person tries, all doing the same thing on both platforms.
 */
export function Sheet({
  title,
  onClose,
  children,
}: {
  title: string;
  onClose: () => void;
  children: ReactNode;
}) {
  const [dragY, setDragY] = useState(0);
  const startY = useRef<number | null>(null);
  const sheetRef = useRef<HTMLDivElement | null>(null);
  useEscape(onClose);

  useEffect(() => {
    sheetRef.current?.focus();
  }, []);

  const onPointerDown = useCallback((event: React.PointerEvent<HTMLDivElement>) => {
    startY.current = event.clientY;
    event.currentTarget.setPointerCapture(event.pointerId);
  }, []);

  const onPointerMove = useCallback((event: React.PointerEvent<HTMLDivElement>) => {
    if (startY.current === null) return;
    setDragY(Math.max(0, event.clientY - startY.current));
  }, []);

  const onPointerUp = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      if (startY.current === null) return;
      const travelled = event.clientY - startY.current;
      startY.current = null;
      setDragY(0);
      if (travelled > 60) onClose();
    },
    [onClose],
  );

  return (
    <div
      className="backdrop"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        className="sheet"
        ref={sheetRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        style={dragY > 0 ? { transform: `translateY(${dragY}px)` } : undefined}
      >
        <div
          className="sheet-grip"
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerCancel={onPointerUp}
        />
        {children}
      </div>
    </div>
  );
}

// ── Rows ────────────────────────────────────────────────────────────────────

export function NavRow({
  title,
  sub,
  side,
  lead,
  onClick,
  selected,
  disabled,
}: {
  title: string;
  sub?: string;
  side?: ReactNode;
  lead?: ReactNode;
  onClick?: () => void;
  selected?: boolean;
  disabled?: boolean;
}) {
  const content = (
    <>
      {lead}
      <span className="row-main">
        <span className="row-title">{title}</span>
        {sub ? <span className="row-sub">{sub}</span> : null}
      </span>
      {side !== undefined ? (
        <span className="row-side">{side}</span>
      ) : onClick ? (
        <span className="row-side">
          <IconChevron />
        </span>
      ) : null}
    </>
  );

  if (!onClick) {
    return <div className="row">{content}</div>;
  }

  return (
    <button
      type="button"
      className="row"
      onClick={onClick}
      disabled={disabled}
      data-selected={selected ? "true" : undefined}
      aria-current={selected ? "true" : undefined}
    >
      {content}
    </button>
  );
}

// ── Toast ───────────────────────────────────────────────────────────────────

export function Toast({ message }: { message: string }) {
  return (
    <div className="toast" role="status" aria-live="polite">
      {message}
    </div>
  );
}

export function Spinner() {
  return <span className="spinner" aria-hidden="true" />;
}
