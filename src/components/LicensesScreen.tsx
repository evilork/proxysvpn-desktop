// src/components/LicensesScreen.tsx
//
// "Лицензии открытого ПО": every third-party component this build ships,
// grouped by licence, and for each one the licence text and where its source
// code is. MIT, BSD, Apache and the rest ask for their notice to travel with
// the binary; MPL-2.0 (Xray-core) also asks that people be told where the
// source is, and for the patched iOS Xray-core that includes our patch.
//
// The source is shown as text with a copy button, not as a link: an App Store
// build opens only its allowlisted pages (src/externalUrl.ts), and GitHub is
// not one of them. The notice only has to say where the code is.
//
// The list and the texts load when the screen opens, as separate chunks: a
// few hundred rows and ~50 KB of licence text have no business in the bundle
// every launch parses.
//
// A row of kind "library" is a shared LGPL library of the Linux AppImage
// (scripts/gen-notices.mjs, APPIMAGE_LIBRARIES). The LGPL asks that people
// can put their own version of it in its place, so its page says that it is
// linked at run time and how to swap it, and where the exact source is.

import { Fragment, useCallback, useEffect, useState } from "react";

import { bridge, type AppInfo } from "../bridge";
import { groupByLicense, parseNotices, type NoticeComponent, type NoticeGroup } from "../notices";
import { IconChevron, NavRow, Screen, Spinner, useUi } from "./ui";

/** Loaders for src/assets/licenses/<SPDX id>.txt, one chunk each. */
const LICENSE_TEXTS = import.meta.glob<string>("../assets/licenses/*.txt", {
  query: "?raw",
  import: "default",
});

function loadLicenseText(id: string): Promise<string> {
  const load = LICENSE_TEXTS[`../assets/licenses/${id}.txt`];
  return load ? load() : Promise.reject(new Error(`no bundled text for ${id}`));
}

const INDENT = <span aria-hidden="true" style={{ width: 8, flex: "0 0 auto" }} />;

function title(component: NoticeComponent): string {
  return component.version ? `${component.name} ${component.version}` : component.name;
}

export default function LicensesScreen({
  platform,
  onClose,
}: {
  platform: AppInfo["platform"] | undefined;
  onClose: () => void;
}) {
  const { t } = useUi();
  const [groups, setGroups] = useState<NoticeGroup[] | null>(null);
  const [failed, setFailed] = useState(false);
  const [open, setOpen] = useState<ReadonlySet<string>>(() => new Set());
  const [selected, setSelected] = useState<NoticeComponent | null>(null);

  useEffect(() => {
    let alive = true;
    void import("../assets/third-party-notices.json")
      .then((module) => {
        if (alive) setGroups(groupByLicense(parseNotices(module.default), platform));
      })
      .catch(() => {
        if (alive) setFailed(true);
      });
    return () => {
      alive = false;
    };
  }, [platform]);

  const toggle = useCallback((license: string) => {
    setOpen((previous) => {
      const next = new Set(previous);
      if (next.has(license)) next.delete(license);
      else next.add(license);
      return next;
    });
  }, []);

  if (selected) {
    return <ComponentDetail component={selected} onClose={() => setSelected(null)} />;
  }

  return (
    <Screen title={t("about.licenses")} onClose={onClose} closeKind="back">
      <p className="body dim">{t("lic.intro")}</p>

      {failed ? <p className="body danger">{t("lic.loadFailed")}</p> : null}
      {!failed && groups === null ? (
        <p className="body dim">
          <Spinner /> {t("common.loading")}
        </p>
      ) : null}

      {groups ? (
        <div className="list">
          {groups.map((group) => {
            const expanded = open.has(group.license);
            return (
              <Fragment key={group.license}>
                <button
                  type="button"
                  className="row"
                  aria-expanded={expanded}
                  onClick={() => toggle(group.license)}
                >
                  <span className="row-main">
                    <span className="row-title">{group.license}</span>
                    <span className="row-sub">
                      {t("lic.count", { n: group.components.length })}
                    </span>
                  </span>
                  <span
                    className="row-side"
                    style={{ transform: expanded ? "rotate(90deg)" : undefined }}
                  >
                    <IconChevron />
                  </span>
                </button>
                {expanded
                  ? group.components.map((component) => (
                      <NavRow
                        key={`${component.kind}:${component.name}@${component.version ?? ""}`}
                        lead={INDENT}
                        title={title(component)}
                        sub={
                          component.kind === "engine"
                            ? t("lic.kind.engine")
                            : component.kind === "font"
                              ? t("lic.kind.font")
                              : component.kind === "library"
                                ? t("lic.kind.library")
                                : undefined
                        }
                        onClick={() => setSelected(component)}
                      />
                    ))
                  : null}
              </Fragment>
            );
          })}
        </div>
      ) : null}
    </Screen>
  );
}

