// src/components/PairScreen.tsx
//
// [1] Adding the subscription — the heaviest step for an older person, and
// the one we can remove almost entirely.
//
// The code on screen is a PAIRING token (32 hex characters, alive for five
// minutes), not the subscription link: the cabinet's scanner already accepts
// exactly `^[a-f0-9]{32}$` and posts the link itself. So the QR that used to
// encode the literal `proxysvpn-pairing-soon` now encodes something real, and
// the placeholder is gone rather than relabelled.
//
// The QR stays black on white in both themes. A dark-mode QR is a QR that
// does not scan, and the phone camera is the whole point of this screen.

import { useCallback, useEffect, useRef, useState } from "react";
import QRCode from "qrcode";

import { bridge, toAppError, type PairSession } from "../bridge";
import { formatCountdown } from "../i18n";
import { Screen, Spinner, useLatest, useNow, useUi } from "./ui";

const POLL_MS = 2000;
const DONE_PAUSE_MS = 600;

type Phase = "preparing" | "waiting" | "expired" | "failed" | "linked";

export default function PairScreen({
  onDone,
  onClose,
}: {
  /** Called once a link is stored; the app connects straight away. */
  onDone: () => void;
  /** Absent while this screen is the only thing the person can do. */
  onClose?: () => void;
}) {
  const { t } = useUi();
  const now = useNow(1000);
  const [phase, setPhase] = useState<Phase>("preparing");
  const [session, setSession] = useState<PairSession | null>(null);
  const [qr, setQr] = useState<string>("");
  const [notice, setNotice] = useState<string | null>(null);
  const started = useRef(false);
  const doneRef = useLatest(onDone);

  const start = useCallback(async () => {
    setPhase("preparing");
    setNotice(null);
    setQr("");
    try {
      const next = await bridge.pairStart();
      setSession(next);
      setPhase("waiting");
      const image = await QRCode.toDataURL(next.token, {
        width: 440,
        margin: 1,
        color: { dark: "#000000", light: "#ffffff" },
      });
      setQr(image);
    } catch {
      // Whatever the code failed on, the person can only do one thing about
      // it, so there is one sentence and one button.
      setPhase("failed");
      setNotice(t("pair.codeFailed"));
    }
  }, [t]);

  useEffect(() => {
    if (started.current) return;
    started.current = true;
    void start();
  }, [start]);

  // Poll for the phone. No spinner: a code on screen is already the "we are
  // waiting" signal, and a second one would just add noise.
  useEffect(() => {
    if (phase !== "waiting" || !session) return;
    let stopped = false;
    const id = window.setInterval(() => {
      void bridge
        .pairPoll(session.token)
        .then((status) => {
          if (stopped) return;
          if (status === "linked") {
            setPhase("linked");
            window.setTimeout(() => doneRef.current(), DONE_PAUSE_MS);
          } else if (status === "expired") {
            setPhase("expired");
          }
        })
        .catch(() => {
          // A single failed poll is not news: the ladder retries on its own
          // and the code is still valid.
        });
    }, POLL_MS);
    return () => {
      stopped = true;
      window.clearInterval(id);
    };
  }, [phase, session, doneRef]);

  // Local expiry, so the countdown never shows 0:00 while still polling.
  useEffect(() => {
    if (phase === "waiting" && session && session.expiresAt <= now) setPhase("expired");
  }, [phase, session, now]);

  const paste = useCallback(async () => {
    setNotice(null);
    try {
      const text = (await bridge.readClipboard()).trim();
      if (!/^https?:\/\/\S+$/i.test(text)) {
        setNotice(t("pair.notOurLink"));
        return;
      }
      await bridge.setSubscription(text);
      setPhase("linked");
      window.setTimeout(() => doneRef.current(), DONE_PAUSE_MS);
    } catch (err) {
      const app = toAppError(err);
      setNotice(app.code === "SUB_MALFORMED" ? t("pair.notOurLink") : t("pair.clipboardFailed"));
    }
  }, [doneRef, t]);

  const body = () => {
    if (phase === "linked") {
      return (
        <div className="section" style={{ alignItems: "center", textAlign: "center" }}>
          <p className="h2 accent">{t("pair.done")}</p>
        </div>
      );
    }

    return (
      <>
        <p className="body dim">{t("pair.body")}</p>

        {/* The card keeps its size while the code is being made, so the
            layout does not jump under the person's eyes. */}
        <div className="qr-card">
          {qr ? (
            <img src={qr} alt="" width={220} height={220} />
          ) : (
            <span className="small" style={{ color: "#5b6672" }}>
              {phase === "failed" ? "—" : t("pair.preparing")}
            </span>
          )}
        </div>

        <div style={{ textAlign: "center" }}>
          {phase === "waiting" && session ? (
            <>
              <p className="body nowrap-num">
                {t("pair.countdown", { time: formatCountdown(session.expiresAt - now) })}
              </p>
              <p className="small dim" style={{ marginTop: 4 }}>
                {t("pair.waiting")}
              </p>
            </>
          ) : null}
          {phase === "preparing" ? (
            <p className="body dim">
              <Spinner /> {t("pair.preparing")}
            </p>
          ) : null}
          {phase === "expired" ? <p className="body">{t("pair.expired")}</p> : null}
          {notice ? <p className="body danger">{notice}</p> : null}
        </div>
      </>
    );
  };

  return (
    <Screen
      title={t("pair.title")}
      onClose={onClose}
      closeKind="close"
      footer={
        phase === "linked" ? null : (
          <>
            {phase === "expired" || phase === "failed" ? (
              <button type="button" className="btn btn-primary" onClick={() => void start()}>
                {phase === "expired" ? t("pair.refresh") : t("common.retry")}
              </button>
            ) : null}
            <button type="button" className="btn btn-quiet" onClick={() => void paste()}>
              {t("pair.paste")}
            </button>
            <p className="small faint" style={{ textAlign: "center" }}>
              {t("pair.keychain")}
            </p>
          </>
        )
      }
    >
      {body()}
    </Screen>
  );
}
