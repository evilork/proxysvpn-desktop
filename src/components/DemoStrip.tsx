// src/components/DemoStrip.tsx
//
// Development only. Rendered exclusively when there is no core behind the
// window (`IS_TAURI === false`), so it cannot appear in the app — App.tsx
// gates it, and the mock bridge is the only thing that makes it useful.
//
// It exists so the owner can click through every state without editing the
// address bar by hand. `?mock=<name>` remains the actual switch.

import { MOCK_SCENARIOS, MOCK_SCENARIO, applyScenario, type MockScenario } from "../bridge";
import { ERROR_ACTION, type ErrorCode } from "../types";
import { useUi } from "./ui";

const CODES = Object.keys(ERROR_ACTION) as ErrorCode[];

export default function DemoStrip() {
  const { t } = useUi();
  const all: MockScenario[] = [...MOCK_SCENARIOS, ...CODES];

  return (
    <details className="demo">
      <summary>{t("demo.title")}: {MOCK_SCENARIO}</summary>
      <div className="demo-list">
        <span style={{ maxWidth: 220 }}>{t("demo.hint")}</span>
        {all.map((scenario) => (
          <button
            key={scenario}
            type="button"
            aria-pressed={scenario === MOCK_SCENARIO}
            onClick={() => applyScenario(scenario)}
          >
            ?mock={scenario}
          </button>
        ))}
      </div>
    </details>
  );
}
