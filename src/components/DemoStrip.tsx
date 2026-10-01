// src/components/DemoStrip.tsx
//
// Development only. Rendered exclusively when there is no core behind the
// window (`IS_TAURI === false`), so it cannot appear in the app — App.tsx
// gates it, and the mock bridge is the only thing that makes it useful.
//
// It exists so the owner can click through every state without editing the
// address bar by hand. `?mock=<name>` remains the actual switch.
//
// App Store builds never render it, even outside the app: demo content in a
// reviewed bundle is what guideline 2.2 rules out, and App.tsx's gate alone
// would leave that to one line elsewhere.

import { useEffect, useState } from "react";

import { MOCK_SCENARIOS, MOCK_SCENARIO, applyScenario, type MockScenario } from "../bridge";
import { IS_APPSTORE } from "../dist";
import { gazeSource, type GazeReport } from "../gaze";
import { ERROR_ACTION, type ErrorCode } from "../types";
import { useUi } from "./ui";

const CODES = Object.keys(ERROR_ACTION) as ErrorCode[];

/**
 * Что живой взгляд видит на самом деле.
 *
 * Приговор вынесен в заголовок полосы, а не спрятан внутрь: смотреть на него
 * приходится с телефона, где консоли нет, и раскрывать что-то ради одной
 * строки - лишний шаг. Без него «не следит» - это гадание: не спросили,
 * отказали, включено «уменьшить движение», соединение не защищено или глаз
 * просто закрыт, потому что защита стоит. Пять причин с одним симптомом.
 */
function useGazeReport(): GazeReport {
  const [report, setReport] = useState<GazeReport>(() => gazeSource.report());
  useEffect(() => {
    const id = window.setInterval(() => setReport(gazeSource.report()), 500);
    return () => window.clearInterval(id);
  }, []);
  return report;
}

function verdictOf(r: GazeReport): string {
  if (!r.secure) return "нет https, датчик не выдадут";
  if (r.reducedMotion) return "выключен: «уменьшить движение»";
  if (r.listeners === 0) return "глаз закрыт, защита включена";
  if (r.tiltEvents > 0) return `наклоны идут (${r.tiltEvents})`;
  if (r.asked && r.granted === false) return "в доступе отказано";
  if (r.asked) return "спросили, наклонов нет";
  return "коснитесь экрана, чтобы спросить";
}

function GazeDetails({ report }: { report: GazeReport }) {
  const tilt = report.lastTilt
    ? `${report.lastTilt.beta.toFixed(0)}/${report.lastTilt.gamma.toFixed(0)}`
    : "—";
  return (
    <div className="demo-gaze">
      <span>
        secure {String(report.secure)} · запрос нужен {String(report.needsPermission)} ·
        спросили {String(report.asked)} · дали {String(report.granted)} ·
        меньше движения {String(report.reducedMotion)}
      </span>
      <span>
        наклонов {report.tiltEvents} · касаний {report.pointerEvents} · β/γ {tilt} ·
        подписчиков {report.listeners} · источник {report.source} · датчик приложения{" "}
        {String(report.native)}
      </span>
    </div>
  );
}

export default function DemoStrip() {
  if (IS_APPSTORE) return null;
  return <DemoStripBody />;
}

function DemoStripBody() {
  const { t } = useUi();
  const all: MockScenario[] = [...MOCK_SCENARIOS, ...CODES];
  const report = useGazeReport();

  return (
    <details className="demo">
      <summary>
        {t("demo.title")}: {MOCK_SCENARIO} · взгляд: {verdictOf(report)}
      </summary>
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
      <GazeDetails report={report} />
    </details>
  );
}