function ComponentDetail({
  component,
  onClose,
}: {
  component: NoticeComponent;
  onClose: () => void;
}) {
  const { t, toast } = useUi();
  const [texts, setTexts] = useState<Record<string, string>>({});
  const [textFailed, setTextFailed] = useState(false);

  useEffect(() => {
    let alive = true;
    void Promise.all(
      component.licenseIds.map(async (id) => [id, await loadLicenseText(id)] as const),
    )
      .then((pairs) => {
        if (alive) setTexts(Object.fromEntries(pairs));
      })
      .catch(() => {
        if (alive) setTextFailed(true);
      });
    return () => {
      alive = false;
    };
  }, [component]);

  const copyLink = useCallback(
    (link: string) => {
      void bridge
        .writeClipboard(link)
        .then(() => toast(t("common.copied")))
        .catch(() => toast(t("lic.copyFailed")));
    },
    [t, toast],
  );

  const changes = component.changes;
  const credits = component.copyright ?? (component.authors?.length ? component.authors.join(", ") : null);

  return (
    <Screen title={title(component)} onClose={onClose} closeKind="back">
      <div className="section">
        <span className="caps">{t("lic.license")}</span>
        <p className="body">{component.license}</p>
        {credits ? (
          <p className="small dim" style={{ whiteSpace: "pre-line" }}>
            {credits}
          </p>
        ) : null}
      </div>

      {component.kind === "library" ? (
        <div className="section">
          <span className="caps">{t("lic.kind.library")}</span>
          <p className="body dim">{t("lic.library.note")}</p>
        </div>
      ) : null}

      <div className="section">
        <span className="caps">{t("lic.source")}</span>
        <p className="mono-box" style={{ margin: 0 }}>
          {component.source}
        </p>
        <button type="button" className="btn btn-quiet" onClick={() => copyLink(component.source)}>
          {t("lic.copySource")}
        </button>
      </div>

      {/* A patched copy (the iOS Xray-core): its source is the upstream code
          above plus these changes, and MPL-2.0 asks that both be findable. */}
      {changes ? (
        <div className="section">
          <span className="caps">{t("lic.changes")}</span>
          <p className="mono-box" style={{ margin: 0 }}>
            {changes}
          </p>
          <button type="button" className="btn btn-quiet" onClick={() => copyLink(changes)}>
            {t("lic.copySource")}
          </button>
        </div>
      ) : null}

      {component.licenseIds.map((id) => (
        <div className="section" key={id}>
          <span className="caps">{t("lic.text", { id })}</span>
          {texts[id] !== undefined ? (
            <pre className="mono-box" style={{ margin: 0 }}>
              {texts[id]}
            </pre>
          ) : textFailed ? (
            <p className="body danger">{t("lic.textFailed")}</p>
          ) : (
            <p className="body dim">
              <Spinner /> {t("common.loading")}
            </p>
          )}
        </div>
      ))}
    </Screen>
  );
}
