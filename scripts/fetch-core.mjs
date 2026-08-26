// Downloads the latest stable mihomo core for the current OS/arch from the
// official MetaCubeX/mihomo GitHub releases into src-tauri/resources/mihomo/.
// Cross-platform: Windows / macOS / Linux, x64 / arm64.
import { execFileSync } from "node:child_process";
import { chmodSync, mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync, copyFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { gunzipSync } from "node:zlib";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const destDir = join(root, "src-tauri", "resources", "mihomo");
mkdirSync(destDir, { recursive: true });

const os = { win32: "windows", darwin: "darwin", linux: "linux" }[process.platform];
const arch = { x64: "amd64", arm64: "arm64" }[process.arch];
if (!os || !arch) throw new Error(`unsupported platform: ${process.platform}/${process.arch}`);
const ext = os === "windows" ? "zip" : "gz";
const assetRe = new RegExp(`^mihomo-${os}-${arch}-v[\\d.]+\\.${ext}$`);

const gh = await fetch("https://api.github.com/repos/MetaCubeX/mihomo/releases/latest", {
  headers: { "user-agent": "mfork-build" },
});
if (!gh.ok) throw new Error(`GitHub API: HTTP ${gh.status}`);
const release = await gh.json();
const asset = release.assets.find((a) => assetRe.test(a.name));
if (!asset) throw new Error(`no asset matching ${assetRe} in ${release.tag_name}`);

console.log(`Downloading ${asset.name} (${(asset.size / 1048576).toFixed(1)} MB) from ${release.tag_name}...`);
const buf = Buffer.from(await (await fetch(asset.browser_download_url)).arrayBuffer());

const binName = os === "windows" ? "mihomo.exe" : "mihomo";
const dest = join(destDir, binName);

if (ext === "gz") {
  writeFileSync(dest, gunzipSync(buf));
} else {
  // Windows 10+ ships bsdtar, which extracts zip archives
  const tmp = mkdtempSync(join(tmpdir(), "mihomo-"));
  const zip = join(tmp, asset.name);
  writeFileSync(zip, buf);
  execFileSync("tar", ["-xf", zip, "-C", tmp]);
  const exe = readdirSync(tmp).find((f) => f.endsWith(".exe"));
  if (!exe) throw new Error("no .exe found in archive");
  copyFileSync(join(tmp, exe), dest);
  rmSync(tmp, { recursive: true, force: true });
}
if (os !== "windows") chmodSync(dest, 0o755);
writeFileSync(join(destDir, "VERSION"), release.tag_name + "\n");
console.log(`OK: ${dest} (${release.tag_name})`);
