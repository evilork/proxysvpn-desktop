#!/usr/bin/env node
// scripts/gen-notices.mjs
//
// Builds src/assets/third-party-notices.json: every third-party component the
// app ships, with its licence and where its source lives. The "Лицензии
// открытого ПО" screen (LicensesScreen.tsx) reads it; the licence texts
// themselves are src/assets/licenses/<SPDX id>.txt.
//
//   node scripts/gen-notices.mjs           # write the file
//   node scripts/gen-notices.mjs --check   # exit 1 if the committed file is stale
//
// Run it after any change to Cargo.lock or package-lock.json, and commit the
// result. Not an npm script on purpose: package.json stays as it is.
//
// ── What goes in ─────────────────────────────────────────────────────────────
//   • Rust crates the app links, from `cargo metadata --filter-platform` for
//     aarch64-apple-ios, aarch64-apple-darwin, x86_64-pc-windows-msvc,
//     aarch64-pc-windows-msvc and x86_64-unknown-linux-gnu, walking NORMAL
//     dependencies from the app crate only: build scripts and dev-dependencies
//     do not end up in the binary. Each crate records which of the builds has
//     it; both Windows builds count as "windows", the one licences screen they
//     share.
//   • npm packages of the window, from `npm ls --omit=dev --all`, with the
//     licence from each package's own package.json.
//   • What neither tool sees: the tunnel engines (Xray-core and libXray in the
//     Apple builds; Xray-core, Hysteria and tun2socks beside the desktop app,
//     fetched by scripts/fetch-binaries.sh at the versions in
//     scripts/sidecars.lock), the GPL-3.0-or-later Go modules linked into
//     the stock desktop Xray-core and Hysteria, and the Rubik font. Wintun is
//     not listed here: its own licence file is installed beside wintun.dll.
//   • The LGPL system libraries the Linux AppImage carries (APPIMAGE_LIBRARIES
//     below): linuxdeploy copies WebKitGTK, GTK and what they link from the
//     CI image (Ubuntu 22.04) into the image's usr/lib. They are shared
//     libraries, loaded at run time and replaceable; the licences screen says
//     so for every row of kind "library", and THIRD-PARTY-NOTICES.md says how.
//   • The Go modules inside libXray, when scripts/libxray-notices.json exists
//     (written by scripts/build-libxray.sh with the Apple engine). Two of them
//     ARE the engines above: their pinned version, source and our patch go
//     into the iOS row of Xray-core and libXray instead of a second row named
//     by module path. Our own code in that list (the juju/ratelimit stand-in
//     under scripts/libxray/) is not third-party and is left out.
//
// ── MPL-2.0 and our patch ───────────────────────────────────────────────────
// The iOS Xray-core is built with scripts/libxray/xray-core-no-gpl.patch. MPL
// asks that whoever gets the binary is told where its source is, and here
// that is the upstream commit PLUS the patch: the row carries both (`source`
// and `changes`), and the patch is linked in this repository, which is public.
//
// ── Why it refuses instead of guessing ──────────────────────────────────────
// A component without a licence field, or with a licence whose text is not
// bundled, stops the script with a non-zero exit. An app that ships a notice
// screen with a hole in it is worse than a build that fails and says which
// component to look at. To add a licence text, take it from the SPDX list
// pinned below and name the file by its SPDX id:
//   curl -fsSo src/assets/licenses/<id>.txt \
//     https://raw.githubusercontent.com/spdx/license-list-data/v3.29.0/text/<id>.txt

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const OUT = join(ROOT, "src/assets/third-party-notices.json");
const TEXTS_DIR = join(ROOT, "src/assets/licenses");
const LIBXRAY_NOTICES = join(ROOT, "scripts/libxray-notices.json");
/**
 * Where files of this repository are browsable (README links the same repo).
 * `main` because that is what a shipped build is made from.
 */
const THIS_REPO_FILES = "https://github.com/evilork/proxysvpn-desktop/blob/main";

