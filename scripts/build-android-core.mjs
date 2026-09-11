import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

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
  execFileSync("go", ["build", "-mod=readonly", "-tags", "with_gvisor,cmfa", "-trimpath", "-ldflags", `-s -w -X github.com/metacubex/mihomo/constant.Version=${VERSION}-mfork`, "-o", join(destination, "libmihomo.so"), "."], {
    cwd: source, stdio: "inherit", env: { ...process.env, GOOS: "android", GOARCH: arch, CGO_ENABLED: "0" },
  });
  writeFileSync(join(destination, "libmihomo.VERSION"), `${VERSION}-mfork\n${COMMIT}\ntags=with_gvisor,cmfa\n`);
}
