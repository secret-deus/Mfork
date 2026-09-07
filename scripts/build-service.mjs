// Builds the elevated service (service/ crate) for the current platform and
// copies the binary into src-tauri/resources/service/ so tauri bundles it.
// Windows: SCM service. macOS: launchd daemon. Linux: systemd unit.
import { execFileSync } from "node:child_process";
import { chmodSync, copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
execFileSync("cargo", ["build", "--release"], {
  cwd: join(root, "service"),
  stdio: "inherit",
});
const bin = process.platform === "win32" ? "mfork-service.exe" : "mfork-service";
const destDir = join(root, "src-tauri", "resources", "service");
mkdirSync(destDir, { recursive: true });
const dest = join(destDir, bin);
copyFileSync(join(root, "service", "target", "release", bin), dest);
if (process.platform !== "win32") chmodSync(dest, 0o755);
console.log(`OK: ${dest}`);
