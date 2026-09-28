// Build a native Ubuntu .deb without requiring the release signing key.
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
if (process.platform !== "linux") throw new Error("Run this build on Ubuntu Linux.");
const read = (path) => readFileSync(join(root, path), "utf8");
const version = JSON.parse(read("package.json")).version;
const versions = [
  JSON.parse(read("src-tauri/tauri.conf.json")).version,
  ...["src-tauri/Cargo.toml", "service/Cargo.toml"].map((path) =>
    read(path).match(/^version\s*=\s*"([^"]+)"/m)?.[1]),
];
if (versions.some((v) => v !== version)) {
  throw new Error(`Version mismatch: package.json=${version}, Tauri/app/service=${versions.join("/")}`);
}
console.log(`Building Mfork ${version} for Ubuntu (${process.arch})`);
const run = (bin, args) => execFileSync(bin, args, { cwd: root, stdio: "inherit" });
for (const bin of ["cargo", "rustc", "pnpm", "pkg-config"]) run(bin, ["--version"]);
run("pkg-config", ["--exists", "gtk+-3.0", "webkit2gtk-4.1", "ayatana-appindicator3-0.1", "openssl"]);
run(process.execPath, [join(root, "scripts/fetch-core.mjs")]);
run(process.execPath, [join(root, "scripts/build-service.mjs")]);
run("pnpm", ["exec", "tauri", "build", "--config", "src-tauri/tauri.ubuntu.conf.json", "--bundles", "deb", "--", "--locked"]);
console.log("Ubuntu package: src-tauri/target/release/bundle/deb/");