/**
 * Go module paths of the Apple engine that are listed as engines by name. A
 * module path in this map folds into the static entry instead of its own row.
 */
const ENGINE_MODULES = new Map([
  ["github.com/xtls/xray-core", "Xray-core"],
  ["github.com/xtls/libxray", "libXray"],
]);

/**
 * Build targets the notices cover, and the name the window uses for each. The
 * arm64 Windows installer (02.10.2026) links the windows-targets import
 * libraries for its own CPU (windows_aarch64_msvc, not windows_x86_64_msvc),
 * so its screen would otherwise miss crates it ships.
 */
const CARGO_TARGETS = [
  ["ios", "aarch64-apple-ios"],
  ["macos", "aarch64-apple-darwin"],
  ["windows", "x86_64-pc-windows-msvc"],
  ["windows", "aarch64-pc-windows-msvc"],
  ["linux", "x86_64-unknown-linux-gnu"],
];
const ALL_PLATFORMS = ["ios", "macos", "windows", "linux"];
/** The three desktop builds, which ship the same three engine sidecars. */
const DESKTOP = ["macos", "windows", "linux"];

/**
 * Components no package manager here knows about. Versions are absent where
 * this repository does not pin one (fetch-binaries.sh takes the latest
 * release; the Apple engine's version belongs to build-libxray.sh).
 */
const STATIC_COMPONENTS = [
  {
    kind: "engine",
    name: "Xray-core",
    // The desktop sidecar pinned in scripts/sidecars.lock. The iOS row gets
    // its own version, source and patch from the libXray build.
    version: "26.3.27",
    license: "MPL-2.0",
    source: "https://github.com/XTLS/Xray-core",
    platforms: ["ios", ...DESKTOP],
  },
  {
    kind: "engine",
    name: "libXray",
    version: null,
    license: "MIT",
    source: "https://github.com/XTLS/libXray",
    copyright: "Copyright (c) 2023-2025 XTLS",
    platforms: ["ios"],
  },
  {
    kind: "engine",
    name: "Hysteria",
    version: "2.9.3",
    license: "MIT",
    source: "https://github.com/apernet/hysteria",
    copyright: "Copyright 2023 Toby",
    platforms: DESKTOP,
  },
  {
    kind: "engine",
    name: "tun2socks",
    version: "2.6.0",
    license: "MIT",
    source: "https://github.com/xjasonlyu/tun2socks",
    copyright: "Copyright (c) 2019 Jason Lyu",
    platforms: DESKTOP,
  },
  // GPL-3.0-or-later code statically linked into the stock desktop engines
  // (`go version -m` on the pinned releases). The desktop sidecars are the
  // unmodified upstream releases, so their exact-tag sources are the
  // upstream repositories; the iOS engine is built without these modules
  // (scripts/libxray/xray-core-no-gpl.patch).
  {
    kind: "go",
    name: "github.com/sagernet/sing (in Xray-core)",
    version: "v0.5.1",
    license: "GPL-3.0-or-later",
    source: "https://github.com/SagerNet/sing/tree/v0.5.1",
    copyright: "Copyright (C) 2022 by nekohasekai",
    platforms: DESKTOP,
  },
  {
    kind: "go",
    name: "github.com/sagernet/sing-shadowsocks (in Xray-core)",
    version: "v0.2.7",
    license: "GPL-3.0-or-later",
    source: "https://github.com/SagerNet/sing-shadowsocks/tree/v0.2.7",
    copyright: "Copyright (C) 2022 by nekohasekai",
    platforms: DESKTOP,
  },
  {
    kind: "go",
    name: "github.com/sagernet/sing (in Hysteria)",
    version: "v0.3.2",
    license: "GPL-3.0-or-later",
    source: "https://github.com/SagerNet/sing/tree/v0.3.2",
    copyright: "Copyright (C) 2022 by nekohasekai",
    platforms: DESKTOP,
  },
  {
    kind: "go",
    name: "github.com/apernet/sing-tun (in Hysteria)",
    version: "v0.2.6-0.20250920121535-299f04629986",
    license: "GPL-3.0-or-later",
    source: "https://github.com/apernet/sing-tun/tree/299f04629986",
    copyright: "Copyright (C) 2022 by nekohasekai",
    platforms: DESKTOP,
  },
  {
    kind: "font",
    name: "Rubik",
    version: null,
    license: "OFL-1.1",
    source: "https://github.com/googlefonts/rubik",
    copyright: "Copyright 2015 The Rubik Project Authors (https://github.com/googlefonts/rubik)",
    platforms: ALL_PLATFORMS,
  },
];

