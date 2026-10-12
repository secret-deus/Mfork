// Bump every version manifest in lockstep and scaffold the release note.
// The four manifests (package.json, tauri.conf.json, both Cargo.tomls) plus
// the two Cargo.lock self-entries must agree or tauri-action ships a build
// whose about dialog, updater target and service protocol all disagree.
//
//   node scripts/release.mjs 0.1.23
//
// Does not commit: review docs/releases/<v>.md first, then commit and tag.
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const version = process.argv[2];
if (!/^\d+\.\d+\.\d+$/.test(version ?? "")) {
  console.error("usage: node scripts/release.mjs <x.y.z>");
  process.exit(1);
}

// exact occurrence sites only: a stray "version" key elsewhere is never touched
const manifests = [
  ["package.json", /"version": "\d+\.\d+\.\d+"/, `"version": "${version}"`],
  [
    join("src-tauri", "tauri.conf.json"),
    /"version": "\d+\.\d+\.\d+"/,
    `"version": "${version}"`,
  ],
  [
    join("src-tauri", "Cargo.toml"),
    /^version = "\d+\.\d+\.\d+"/m,
    `version = "${version}"`,
  ],
  [
    join("service", "Cargo.toml"),
    /^version = "\d+\.\d+\.\d+"/m,
    `version = "${version}"`,
  ],
];
for (const [file, re, replacement] of manifests) {
  const p = join(root, file);
  const before = readFileSync(p, "utf8");
  if (!re.test(before)) throw new Error(`no version field matched in ${file}`);
  writeFileSync(p, before.replace(re, replacement));
  console.log(`bumped ${file}`);
}

// Cargo.lock: only the workspace crate's own entry, anchored by its name
for (const [lock, name] of [
  [join("src-tauri", "Cargo.lock"), "mfork"],
  [join("service", "Cargo.lock"), "mfork-service"],
]) {
  const p = join(root, lock);
  const before = readFileSync(p, "utf8");
  const re = new RegExp(
    `(name = "${name}"\\r?\\nversion = )"\\d+\\.\\d+\\.\\d+"`,
  );
  if (!re.test(before)) throw new Error(`no self entry for ${name} in ${lock}`);
  writeFileSync(p, before.replace(re, `$1"${version}"`));
  console.log(`bumped ${lock} (${name})`);
}

const note = join(root, "docs", "releases", `${version}.md`);
if (existsSync(note)) {
  console.log(`release note already exists: docs/releases/${version}.md`);
} else {
  writeFileSync(
    note,
    `# Mfork ${version}

- （待补充）

## 升级说明

`,
  );
  console.log(`created docs/releases/${version}.md`);
}
console.log(`
next:
  1. edit docs/releases/${version}.md
  2. git add -A && git commit -m "chore: release Mfork ${version}"
  3. git tag -a v${version} -m "Mfork ${version}" && git push github master v${version}
     (draft release appears on GitHub; publish it to serve latest.json)`);
