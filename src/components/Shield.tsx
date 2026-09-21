// src/components/Shield.tsx
//
// One control, five states, four SHAPES.
//
// Colour is not allowed to be the only difference. Roughly one man in twelve
// cannot separate this green from this grey, and macOS "Increase contrast"
// flattens the palette for everyone who turns it on. So:
//
//   thin open outline   — off (and dashed when there is no link yet)
//   outline + arc       — working on it (starting, and loud healing)
//   solid fill + tick   — protected, and only after a byte came back
//   thick outline + dash— up, but unproven ("подтвердить не удалось")
//   broken outline      — not getting through
//
// The gap in the broken variant is drawn with `pathLength={100}`, so it sits
// at the same place regardless of the path's real length.

import type { VpnPhase } from "../types";

/** `nolink` is not a phase of the tunnel — there is nothing to turn on yet. */
export type ShieldState = VpnPhase | "nolink";

const SHIELD_PATH =
  "M60 6 L108 26 V68 C108 98 88 118 60 126 C32 118 12 98 12 68 V26 Z";

export default function Shield({
  state,
  label,
  onClick,
  disabled,
}: {
  state: ShieldState;
  label: string;
  onClick?: () => void;
  disabled?: boolean;
}) {
  const working = state === "starting" || state === "healing";
  const solid = state === "on";
  const broken = state === "failed";
  const unproven = state === "unconfirmed";

  const stroke = solid || working || unproven ? "var(--accent)" : "var(--text-faint)";
  const strokeWidth = unproven ? 7 : broken ? 5 : 5;

  return (
    <button
      type="button"
      className="shield-btn"
      onClick={onClick}
      disabled={disabled || !onClick}
      aria-label={label}
    >
      <svg className="shield-svg" viewBox="0 0 120 132" role="img" aria-label={label}>
        {working ? (
          <circle
            className="shield-arc"
            cx="60"
            cy="66"
            r="58"
            fill="none"
            stroke="var(--accent)"
            strokeWidth="4"
            strokeLinecap="round"
            strokeDasharray="34 330"
          />
        ) : null}

        <path
          d={SHIELD_PATH}
          pathLength={100}
          fill={solid ? "var(--accent)" : "none"}
          stroke={solid ? "none" : stroke}
          strokeWidth={strokeWidth}
          strokeLinejoin="round"
          strokeDasharray={
            broken ? "8 16 76" : state === "nolink" ? "6 7" : undefined
          }
        />

        {solid ? (
          <path
            d="M42 66 l12 13 25-29"
            fill="none"
            stroke="var(--accent-ink)"
            strokeWidth="8"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        ) : null}

        {unproven ? (
          <path
            d="M44 66 h32"
            fill="none"
            stroke="var(--accent)"
            strokeWidth="7"
            strokeLinecap="round"
          />
        ) : null}
      </svg>
    </button>
  );
}
