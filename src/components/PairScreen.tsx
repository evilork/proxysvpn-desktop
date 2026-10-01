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
//
// App Store builds word the same steps as signing in to an account the
// person already has: guideline 3.1.1 names QR codes and keys that unlock
// paid features, and the app ships without in-app purchases, so the screen
// must read as a sign-in, never as entering a key, and never mention the bot
// (it sells top-ups). The mechanics do not change.
//
// Pair code v1 sits under the QR as a second way in: the eight characters the
// cabinet or the bot shows as "Код для приложения", for when the cabinet is
// open on this same phone and there is nothing to point a camera at. A code
// that works ends exactly like a scanned QR - the same "done" line, the same
// pause, the same `onDone` - and the QR keeps polling meanwhile, untouched.

import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ChangeEvent,
  type ClipboardEvent,
} from "react";
import QRCode from "qrcode";

import { bridge, toAppError, type PairSession } from "../bridge";
import { IS_APPSTORE } from "../dist";
import { formatCountdown, type MsgKey } from "../i18n";
import { privacyUrl } from "../legal";
import {
  editPairCode,
  isCompletePairCode,
  normalisePairCode,
  pairCodeProblemKey,
  pastedPairCode,
  rateLimitPauseMs,
  type PairCodeField,
} from "../pairCode";
import { Screen, Spinner, useLatest, useNow, useUi } from "./ui";

const POLL_MS = 2000;
const DONE_PAUSE_MS = 600;

const CODE_FORM_ID = "pair-code-form";
const CODE_ERROR_ID = "pair-code-error";

type Phase = "preparing" | "waiting" | "expired" | "failed" | "linked";

/** The lines that differ between the direct and the App Store build. */
const COPY: Record<"title" | "body" | "paste" | "notOurLink" | "done", MsgKey> = IS_APPSTORE
  ? {
      title: "pair.appstore.title",
      body: "pair.appstore.body",
      paste: "pair.appstore.paste",
      notOurLink: "pair.appstore.notOurLink",
      done: "pair.appstore.done",
    }
  : {
      title: "pair.title",
      body: "pair.body",
      paste: "pair.paste",
      notOurLink: "pair.notOurLink",
      done: "pair.done",
    };

/** The code entry's own two lines: a sign-in in App Store builds, too. */
const CODE_COPY: Record<"open" | "submit", MsgKey> = IS_APPSTORE
  ? { open: "pair.appstore.code.open", submit: "pair.appstore.code.submit" }
  : { open: "pair.code.open", submit: "pair.code.submit" };

