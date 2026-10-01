// src/distPolicy.ts
//
// The distribution rule itself, with no build-time input, so the Node test
// runner can import it as is (src/dist.ts reads `import.meta.env`, which only
// exists under Vite).
//
// App Store builds carry no purchase link of any kind, on any storefront: the
// owner chose no in-app purchases, and without them guideline 3.1.1 leaves no
// room for a button that sends people to pay elsewhere. The direct DMG keeps
// every link it has today.

export function decidePurchaseLinks(isAppstore: boolean): boolean {
  return !isAppstore;
}