/**
 * The LGPL shared libraries in the Linux AppImage's usr/lib: WebKitGTK and
 * GTK 3, and what they link that linuxdeploy does not leave to the system
 * (its excludelist keeps glibc, libstdc++, GL, X11, fontconfig, freetype,
 * harfbuzz, fribidi, libgpg-error and a few more out). Versions are what
 * Ubuntu 22.04 ships when CI builds the image, so none is pinned here: CI
 * writes the exact binary and source package of each, with version, into
 * the image and beside it (scripts/appimage-lgpl-sources.sh,
 * LGPL-SOURCES.txt), and the project's own repository is the `source` below.
 * Licences are the projects'
 * SPDX expressions as Fedora's packages state them; where a project offers a
 * choice, the options whose texts are bundled are listed. Permissively
 * licensed libraries of the image (libxml2, libwebp, pixman, …) are not
 * listed one by one yet, as with the engines' Go modules.
 *
 * Only the AppImage carries them: the .deb depends on the distribution's own
 * packages instead. The screen has one list per platform, so they are listed
 * for "linux" and the row says "AppImage only".
 */
const APPIMAGE_LIBRARIES = [
  ["WebKitGTK (libwebkit2gtk-4.1, libjavascriptcoregtk-4.1)", "LGPL-2.1-only AND BSD-2-Clause", "https://webkitgtk.org/releases/"],
  ["GTK 3 (libgtk-3, libgdk-3)", "LGPL-2.0-or-later", "https://gitlab.gnome.org/GNOME/gtk"],
  ["GLib (libglib-2.0, libgobject-2.0, libgio-2.0, libgmodule-2.0)", "LGPL-2.1-or-later", "https://gitlab.gnome.org/GNOME/glib"],
  ["GDK-Pixbuf (libgdk_pixbuf-2.0 and its loaders)", "LGPL-2.1-or-later", "https://gitlab.gnome.org/GNOME/gdk-pixbuf"],
  ["Pango (libpango-1.0, libpangocairo-1.0, libpangoft2-1.0)", "LGPL-2.0-or-later", "https://gitlab.gnome.org/GNOME/pango"],
  ["cairo (libcairo, libcairo-gobject)", "LGPL-2.1-only OR MPL-1.1", "https://gitlab.freedesktop.org/cairo/cairo"],
  ["ATK (libatk-1.0)", "LGPL-2.0-or-later", "https://gitlab.gnome.org/GNOME/atk"],
  ["AT-SPI (libatspi, libatk-bridge-2.0)", "LGPL-2.1-or-later", "https://gitlab.gnome.org/GNOME/at-spi2-core"],
  ["libsoup (libsoup-3.0)", "LGPL-2.0-or-later AND LGPL-2.1-or-later", "https://gitlab.gnome.org/GNOME/libsoup"],
  ["GStreamer (libgstreamer-1.0, the gst-plugins-base and gst-plugins-bad libraries)", "LGPL-2.1-or-later", "https://gitlab.freedesktop.org/gstreamer/gstreamer"],
  ["librsvg (librsvg-2)", "LGPL-2.1-or-later", "https://gitlab.gnome.org/GNOME/librsvg"],
  ["libsecret (libsecret-1)", "LGPL-2.1-or-later", "https://gitlab.gnome.org/GNOME/libsecret"],
  ["Enchant (libenchant-2)", "LGPL-2.0-or-later", "https://github.com/rrthomas/enchant"],
  // GPL-2.0-only OR LGPL-2.1-or-later OR MPL-1.1: either of the last two
  // covers the copy in the image.
  ["Hyphen (libhyphen)", "LGPL-2.1-or-later OR MPL-1.1", "https://github.com/hunspell/hyphen"],
  ["libmanette (libmanette-0.2)", "LGPL-2.1-or-later", "https://gitlab.gnome.org/GNOME/libmanette"],
  ["libgudev (libgudev-1.0)", "LGPL-2.1-or-later", "https://gitlab.gnome.org/GNOME/libgudev"],
  ["libseccomp", "LGPL-2.1-only", "https://github.com/seccomp/libseccomp"],
  ["libtasn1", "LGPL-2.1-or-later", "https://gitlab.com/gnutls/libtasn1"],
  ["Libgcrypt (libgcrypt)", "LGPL-2.1-or-later", "https://gnupg.org/software/libgcrypt/"],
  ["libsystemd", "LGPL-2.1-or-later", "https://github.com/systemd/systemd"],
  ["libthai", "LGPL-2.1-or-later", "https://github.com/tlwg/libthai"],
  ["libdatrie", "LGPL-2.1-or-later", "https://github.com/tlwg/libdatrie"],
  ["util-linux (libmount, libblkid)", "LGPL-2.1-or-later", "https://github.com/util-linux/util-linux"],
].map(([name, license, source]) => ({
  kind: "library",
  name,
  version: null,
  license,
  source,
  platforms: ["linux"],
}));

