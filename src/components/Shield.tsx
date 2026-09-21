// src/components/Shield.tsx
//
// The one control on the main screen: our own emblem, with the state drawn
// around it.
//
// Why the state is not painted INTO the emblem. The mark is the brand — a
// shield with an eye, a struck-out ear and a padlock — and recolouring it per
// state would wreck the one thing a logo is for. So the emblem stays itself,
// and a ring around it carries the state.
//
// Colour is still not allowed to be the only difference. Roughly one man in
// twelve cannot separate this sage from this grey, and macOS "Increase
// contrast" flattens the palette for everyone who turns it on. So every state
// has its own RING SHAPE, readable with the colour thrown away:
//
//   no ring, sunken plate  — off
//   faint dotted ring      — no link added yet
//   short arc, turning     — working on it (starting, and loud healing)
//   full unbroken ring     — protected, and only after a byte came back
//   dashed ring            — up, but unproven («подтвердить не удалось»)
//   ring with one gap      — not getting through
//
// The emblem says the same thing in a second channel, without words: grey
// while the tunnel is down, full colour once a byte has crossed.
//
// The art is a vector traced from the original mark (src/assets/logo.svg), so
// it is sharp at any size — the 430 px PNG the site ships went soft the moment
// it was drawn at 160 pt on a Retina panel.

import type { VpnPhase } from "../types";
import logoUrl from "../assets/logo.svg";

/** `nolink` is not a phase of the tunnel — there is nothing to turn on yet. */
export type ShieldState = VpnPhase | "nolink";

interface RingLook {
  /** `stroke-dasharray` in the pinned 0..100 space of `pathLength`. */
  dash?: string;
  width: number;
  colour: string;
  /** Turns slowly — only where something is actually happening. */
  spinning?: boolean;
}

function ringFor(state: ShieldState): RingLook | null {
  switch (state) {
    case "off":
      return null;
    case "nolink":
      return { dash: "0.6 3.4", width: 3, colour: "var(--text-faint)" };
    case "starting":
    case "healing":
      return { dash: "20 80", width: 5, colour: "var(--accent)", spinning: true };
    case "on":
      return { width: 5, colour: "var(--accent)" };
    case "unconfirmed":
      return { dash: "5 5", width: 5, colour: "var(--accent)" };
    case "failed":
      // One clean gap: a break you can see at a glance, and the only ring that
      // is neither whole nor evenly broken.
      return { dash: "84 16", width: 5, colour: "var(--text-dim)" };
  }
}

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
  const ring = ringFor(state);

  return (
    <button
      type="button"
      className="emblem-btn"
      data-state={state}
      onClick={onClick}
      disabled={disabled || !onClick}
      aria-label={label}
    >
      <svg className="emblem-ring" viewBox="0 0 200 200" aria-hidden="true" focusable="false">
        {ring ? (
          <circle
            className={ring.spinning ? "ring-arc spinning" : "ring-arc"}
            cx="100"
            cy="100"
            r="92"
            pathLength={100}
            fill="none"
            stroke={ring.colour}
            strokeWidth={ring.width}
            strokeLinecap="round"
            strokeDasharray={ring.dash}
          />
        ) : null}
      </svg>

      <span className="emblem-plate">
        <img className="emblem-mark" src={logoUrl} alt="" draggable={false} />
      </span>
    </button>
  );
}
