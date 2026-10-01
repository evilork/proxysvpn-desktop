// src/components/LiveRegion.tsx
//
// A line only a screen reader hears. [4] and [8] draw their results as a
// tick, a cross or a spinner, which VoiceOver cannot see (the glyphs are
// aria-hidden on purpose: "check mark" read out on every row is noise). This
// says the same thing in words, once, when it changes.
//
// Why the text is set a moment late rather than rendered straight away:
// screen readers announce CHANGES to a live region they already know about.
// A region that arrives in the DOM with its words already inside - the
// recovery sheet opening on its current step - is silently skipped. The same
// delay also swallows a burst: a step that is replaced by the outcome within
// the pause is never read half-way.
//
// The region is emptied first on every change, so a result that comes back
// word for word the same as last time ("Проверить ещё раз", nothing changed)
// is still read out: an unchanged region says nothing.

import { useEffect, useState } from "react";

/** Long enough for VoiceOver to register a freshly mounted region. */
const ANNOUNCE_AFTER_MS = 150;

export default function LiveRegion({ text }: { text: string }) {
  const [spoken, setSpoken] = useState("");

  useEffect(() => {
    setSpoken("");
    const id = window.setTimeout(() => setSpoken(text), ANNOUNCE_AFTER_MS);
    return () => window.clearTimeout(id);
  }, [text]);

  return (
    <div className="visually-hidden" aria-live="polite" aria-atomic="true">
      {spoken}
    </div>
  );
}
