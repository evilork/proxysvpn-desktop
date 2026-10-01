// src/dist.ts
//
// Which channel this bundle ships through, fixed at build time.
//
// `VITE_DIST=appstore` is set by `tauri.ios.conf.json` for every iOS build
// (all of them go to TestFlight / the App Store) and will be for a future Mac
// App Store build; the direct DMG builds without it. A compile-time constant
// rather than a runtime question on purpose: Vite inlines the value, so the
// App Store bundle drops the branches it can never take — the mock core and
// the demo strip among them.

import { decidePurchaseLinks } from "./distPolicy";

const rawDist: unknown = import.meta.env.VITE_DIST;

export const IS_APPSTORE: boolean = rawDist === "appstore";

export function purchaseLinksAllowed(): boolean {
  return decidePurchaseLinks(IS_APPSTORE);
}
