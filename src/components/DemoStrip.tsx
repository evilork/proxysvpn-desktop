// src/components/DemoStrip.tsx
//
// Development only. Rendered exclusively when there is no core behind the
// window (`IS_TAURI === false`), so it cannot appear in the app — App.tsx
// gates it, and the mock bridge is the only thing that makes it useful.
//
// It exists so the owner can click through every state without editing the
// address bar by hand. `?mock=<name>` remains the actual switch.

import { useEffect, useState } from "react";

import { MOCK_SCENARIOS, MOCK_SCENARIO, applyScenario, type MockScenario } from "../bridge";
import { gazeSource, type GazeReport } from "../gaze";
import { ERROR_ACTION, type ErrorCode } from "../types";
import { useUi } from "./ui";

const CODES = Object.keys(ERROR_ACTION) as ErrorCode[];

/**
 * Что живой взгляд видит на самом деле.
 *
 * Стоит здесь, а не в консоли, потому что смотреть на это приходится с
 * телефона, где консоли нет. Без него «не следит» - это гадание: разрешения
 * не спросили, в доступе отказали, включено «уменьшить движение» или
 * соединение не защищено. Четыре разные причины с одним симптомом.
 */
function GazeReadout() {
  const [report, setReport] = useState<GazeReport>(() => gazeSource.report());

  useEffect(() => {
    const id = window.setInterval(() => setReport(gazeSource.report()), 500);
    return () => window.clearInterval(id);
  }, []);

  const tilt = report.lastTilt
    ? `${report.lastTilt.beta.toFixed(0)}/${report.lastTilt.gamma.toFixed(0)}`
    : "—";

  // Самое частое и самое неочевидное впереди остальных: Safari отдаёт датчик
  // только в защищённом контексте.
  const verdict = !report.secure
    ? "соединение не защищено, датчик не выдадут"
    : report.reducedMotion
      ? "включено «уменьшить движение»"
      : report.tiltEvents > 0
        ? "датчик работает"
        : report.asked && report.granted === false
          ? "в доступе отказано"
          : report.asked
            ? "спросили, событий пока нет"
            : "ещё не спрашивали, коснитесь экрана";

  return (
    <div className="demo-gaze">
      <b>Взгляд: {verdict}</b>
      <span>
        secure {String(report.secure)} · запрос нужен {String(report.needsPermission)} ·
        спросили {String(report.asked)} · дали {String(report.granted)} ·
        меньше движения {String(report.reducedMotion)}
      </span>
      <span>
        наклонов {report.tiltEvents} · касаний {report.pointerEvents} · β/γ {tilt} ·
        подписчиков {report.listeners} · источник {report.source}
      </span>
    </div>
  );
}

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
      <GazeReadout />
    </details>
  );
}
