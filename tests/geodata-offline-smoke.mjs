// Real-core smoke check; requires fetch-core.mjs and rustc. No running GUI or
// service is touched. Unreachable download URLs make missing data fail locally.
import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const scratch = mkdtempSync(join(tmpdir(), "mfork-geo-smoke-"));
try {
  const module = join(root, "shared/core_geodata.rs").replaceAll("\\", "/");
  const harness = join(scratch, "seed.rs");
  const executable = join(scratch, process.platform === "win32" ? "seed.exe" : "seed");
  writeFileSync(harness, `#[path = ${JSON.stringify(module)}] mod geodata;
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let result = geodata::prepare_with_bundled(std::path::Path::new(&args[1]),
        &geodata::bundled_sources(std::path::Path::new(&args[2]))).unwrap();
    assert!(!result.cold_start);
}
`);
  execFileSync("rustc", ["--edition=2021", harness, "-o", executable], { stdio: "inherit" });
  const resources = join(root, "src-tauri/resources/mihomo");
  const core = join(resources, process.platform === "win32" ? "mihomo.exe" : "mihomo");
  for (const mode of [false, true]) {
    const data = join(scratch, `data-${mode}`);
    mkdirSync(data);
    execFileSync(executable, [data, resources], { stdio: "inherit" });
    const config = join(data, "config.yaml");
    writeFileSync(config, `geodata-mode: ${mode}
geo-auto-update: false
geox-url:
  geoip: http://127.0.0.1:1/unavailable
  geosite: http://127.0.0.1:1/unavailable
  mmdb: http://127.0.0.1:1/unavailable
  asn: http://127.0.0.1:1/unavailable
rules:
  - GEOSITE,cn,DIRECT
  - GEOIP,CN,DIRECT
  - IP-ASN,13335,DIRECT
  - MATCH,DIRECT
`);
    execFileSync(core, ["-t", "-d", data, "-f", config], { stdio: "inherit", timeout: 30000 });
  }
  console.log("PASS: bundled MMDB, DAT, GeoSite and ASN work with unreachable download URLs");
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