/** A Set: a crate both builds link would otherwise be reported twice. */
const problems = new Set();

function problem(message) {
  problems.add(message);
}

function run(command, args, cwd) {
  try {
    return execFileSync(command, args, {
      cwd,
      encoding: "utf8",
      maxBuffer: 256 * 1024 * 1024,
      stdio: ["ignore", "pipe", "pipe"],
    });
  } catch (error) {
    const stderr = error && typeof error === "object" && "stderr" in error ? String(error.stderr) : "";
    throw new Error(`${command} ${args.join(" ")} failed${stderr ? `:\n${stderr.trim()}` : ""}`);
  }
}

// ── SPDX expressions ────────────────────────────────────────────────────────

const AVAILABLE_TEXTS = new Set(
  readdirSync(TEXTS_DIR)
    .filter((file) => file.endsWith(".txt"))
    .map((file) => file.slice(0, -".txt".length)),
);

/**
 * Parses an SPDX licence expression into a tree. Accepts the legacy crates.io
 * spelling "MIT/Apache-2.0" (a slash meant OR) and operators in any case.
 * Returns null for anything it cannot read.
 */
function parseSpdx(raw) {
  const source = raw.replace(/\s*\/\s*/g, " OR ").trim();
  const tokens = source.match(/\(|\)|[A-Za-z0-9.+-]+/g);
  if (!tokens || tokens.join("").replace(/\s/g, "") !== source.replace(/\s/g, "")) return null;
  let at = 0;
  const peek = () => tokens[at];
  const isOp = (token, op) => typeof token === "string" && token.toUpperCase() === op;

  function atom() {
    const token = tokens[at++];
    if (token === undefined || token === ")") return null;
    if (token === "(") {
      const inner = orExpr();
      if (inner === null || tokens[at++] !== ")") return null;
      return inner;
    }
    if (isOp(token, "AND") || isOp(token, "OR") || isOp(token, "WITH")) return null;
    let node = { id: token };
    if (isOp(peek(), "WITH")) {
      at += 1;
      const exception = tokens[at++];
      if (exception === undefined || exception === "(" || exception === ")") return null;
      node = { id: token, exception };
    }
    return node;
  }

  function andExpr() {
    const items = [atom()];
    while (isOp(peek(), "AND")) {
      at += 1;
      items.push(atom());
    }
    if (items.some((item) => item === null)) return null;
    return items.length === 1 ? items[0] : { op: "AND", items };
  }

  function orExpr() {
    const items = [andExpr()];
    while (isOp(peek(), "OR")) {
      at += 1;
      items.push(andExpr());
    }
    if (items.some((item) => item === null)) return null;
    return items.length === 1 ? items[0] : { op: "OR", items };
  }

  const tree = orExpr();
  return tree !== null && at === tokens.length ? tree : null;
}

