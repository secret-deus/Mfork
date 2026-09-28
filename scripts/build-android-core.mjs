import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, rmdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";

// The generic Android release expects privileged access to packages.xml.
// cmfa delegates package filtering to Android VpnService instead.
const VERSION = "v1.19.30";
const COMMIT = "ac017cdd246ce8bd547653d927e7bf77d7ee73d5";
export function buildAndroidCore(arch, destination) {
  if (!["arm64", "amd64"].includes(arch)) throw new Error(`Unsupported Android architecture: ${arch}`);
  const source = process.env.MFORK_CORE_SOURCE ?? join(tmpdir(), `mfork-core-${COMMIT}`);
  if (!existsSync(source)) execFileSync("git", ["clone", "--depth", "1", "--branch", VERSION, "https://github.com/MetaCubeX/mihomo.git", source], { stdio: "inherit" });
  const head = execFileSync("git", ["rev-parse", "HEAD"], { cwd: source, encoding: "utf8" }).trim();
  const dirty = execFileSync("git", ["status", "--porcelain"], { cwd: source, encoding: "utf8" }).trim();
  if (head !== COMMIT || dirty) throw new Error("Android core source must match the clean pinned upstream commit");
  mkdirSync(destination, { recursive: true });
  const original = join(source, "hub", "route", "patch_android.go");
  if (!readFileSync(original, "utf8").includes("SetEmbedMode(true)")) {
    throw new Error("Upstream Android controller initialization changed; review the standalone overlay");
  }
  const replacement = join(dirname(fileURLToPath(import.meta.url)), "android-core", "patch_android.go");
  const overlayDir = mkdtempSync(join(tmpdir(), "mfork-core-overlay-"));
  const overlay = join(overlayDir, "overlay.json");
  writeFileSync(overlay, JSON.stringify({ Replace: { [original]: replacement } }));
  try {
    execFileSync("go", ["build", "-overlay", overlay, "-mod=readonly", "-tags", "with_gvisor,cmfa", "-trimpath", "-ldflags", `-s -w -X github.com/metacubex/mihomo/constant.Version=${VERSION}-mfork`, "-o", join(destination, "libmihomo.so"), "."], {
      cwd: source, stdio: "inherit", env: { ...process.env, GOOS: "android", GOARCH: arch, CGO_ENABLED: "0" },
    });
  } finally {
    rmSync(overlay, { force: true });
    rmdirSync(overlayDir);
  }
  writeFileSync(join(destination, "libmihomo.VERSION"), `${VERSION}-mfork\n${COMMIT}\ntags=with_gvisor,cmfa\noverlay=standalone-controller\n`);
}
