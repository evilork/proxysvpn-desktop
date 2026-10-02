// src/notices.ts
//
// The third-party notices (src/assets/third-party-notices.json, written by
// scripts/gen-notices.mjs) as the licences screen uses them.
//
// The file is checked on the way in rather than cast: it is generated, and a
// generator bug that drops a field should show as "could not open the list",
// not as a row that reads "undefined".
//
// Pure, with no build-time input: node --test imports it as is.

export type NoticePlatform = "ios" | "macos" | "windows" | "linux";

/**
 * "library": a shared system library the Linux AppImage carries in usr/lib
 * (WebKitGTK, GTK and what they link), loaded at run time and replaceable.
 */
export type NoticeKind = "engine" | "font" | "library" | "cargo" | "go" | "npm";

export interface NoticeComponent {
  kind: NoticeKind;
  name: string;
  /** null where the repository pins no version (engines fetched as "latest"). */
  version: string | null;
  /** Canonical SPDX expression: operands sorted, "/" read as OR. */
  license: string;
  /** Every SPDX id in `license`; each has src/assets/licenses/<id>.txt. */
  licenseIds: string[];
  /** Where the source is, https. */
  source: string;
  /** Which builds ship it. */
  platforms: NoticePlatform[];
  /**
   * Our changes to it, https, where the build carries a patched copy (the iOS
   * Xray-core). MPL-2.0 asks where the source of what ships is: that is
   * `source` plus this.
   */
  changes?: string;
  /** One or more lines, "\n" between them. */
  copyright?: string;
  authors?: string[];
}

export interface NoticeGroup {
  license: string;
  components: NoticeComponent[];
}

const KINDS: ReadonlySet<string> = new Set<NoticeKind>(["engine", "font", "library", "cargo", "go", "npm"]);
const PLATFORMS: ReadonlySet<string> = new Set<NoticePlatform>(["ios", "macos", "windows", "linux"]);

function isKind(value: unknown): value is NoticeKind {
  return typeof value === "string" && KINDS.has(value);
}

function isPlatformList(value: unknown): value is NoticePlatform[] {
  return (
    Array.isArray(value) &&
    value.length > 0 &&
    value.every((item) => typeof item === "string" && PLATFORMS.has(item))
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item) => typeof item === "string");
}

function readComponent(value: unknown, index: number): NoticeComponent {
  const fail = (field: string): never => {
    throw new Error(`third-party notice #${index}: bad ${field}`);
  };
  if (!isRecord(value)) return fail("entry");
  const { kind, name, version, license, licenseIds, source, platforms, changes, copyright, authors } = value;
  if (!isKind(kind)) return fail("kind");
  if (typeof name !== "string" || name === "") return fail("name");
  if (version !== null && typeof version !== "string") return fail("version");
  if (typeof license !== "string" || license === "") return fail("license");
  if (!isStringArray(licenseIds) || licenseIds.length === 0) return fail("licenseIds");
  if (typeof source !== "string" || !source.startsWith("https://")) return fail("source");
  if (!isPlatformList(platforms)) return fail("platforms");
  if (changes !== undefined && (typeof changes !== "string" || !changes.startsWith("https://"))) {
    return fail("changes");
  }
  if (copyright !== undefined && typeof copyright !== "string") return fail("copyright");
  if (authors !== undefined && !isStringArray(authors)) return fail("authors");
  return {
    kind,
    name,
    version,
    license,
    licenseIds,
    source,
    platforms,
    ...(changes !== undefined ? { changes } : {}),
    ...(copyright !== undefined ? { copyright } : {}),
    ...(authors !== undefined ? { authors } : {}),
  };
}

/** The notices document, checked. Throws on anything malformed. */
export function parseNotices(value: unknown): NoticeComponent[] {
  if (!isRecord(value) || !Array.isArray(value.components)) {
    throw new Error("third-party notices: no components list");
  }
  return value.components.map(readComponent);
}

/**
 * Groups by licence, for the build this is: an iOS screen does not list what
 * only the desktop apps carry, a Windows screen lists the crates only the
 * Windows build links (windows, webview2-com, …) and not the iOS engine, and
 * so on. An unknown platform lists everything — more than needed is fine,
 * less is not.
 *
 * Largest group first (that is where most people look for "MIT"), then by
 * name; components by name. O(n log n).
 */
export function groupByLicense(
  components: readonly NoticeComponent[],
  platform: string | undefined,
): NoticeGroup[] {
  const shown =
    platform !== undefined && PLATFORMS.has(platform)
      ? components.filter((component) => component.platforms.includes(platform as NoticePlatform))
      : components;
  const groups = new Map<string, NoticeComponent[]>();
  for (const component of shown) {
    const list = groups.get(component.license);
    if (list) list.push(component);
    else groups.set(component.license, [component]);
  }
  const byName = (a: NoticeComponent, b: NoticeComponent) =>
    a.name.localeCompare(b.name, "en") || (a.version ?? "").localeCompare(b.version ?? "", "en");
  return [...groups.entries()]
    .map(([license, list]) => ({ license, components: [...list].sort(byName) }))
    .sort(
      (a, b) =>
        b.components.length - a.components.length || a.license.localeCompare(b.license, "en"),
    );
}
