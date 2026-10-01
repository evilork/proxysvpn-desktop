// src/escapeStack.ts
//
// Esc closes the topmost layer only.
//
// Every Screen and Sheet used to add its own window keydown listener and call
// `stopPropagation`, which does not stop other listeners on the same target:
// Esc on a sheet opened over a screen ran both, so cancelling "Delete
// account" also popped the whole More screen. One listener now asks this
// stack, and only the layer registered last answers.
//
// Pure, with no DOM: node --test imports it as is.

export interface EscapeStack {
  /** Register a layer; returns the function that removes it again. */
  push(close: () => void): () => void;
  /** Handle one key; true when a layer took it. */
  handle(key: string): boolean;
}

export function createEscapeStack(): EscapeStack {
  const layers: { close: () => void }[] = [];
  return {
    push(close) {
      const layer = { close };
      layers.push(layer);
      return () => {
        const at = layers.lastIndexOf(layer);
        if (at >= 0) layers.splice(at, 1);
      };
    },
    handle(key) {
      if (key !== "Escape") return false;
      const top = layers[layers.length - 1];
      if (!top) return false;
      top.close();
      return true;
    },
  };
}
