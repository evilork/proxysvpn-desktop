// src/components/ReportScreen.tsx
//
// [9] The support report, shown in full before anything leaves the device.
//
// An app that sends something about a person without showing it is no better
// than what they are hiding from. So: the whole text on screen, its size as a
// number so it is visibly not megabytes, and two buttons. There is no
// automatic sending.
//
// App Store builds send the person to the support page instead of the bot
// (the bot sells top-ups; App.tsx picks the address), and say so on the button.

import { useCallback, useEffect, useState } from "react";

import { bridge, type SupportReport } from "../bridge";
import { IS_APPSTORE } from "../dist";
import { formatSize } from "../i18n";
import { Screen, Spinner, useUi } from "./ui";

export default function ReportScreen({
  onClose,
  onOpenBot,
}: {
  onClose: () => void;
  onOpenBot: () => void;
}) {
  const { t, lang, toast } = useUi();
  const [report, setReport] = useState<SupportReport | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let alive = true;
    void bridge
      .buildReport()
      .then((value) => alive && setReport(value))
      .catch(() => alive && setFailed(true));
    return () => {
      alive = false;
    };
  }, []);

  const copy = useCallback(async () => {
    if (!report) return;
    try {
      await bridge.writeClipboard(report.text);
      toast(t("common.copied"));
    } catch {
      // Copying is the only thing this screen promises; when the clipboard
      // refuses, the text is still on screen and selectable.
      toast(t("report.failed"));
    }
  }, [report, t, toast]);

  return (
    <Screen
      title={t("report.title")}
      onClose={onClose}
      closeKind="back"
      footer={
        <div className="btn-row">
          <button
            type="button"
            className="btn btn-primary"
            onClick={() => void copy()}
            disabled={!report}
          >
            {t("common.copy")}
          </button>
          <button type="button" className="btn btn-outline" onClick={onOpenBot}>
            {t(IS_APPSTORE ? "report.appstore.openSupport" : "report.openBot")}
          </button>
        </div>
      }
    >
      <p className="body">{t("report.lead")}</p>
      <p className="small dim">{t("report.privacy")}</p>
      {report ? (
        <p className="small faint nowrap-num">
          {t("report.size", { size: formatSize(t, lang, report.bytes) })}
        </p>
      ) : null}

      {failed ? <p className="body danger">{t("report.failed")}</p> : null}

      {report ? (
        <div className="mono-box" style={{ flex: "1 1 auto", minHeight: 180 }}>
          {report.text}
        </div>
      ) : failed ? null : (
        <p className="body dim">
          <Spinner /> {t("common.loading")}
        </p>
      )}
    </Screen>
  );
}
