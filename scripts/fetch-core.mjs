// Downloads the latest stable mihomo core for the current OS/arch from the
// official MetaCubeX/mihomo GitHub releases into src-tauri/resources/mihomo/.
// Cross-platform: Windows / macOS / Linux, x64 / arm64.
//
// `--android [arm64|amd64]` builds the pinned, non-root Android core and places it
// as gen/android/app/src/main/jniLibs/<abi>/libmihomo.so, the only location
// an app may execute a binary from on Android 10+ (see docs/ANDROID.md).
import { execFileSync } from "node:child_process";
import { chmodSync, mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync, copyFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { gunzipSync } from "node:zlib";
import { buildAndroidCore } from "./build-android-core.mjs";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
// always use the Windows-bundled bsdtar: a GNU tar earlier in PATH (e.g.
// Git Bash) treats "C:" in paths as a remote host and fails
const tar = process.platform === "win32"
  ? join(process.env.SystemRoot ?? "C:\\Windows", "System32", "tar.exe")
  : "tar";
const destDir = join(root, "src-tauri", "resources", "mihomo");
mkdirSync(destDir, { recursive: true });

const androidIdx = process.argv.indexOf("--android");
const android = androidIdx !== -1;
const os = android ? "android" : { win32: "windows", darwin: "darwin", linux: "linux" }[process.platform];
const arch = android
  ? (process.argv[androidIdx + 1] ?? "arm64")
  : { x64: "amd64", arm64: "arm64" }[process.arch];
if (!os || !arch) throw new Error(`unsupported platform: ${process.platform}/${process.arch}`);
const ext = os === "windows" ? "zip" : "gz";
// the Android arm64 build is published as "android-arm64-v8"
const assetArch = android && arch === "arm64" ? "arm64-v8" : arch;
const assetRe = new RegExp(`^mihomo-${os}-${assetArch}-v[\\d.]+\\.${ext}$`);

const gh = android ? null : await fetch("https://api.github.com/repos/MetaCubeX/mihomo/releases/latest", {
  headers: { "user-agent": "mfork-build" },
});
if (gh && !gh.ok) throw new Error(`GitHub API: HTTP ${gh.status}`);
const release = gh ? await gh.json() : null;
const asset = android ? null : release.assets.find((a) => assetRe.test(a.name));
if (!android && !asset) throw new Error(`no asset matching ${assetRe} in ${release.tag_name}`);

if (android) {
  const abi = { arm64: "arm64-v8a", amd64: "x86_64" }[arch];
  if (!abi) throw new Error(`unsupported android arch: ${arch}`);
  const jni = join(root, "src-tauri", "gen", "android", "app", "src", "main", "jniLibs", abi);
  mkdirSync(jni, { recursive: true });
  buildAndroidCore(arch, jni);
  console.log(`OK: Android app core built with cmfa at ${jni}`);

  // Geo databases ship inside the APK (as every Android client does):
  // the core would otherwise try to download them from GitHub on first
  // start, which is slow or blocked on many phones. They ride along as
  // "lib*.so" so the package manager extracts them next to libmihomo.so;
  // core.rs copies them into the working dir on first start.
  const GEO = "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest";
  for (const [remote, local] of [["geoip.metadb", "libgeoip_metadb.so"], ["geosite.dat", "libgeosite_dat.so"]]) {
    console.log(`Downloading ${remote}...`);
    const r = await fetch(`${GEO}/${remote}`);
    if (!r.ok) throw new Error(`${remote}: HTTP ${r.status}`);
    const g = Buffer.from(await r.arrayBuffer());
    writeFileSync(join(jni, local), g);
    console.log(`OK: ${join(jni, local)} (${(g.length / 1048576).toFixed(1)} MB)`);
  }
  process.exit(0);
}
console.log(`Downloading ${asset.name} (${(asset.size / 1048576).toFixed(1)} MB) from ${release.tag_name}...`);
const download = await fetch(asset.browser_download_url);
if (!download.ok) throw new Error(`Download ${asset.name}: HTTP ${download.status}`);
const buf = Buffer.from(await download.arrayBuffer());

const binName = os === "windows" ? "mihomo.exe" : "mihomo";
const dest = join(destDir, binName);

if (ext === "gz") {
  writeFileSync(dest, gunzipSync(buf));
} else {
  // Windows 10+ ships bsdtar, which extracts zip archives
  const tmp = mkdtempSync(join(tmpdir(), "mihomo-"));
  const zip = join(tmp, asset.name);
  writeFileSync(zip, buf);
  execFileSync(tar, ["-xf", zip, "-C", tmp]);
  const exe = readdirSync(tmp).find((f) => f.endsWith(".exe"));
  if (!exe) throw new Error("no .exe found in archive");
  copyFileSync(join(tmp, exe), dest);
  rmSync(tmp, { recursive: true, force: true });
}
if (os !== "windows") chmodSync(dest, 0o755);
writeFileSync(join(destDir, "VERSION"), release.tag_name + "\n");
console.log(`OK: ${dest} (${release.tag_name})`);

// TUN mode on Windows needs wintun.dll next to mihomo.exe
if (os === "windows") {
  const WINTUN = "https://www.wintun.net/builds/wintun-0.14.1.zip";
  const dllArch = arch === "arm64" ? "arm64" : "amd64";
  console.log(`Downloading wintun.dll (${dllArch})...`);
  const wbuf = Buffer.from(await (await fetch(WINTUN)).arrayBuffer());
  const tmp = mkdtempSync(join(tmpdir(), "wintun-"));
  const zip = join(tmp, "wintun.zip");
  writeFileSync(zip, wbuf);
  execFileSync(tar, ["-xf", zip, "-C", tmp]);
  copyFileSync(join(tmp, "wintun", "bin", dllArch, "wintun.dll"), join(destDir, "wintun.dll"));
  rmSync(tmp, { recursive: true, force: true });
  console.log(`OK: ${join(destDir, "wintun.dll")}`);
}
