// Builds the elevated Windows service (service/ crate) and copies the exe
// into src-tauri/resources/service/ so tauri bundles it. Windows-only.
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

if (process.platform !== "win32") {
  console.log("mfork-service is Windows-only; skipping");
  process.exit(0);
}

const root = dirname(dirname(fileURLToPath(import.meta.url)));
execFileSync("cargo", ["build", "--release"], {
  cwd: join(root, "service"),
  stdio: "inherit",
});
const destDir = join(root, "src-tauri", "resources", "service");
mkdirSync(destDir, { recursive: true });
copyFileSync(
  join(root, "service", "target", "release", "mfork-service.exe"),
  join(destDir, "mfork-service.exe"),
);
console.log(`OK: ${join(destDir, "mfork-service.exe")}`);