/**
 * One spelling per meaning: operands of AND and OR are flattened and sorted,
 * so "MIT OR Apache-2.0" and "Apache-2.0/MIT" land in the same group.
 */
function canonical(node) {
  if ("id" in node) return node.exception ? `${node.id} WITH ${node.exception}` : node.id;
  return [...new Set(canonicalParts(node))].sort((a, b) => a.localeCompare(b, "en")).join(` ${node.op} `);
}

/** The operands of `node`, with nested operands of the same operator lifted up. */
function canonicalParts(node) {
  return node.items.flatMap((item) => {
    if (!("op" in item)) return [canonical(item)];
    return item.op === node.op ? canonicalParts(item) : [`(${canonical(item)})`];
  });
}

/** Every licence and exception id the expression names, for the texts. */
function idsOf(node) {
  if ("id" in node) {
    const ids = [node.id.replace(/\+$/, "")];
    if (node.exception) ids.push(node.exception);
    return ids;
  }
  return node.items.flatMap(idsOf);
}

/** { license, licenseIds } or null (with the reason recorded). */
function readLicense(raw, who) {
  if (typeof raw !== "string" || raw.trim() === "") {
    problem(`${who}: no licence field`);
    return null;
  }
  const tree = parseSpdx(raw);
  if (!tree) {
    problem(`${who}: cannot read licence expression "${raw}"`);
    return null;
  }
  const ids = [...new Set(idsOf(tree))].sort();
  const missing = ids.filter((id) => !AVAILABLE_TEXTS.has(id));
  if (missing.length > 0) {
    problem(`${who}: no bundled text for ${missing.join(", ")} (in "${raw}")`);
    return null;
  }
  return { license: canonical(tree), licenseIds: ids };
}

// ── Links and names ─────────────────────────────────────────────────────────

/** A browsable https address for a repository field, or null. */
function normalRepo(value) {
  let raw = value;
  if (raw && typeof raw === "object" && typeof raw.url === "string") raw = raw.url;
  if (typeof raw !== "string" || raw.trim() === "") return null;
  let url = raw.trim();
  if (/^[\w.-]+\/[\w.-]+$/.test(url)) url = `https://github.com/${url}`;
  url = url
    .replace(/^github:/, "https://github.com/")
    .replace(/^git\+/, "")
    .replace(/^git:\/\//, "https://")
    .replace(/^ssh:\/\/git@/, "https://")
    .replace(/^git@([^:]+):/, "https://$1/")
    .replace(/\.git$/, "")
    .replace(/\/$/, "");
  return /^https?:\/\//.test(url) ? url.replace(/^http:/, "https:") : null;
}

/**
 * "Name <mail>", "Name (site)" and "Name mail@host" become "Name": the screen
 * credits people, it does not publish their addresses.
 */
function authorName(value) {
  if (value && typeof value === "object" && typeof value.name === "string") return value.name.trim();
  if (typeof value !== "string") return "";
  return value
    .replace(/<[^>]*>/g, "")
    .replace(/\([^)]*\)/g, "")
    .replace(/\S+@\S+/g, "")
    .replace(/\s+/g, " ")
    .trim();
}

function authorsOf(list) {
  const names = (Array.isArray(list) ? list : [list]).map(authorName).filter((n) => n !== "");
  return [...new Set(names)];
}

// ── Rust ────────────────────────────────────────────────────────────────────

