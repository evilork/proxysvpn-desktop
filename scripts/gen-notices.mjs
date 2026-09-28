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
//     aarch64-apple-ios and aarch64-apple-darwin, walking NORMAL dependencies
//     from the app crate only: build scripts and dev-dependencies do not end
//     up in the binary. Each crate records which of the two builds has it.
//   • npm packages of the window, from `npm ls --omit=dev --all`, with the
//     licence from each package's own package.json.
//   • What neither tool sees: the tunnel engines (Xray-core and libXray in the
//     Apple builds; Xray-core, Hysteria and tun2socks beside the desktop app,
//     fetched by scripts/fetch-binaries.sh) and the Rubik font.
//   • The Go modules inside libXray, when scripts/libxray-notices.json exists
//     (written by scripts/build-libxray.sh with the Apple engine).
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

/** Build targets the notices cover, and the name the window uses for each. */
const CARGO_TARGETS = [
  ["ios", "aarch64-apple-ios"],
  ["macos", "aarch64-apple-darwin"],
];
const ALL_PLATFORMS = ["ios", "macos"];

/**
 * Components no package manager here knows about. Versions are absent where
 * this repository does not pin one (fetch-binaries.sh takes the latest
 * release; the Apple engine's version belongs to build-libxray.sh).
 */
const STATIC_COMPONENTS = [
  {
    kind: "engine",
    name: "Xray-core",
    version: null,
    license: "MPL-2.0",
    source: "https://github.com/XTLS/Xray-core",
    platforms: ["ios", "macos"],
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
    version: null,
    license: "MIT",
    source: "https://github.com/apernet/hysteria",
    copyright: "Copyright 2023 Toby",
    platforms: ["macos"],
  },
  {
    kind: "engine",
    name: "tun2socks",
    version: null,
    license: "MIT",
    source: "https://github.com/xjasonlyu/tun2socks",
    copyright: "Copyright (c) 2019 Jason Lyu",
    platforms: ["macos"],
  },
  {
    kind: "font",
    name: "Rubik",
    version: null,
    license: "OFL-1.1",
    source: "https://github.com/googlefonts/rubik",
    copyright: "Copyright 2015 The Rubik Project Authors (https://github.com/googlefonts/rubik)",
    platforms: ["ios", "macos"],
  },
];

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

function staticComponents() {
  const out = [];
  for (const component of STATIC_COMPONENTS) {
    const license = readLicense(component.license, component.name);
    if (license) out.push({ ...component, ...license, authors: [] });
  }
  return out;
}

function libxrayModules() {
  if (!existsSync(LIBXRAY_NOTICES)) return [];
  const notices = JSON.parse(readFileSync(LIBXRAY_NOTICES, "utf8"));
  if (!Array.isArray(notices.modules)) {
    problem("scripts/libxray-notices.json: no modules list");
    return [];
  }
  const out = [];
  for (const module of notices.modules) {
    const who = `libXray module ${module && module.name}`;
    if (!module || typeof module.name !== "string" || typeof module.source !== "string") {
      problem(`${who}: needs name and source`);
      continue;
    }
    const license = readLicense(module.license, who);
    if (!license) continue;
    out.push({
      kind: "go",
      name: module.name,
      version: typeof module.version === "string" && module.version !== "-" ? module.version : null,
      ...license,
      source: normalRepo(module.source) ?? module.source,
      authors: [],
      platforms: ["ios"],
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
  if (entry.copyright) out.copyright = entry.copyright;
  if (entry.authors && entry.authors.length > 0) out.authors = entry.authors;
  return out;
}

function build() {
  const components = [...staticComponents(), ...cargoComponents(), ...npmComponents(), ...libxrayModules()];
  if (problems.size > 0) {
    console.error(`gen-notices: ${problems.size} problem(s) to fix first:`);
    for (const line of problems) console.error(`  - ${line}`);
    process.exit(1);
  }
  const kindOrder = ["engine", "font", "cargo", "go", "npm"];
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
