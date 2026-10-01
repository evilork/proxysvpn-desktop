// src/latency.ts
//
// The latency a Countries row shows, and when it needs its age next to it.
//
// The core measures every row outside the tunnel: with the VPN on, a plain
// handshake went into our own tunnel and read 1-5 ms for every location
// (src-tauri/src/ping.rs, «The Countries list»). Where it cannot measure
// around the tunnel it measures nothing and keeps the last number taken
// outside it. Such a number is real but old, so it is shown with its age:
// "234 ms · measured 12 minutes ago", never as if it were taken just now.

/**
 * A number older than this was not measured by the round the list just ran
 * (that round takes a few seconds at most), so its age is said out loud.
 */
export const RTT_AGE_SHOWN_AFTER_MS = 60_000;

/**
 * When the row's number was measured, if it is old enough to say so; `null`
 * for a fresh number, a row without one, or a core that sent no time.
 */
export function rttMeasuredAtToShow(
  entry: { rttMs?: number; rttAtMs?: number },
  nowMs: number,
): number | null {
  if (entry.rttMs === undefined || entry.rttAtMs === undefined) return null;
  return nowMs - entry.rttAtMs >= RTT_AGE_SHOWN_AFTER_MS ? entry.rttAtMs : null;
}