function cargoComponents() {
  const byKey = new Map();
  for (const [platform, target] of CARGO_TARGETS) {
    const meta = JSON.parse(
      run(
        "cargo",
        [
          "metadata",
          "--format-version",
          "1",
          "--filter-platform",
          target,
          "--manifest-path",
          join(ROOT, "src-tauri/Cargo.toml"),
        ],
        ROOT,
      ),
    );
    if (!meta.resolve || !meta.resolve.root) throw new Error(`cargo metadata for ${target}: no resolve graph`);
    const packages = new Map(meta.packages.map((pkg) => [pkg.id, pkg]));
    const nodes = new Map(meta.resolve.nodes.map((node) => [node.id, node]));
    const members = new Set(meta.workspace_members);

    // Normal dependencies only, from the app crate outwards: O(crates + edges).
    const seen = new Set();
    const stack = [meta.resolve.root];
    while (stack.length > 0) {
      const id = stack.pop();
      if (seen.has(id)) continue;
      seen.add(id);
      const node = nodes.get(id);
      if (!node) continue;
      for (const dep of node.deps) {
        if (dep.dep_kinds.some((kind) => kind.kind === null)) stack.push(dep.pkg);
      }
    }

    for (const id of seen) {
      if (members.has(id)) continue;
      const pkg = packages.get(id);
      if (!pkg) continue;
      const key = `${pkg.name}@${pkg.version}`;
      const existing = byKey.get(key);
      if (existing) {
        existing.platforms.add(platform);
        continue;
      }
      const license = readLicense(pkg.license, `crate ${key}`);
      if (!license) continue;
      byKey.set(key, {
        kind: "cargo",
        name: pkg.name,
        version: pkg.version,
        ...license,
        source: normalRepo(pkg.repository) ?? `https://crates.io/crates/${pkg.name}/${pkg.version}`,
        authors: authorsOf(pkg.authors),
        platforms: new Set([platform]),
      });
    }
  }
  return [...byKey.values()].map((entry) => ({ ...entry, platforms: [...entry.platforms].sort() }));
}

// ── npm ─────────────────────────────────────────────────────────────────────

/** Node's own lookup: <dir>/node_modules/<name>, then each parent directory. */
function packageDir(name, fromDir) {
  let dir = fromDir;
  for (;;) {
    const candidate = join(dir, "node_modules", name);
    if (existsSync(join(candidate, "package.json"))) return candidate;
    const parent = dirname(dir);
    if (parent === dir) return null;
    dir = parent;
  }
}

function npmLicense(manifest) {
  if (typeof manifest.license === "string") return manifest.license;
  if (manifest.license && typeof manifest.license === "object" && typeof manifest.license.type === "string") {
    return manifest.license.type;
  }
  // The old `licenses` array meant "any of these".
  if (Array.isArray(manifest.licenses)) {
    const types = manifest.licenses
      .map((item) => (typeof item === "string" ? item : item && item.type))
      .filter((type) => typeof type === "string" && type !== "");
    if (types.length > 0) return types.join(" OR ");
  }
  return undefined;
}

function npmComponents() {
  const tree = JSON.parse(run("npm", ["ls", "--omit=dev", "--all", "--json"], ROOT));
  const byKey = new Map();
  const walk = (deps, fromDir) => {
    for (const [name, node] of Object.entries(deps ?? {})) {
      const dir = packageDir(name, fromDir);
      if (!dir) {
        problem(`npm ${name}: not installed (run npm ci)`);
        continue;
      }
      const manifest = JSON.parse(readFileSync(join(dir, "package.json"), "utf8"));
      const key = `${manifest.name}@${manifest.version}`;
      if (node.version && node.version !== manifest.version) {
        problem(`npm ${name}: npm ls says ${node.version}, ${dir} has ${manifest.version}`);
      }
      if (!byKey.has(key)) {
        const license = readLicense(npmLicense(manifest), `npm ${key}`);
        if (license) {
          byKey.set(key, {
            kind: "npm",
            name: manifest.name,
            version: manifest.version,
            ...license,
            source:
              normalRepo(manifest.repository) ??
              `https://www.npmjs.com/package/${manifest.name}/v/${manifest.version}`,
            authors: authorsOf(manifest.author ?? []),
            platforms: ALL_PLATFORMS,
          });
        }
      }
      walk(node.dependencies, dir);
    }
  };
  walk(tree.dependencies, ROOT);
  return [...byKey.values()];
}