export default function PairScreen({
  onDone,
  onClose,
  onSupport,
}: {
  /** Called once a link is stored; the app connects straight away. */
  onDone: () => void;
  /** Absent while this screen is the only thing the person can do. */
  onClose?: () => void;
  /** Opens the build's support address (App.tsx `supportUrl`). */
  onSupport: () => void;
}) {
  const { t, lang, openExternal } = useUi();
  const now = useNow(1000);
  const [phase, setPhase] = useState<Phase>("preparing");
  const [session, setSession] = useState<PairSession | null>(null);
  const [qr, setQr] = useState<string>("");
  const [notice, setNotice] = useState<string | null>(null);
  // Счётчик проб. Видно только в разработке: 22.09.2026 приложение показывало
  // код, телефон отчитывался «отправлено», а ядро не получало НИ ОДНОГО
  // опроса, и понять, крутится ли интервал вообще, было нечем.
  const [polls, setPolls] = useState(0);
  const [lastPoll, setLastPoll] = useState<string>("—");
  const started = useRef(false);
  const doneRef = useLatest(onDone);
  const phaseRef = useLatest(phase);

  // Pair code entry. Closed until asked for: the QR is still the first way in.
  const [codeOpen, setCodeOpen] = useState(false);
  const [codeField, setCodeField] = useState<PairCodeField>({ value: "", caret: 0 });
  const [codeBusy, setCodeBusy] = useState(false);
  const [codeError, setCodeError] = useState<string | null>(null);
  // After a 429 the button rests until the service's Retry-After, at most the
  // minute the sentence asks for.
  const [codePausedUntil, setCodePausedUntil] = useState(0);
  const codeInput = useRef<HTMLInputElement>(null);
  const codeErrorLine = useRef<HTMLParagraphElement>(null);

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
      setPolls((n) => n + 1);
      void bridge
        .pairPoll(session.token)
        .then((status) => {
          if (stopped) return;
          setLastPoll(status);
          if (status === "linked") {
            setPhase("linked");
            window.setTimeout(() => doneRef.current(), DONE_PAUSE_MS);
          } else if (status === "expired") {
            setPhase("expired");
          }
        })
        .catch((err: unknown) => {
          // A single failed poll is not news: the ladder retries on its own
          // and the code is still valid. It is recorded for the dev readout
          // all the same — "fails every time" and "blinked once" look the
          // same to the person and must not look the same to us.
          setLastPoll(`ошибка: ${String(err).slice(0, 60)}`);
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
        setNotice(t(COPY.notOurLink));
        return;
      }
      await bridge.setSubscription(text);
      setPhase("linked");
      window.setTimeout(() => doneRef.current(), DONE_PAUSE_MS);
    } catch (err) {
      const app = toAppError(err);
      setNotice(app.code === "SUB_MALFORMED" ? t(COPY.notOurLink) : t("pair.clipboardFailed"));
    }
  }, [doneRef, t]);

  // The field reformats as it is typed (XXXX-XXXX); put the caret back where
  // the person was typing rather than at the end.
  useLayoutEffect(() => {
    const input = codeInput.current;
    if (!input || document.activeElement !== input) return;
    input.setSelectionRange(codeField.caret, codeField.caret);
  }, [codeField]);

  // A refusal nobody sees is a silence: with the keyboard up the body is
  // short, so bring the line into view (no animation, nothing to switch off
  // for reduced motion).
  useEffect(() => {
    if (codeError !== null) codeErrorLine.current?.scrollIntoView({ block: "nearest" });
  }, [codeError]);

  const onCodeChange = useCallback(
    (event: ChangeEvent<HTMLInputElement>) => {
      const input = event.currentTarget;
      const native = event.nativeEvent;
      const backspace =
        typeof InputEvent !== "undefined" &&
        native instanceof InputEvent &&
        native.inputType === "deleteContentBackward";
      const caret = input.selectionStart ?? input.value.length;
      setCodeField(editPairCode(codeField.value, input.value, caret, backspace));
      setCodeError(null);
    },
    [codeField.value],
  );

  const onCodePaste = useCallback((event: ClipboardEvent<HTMLInputElement>) => {
    // A whole code replaces whatever the field held; anything else is pasted
    // as usual and tidied by onCodeChange.
    const whole = pastedPairCode(event.clipboardData.getData("text"));
    if (whole === null) return;
    event.preventDefault();
    setCodeField({ value: whole, caret: whole.length });
    setCodeError(null);
  }, []);

  const codeResting = now < codePausedUntil;
  const canRedeem = isCompletePairCode(codeField.value) && !codeBusy && !codeResting;

  const redeem = useCallback(async () => {
    const code = normalisePairCode(codeField.value);
    if (code === null || codeBusy || Date.now() < codePausedUntil) return;
    setCodeBusy(true);
    setCodeError(null);
    try {
      await bridge.redeemPairCode(code);
      // The QR may have got there first; it is the same link, and onDone
      // connects, so it must run once.
      if (phaseRef.current === "linked") return;
      setPhase("linked");
      window.setTimeout(() => doneRef.current(), DONE_PAUSE_MS);
    } catch (err) {
      const app = toAppError(err);
      setCodeError(t(pairCodeProblemKey(app.code, IS_APPSTORE)));
      if (app.code === "PAIR_RATE_LIMITED") {
        setCodePausedUntil(Date.now() + rateLimitPauseMs(app));
      }
    } finally {
      setCodeBusy(false);
    }
  }, [codeField.value, codeBusy, codePausedUntil, doneRef, phaseRef, t]);

  const body = () => {
    if (phase === "linked") {
      return (
        <div className="section" style={{ alignItems: "center", textAlign: "center" }}>
          <p className="h2 accent">{t(COPY.done)}</p>
        </div>
      );
    }

    return (
      <>
        <p className="body dim">{t(COPY.body)}</p>

        {/* The card keeps its size while the code is being made, so the
            layout does not jump under the person's eyes. */}
        <div className="qr-card">
          {qr ? (
            <img src={qr} alt={t("pair.qrAlt")} width={220} height={220} />
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
              {import.meta.env.DEV ? (
                <p className="small faint" style={{ marginTop: 4 }}>
                  опрошено {polls} · последний ответ: {lastPoll}
                </p>
              ) : null}
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

        {/* The second way in. In the scrolling body, not the pinned footer,
            for the same reason as the links below; and it stays usable
            whatever the QR is doing - a QR that failed to load is exactly
            when a code helps. */}
        <div className="code-entry">
          <button
            type="button"
            className="btn btn-quiet"
            aria-expanded={codeOpen}
            aria-controls={CODE_FORM_ID}
            onClick={() => setCodeOpen((open) => !open)}
          >
            {t(CODE_COPY.open)}
          </button>
          {codeOpen ? (
            <form
              id={CODE_FORM_ID}
              className="code-form"
              noValidate
              onSubmit={(event) => {
                event.preventDefault();
                void redeem();
              }}
            >
              <input
                ref={codeInput}
                type="text"
                className="code-field"
                value={codeField.value}
                placeholder={t("pair.code.placeholder")}
                aria-label={t("pair.code.label")}
                aria-invalid={codeError !== null}
                aria-describedby={codeError !== null ? CODE_ERROR_ID : undefined}
                // Opened by a tap, so the keyboard may come up with it.
                autoFocus
                autoComplete="off"
                autoCapitalize="characters"
                autoCorrect="off"
                spellCheck={false}
                enterKeyHint="go"
                // Read-only rather than disabled while busy: disabling drops
                // the focus, and with it the keyboard, before a wrong code
                // can be corrected.
                readOnly={codeBusy}
                onChange={onCodeChange}
                onPaste={onCodePaste}
              />
              {/* Under the field it is about, above the button: below the
                  button it fell under the fold on a phone. */}
              {codeError !== null ? (
                <p
                  ref={codeErrorLine}
                  id={CODE_ERROR_ID}
                  className="small danger code-error"
                  role="alert"
                >
                  {codeError}
                </p>
              ) : null}
              <button
                type="submit"
                className="btn btn-primary"
                disabled={!canRedeem}
                aria-busy={codeBusy}
              >
                {codeBusy ? <Spinner /> : null}
                {t(CODE_COPY.submit)}
              </button>
            </form>
          ) : null}
        </div>

        {/* Before there is a link this screen is the whole app, and More,
            where support and the policy live, is out of reach. Someone who
            cannot sign in still needs a way to ask (guideline 1.5), and the
            policy must stay reachable in the app (5.1.1(i)). At the end of
            the scrolling body rather than in the pinned footer: at 200 %
            text on a 667 pt phone the footer would have left the code 84 pt. */}
        <div className="link-row link-row--end">
          <button type="button" className="btn btn-quiet" onClick={onSupport}>
            {t("about.support")}
          </button>
          <button
            type="button"
            className="btn btn-quiet"
            onClick={() => openExternal(privacyUrl(lang))}
          >
            {t("notice.privacy")}
          </button>
        </div>
      </>
    );
  };

  return (
    <Screen
      title={t(COPY.title)}
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
              {t(COPY.paste)}
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
