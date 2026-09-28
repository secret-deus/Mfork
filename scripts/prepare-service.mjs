// Tauri beforeBuild hook: never bundle a stale elevated helper after IPC changes.
// Mobile platforms use service stubs and must not package a host-system daemon.
import { statSync } from "node:fs";
import { join } from "node:path";
import { geoDirectory, geoNames } from "./fetch-geodata.mjs";

if (!["android", "ios"].includes(process.env.TAURI_ENV_PLATFORM)) {
  for (const name of geoNames) {
    const resource = statSync(join(geoDirectory, name), { throwIfNoEntry: false });
    if (!resource?.isFile() || !resource.size) {
      throw new Error(`Missing bundled ${name}; run node scripts/fetch-geodata.mjs before building`);
    }
  }
  await import("./build-service.mjs");
}