// ── What the package managers do not see ────────────────────────────────────

/** scripts/libxray-notices.json, or null when the Apple engine was never built here. */
function readLibxrayNotices() {
  if (!existsSync(LIBXRAY_NOTICES)) return null;
  const notices = JSON.parse(readFileSync(LIBXRAY_NOTICES, "utf8"));
  if (!notices || !Array.isArray(notices.modules)) {
    problem("scripts/libxray-notices.json: no modules list");
    return null;
  }
  return notices;
}

/**
 * The copyright lines of a Go module's LICENSE and NOTICE, for the row's
 * credits: MIT and BSD ask for "the above copyright notice" to travel with the
 * binary, Apache for the NOTICE, and the bundled SPDX texts carry neither. A
 * line counts when "Copyright" is followed by a year, with or without (c):
 * that leaves out the licences' own clauses ("copyright notice, this list of
 * conditions…") and the Apache appendix template. Addresses are dropped, as
 * they are for authors.
 */
function copyrightOf(module) {
  const text = [module.noticeText, module.licenseText].filter((t) => typeof t === "string").join("\n");
  const lines = text
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => /^copyright\s+(?:\(c\)\s*|©\s*)?\d{4}/i.test(line))
    .map((line) => line.replace(/<[^>]*>/g, "").replace(/\s+/g, " ").replace(/[\s,]+$/, ""));
  const unique = [...new Set(lines)];
  return unique.length > 0 ? unique.join("\n") : undefined;
}

/**
 * `modifications` reads "<path in this repository> (what the patch does)".
 * The path has to exist here, or the link would lead nowhere.
 */
function changesOf(module, who) {
  if (module.modifications === undefined) return undefined;
  const path = typeof module.modifications === "string" ? module.modifications.trim().split(/\s+/)[0] : "";
  if (!path || !existsSync(join(ROOT, path))) {
    problem(`${who}: modifications "${module.modifications}" name no file of this repository`);
    return undefined;
  }
  return `${THIS_REPO_FILES}/${path}`;
}

/**
 * The Go modules of the Apple engine: rows of their own, and the pinned
 * facts about the two that are listed as engines (ENGINE_MODULES).
 */
function libxrayComponents(notices) {
  const engines = new Map();
  const modules = [];
  if (!notices) return { engines, modules };
  for (const module of notices.modules) {
    const who = `libXray module ${module && module.name}`;
    if (!module || typeof module.name !== "string" || typeof module.source !== "string") {
      problem(`${who}: needs name and source`);
      continue;
    }
    // A path of this repository is our own code (scripts/libxray/ratelimit):
    // not a third-party component, and nothing to credit anyone for.
    if (!/^[a-z]+:/i.test(module.source) && existsSync(join(ROOT, module.source))) continue;
    const source = normalRepo(module.source);
    if (!source) {
      problem(`${who}: source "${module.source}" is not a link`);
      continue;
    }
    const license = readLicense(module.license, who);
    if (!license) continue;
    const version = typeof module.version === "string" && module.version !== "-" ? module.version : null;
    const copyright = copyrightOf(module);
    const engine = ENGINE_MODULES.get(module.name);
    if (engine) {
      engines.set(engine, { version, source, license: license.license, copyright, changes: changesOf(module, who) });
      continue;
    }
    modules.push({ kind: "go", name: module.name, version, ...license, source, copyright, authors: [], platforms: ["ios"] });
  }
  for (const name of ENGINE_MODULES.values()) {
    if (!engines.has(name)) problem(`scripts/libxray-notices.json: no module for ${name}`);
  }
  return { engines, modules };
}

/**
 * The fixed list. An engine the libXray build pins gets its iOS row from
 * there (version, source, our patch); the Mac app fetches the latest release,
 * so its row keeps no version.
 */
function staticComponents(engines) {
  const out = [];
  for (const component of [...STATIC_COMPONENTS, ...APPIMAGE_LIBRARIES]) {
    const license = readLicense(component.license, component.name);
    if (!license) continue;
    const pinned = component.platforms.includes("ios") ? engines.get(component.name) : undefined;
    if (!pinned) {
      out.push({ ...component, ...license, authors: [] });
      continue;
    }
    if (pinned.license !== license.license) {
      problem(`${component.name}: libXray's build says ${pinned.license}, this script says ${license.license}`);
      continue;
    }
    const elsewhere = component.platforms.filter((platform) => platform !== "ios");
    if (elsewhere.length > 0) out.push({ ...component, ...license, platforms: elsewhere, authors: [] });
    out.push({
      ...component,
      ...license,
      version: pinned.version,
      source: pinned.source,
      changes: pinned.changes,
      copyright: pinned.copyright ?? component.copyright,
      platforms: ["ios"],
      authors: [],
    });
  }
  return out;
}

// ── Output ──────────────────────────────────────────────────────────────────

/** Fixed key order, and no empty fields, so the file diffs line by line. */
function shape(entry) {
  const out = {
    kind: entry.kind,
    name: entry.name,
    version: entry.version,
    license: entry.license,
    licenseIds: entry.licenseIds,
    source: entry.source,
    platforms: entry.platforms,
  };
  if (entry.changes) out.changes = entry.changes;
  if (entry.copyright) out.copyright = entry.copyright;
  if (entry.authors && entry.authors.length > 0) out.authors = entry.authors;
  return out;
}

function build() {
  const { engines, modules } = libxrayComponents(readLibxrayNotices());
  const components = [...staticComponents(engines), ...cargoComponents(), ...npmComponents(), ...modules];
  if (problems.size > 0) {
    console.error(`gen-notices: ${problems.size} problem(s) to fix first:`);
    for (const line of problems) console.error(`  - ${line}`);
    process.exit(1);
  }
  const kindOrder = ["engine", "font", "library", "cargo", "go", "npm"];
  components.sort(
    (a, b) =>
      kindOrder.indexOf(a.kind) - kindOrder.indexOf(b.kind) ||
      a.name.localeCompare(b.name, "en") ||
      String(a.version).localeCompare(String(b.version), "en"),
  );
  const usedTexts = [...new Set(components.flatMap((c) => c.licenseIds))].sort();
  const unused = [...AVAILABLE_TEXTS].filter((id) => !usedTexts.includes(id)).sort();
  if (unused.length > 0) {
    console.warn(`gen-notices: texts nothing uses (safe to delete): ${unused.join(", ")}`);
  }
  const document = {
    generatedBy: "scripts/gen-notices.mjs",
    licenseTexts: usedTexts,
    components: components.map(shape),
  };
  return `${JSON.stringify(document, null, 2)}\n`;
}

let text;
try {
  text = build();
} catch (error) {
  console.error(`gen-notices: ${error instanceof Error ? error.message : String(error)}`);
  process.exit(1);
}
if (process.argv.includes("--check")) {
  const current = existsSync(OUT) ? readFileSync(OUT, "utf8") : "";
  if (current !== text) {
    console.error("gen-notices: src/assets/third-party-notices.json is stale; run node scripts/gen-notices.mjs");
    process.exit(1);
  }
  console.log("gen-notices: up to date");
} else {
  writeFileSync(OUT, text);
  const count = JSON.parse(text).components.length;
  console.log(`gen-notices: ${count} components -> src/assets/third-party-notices.json`);
}
