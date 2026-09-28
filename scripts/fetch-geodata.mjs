// Also runnable alone to prepare Geo resources without replacing the core.
import { mkdirSync, renameSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const geoNames = ["geoip.metadb", "geosite.dat", "geoip.dat", "ASN.mmdb"];
export const geoDirectory = join(dirname(dirname(fileURLToPath(import.meta.url))), "src-tauri", "resources", "mihomo");

export async function fetchGeodata(destDir = geoDirectory) {
  mkdirSync(destDir, { recursive: true });
  const base = "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest";
  for (const name of geoNames) {
    console.log(`Downloading ${name}...`);
    // Upstream's asset name differs from mihomo's on-disk cache name.
    const remote = name === "ASN.mmdb" ? "GeoLite2-ASN.mmdb" : name;
    const response = await fetch(`${base}/${remote}`);
    if (!response.ok) throw new Error(`${name}: HTTP ${response.status}`);
    const bytes = Buffer.from(await response.arrayBuffer());
    if (!bytes.length) throw new Error(`${name}: empty database`);
    if ((name.endsWith(".mmdb") || name.endsWith(".metadb")) &&
        !bytes.subarray(-128 * 1024).includes(Buffer.from("abcdef4d61784d696e642e636f6d", "hex"))) {
      throw new Error(`${name}: missing MMDB metadata`);
    }
    const target = join(destDir, name);
    writeFileSync(`${target}.download`, bytes);
    renameSync(`${target}.download`, target);
    console.log(`OK: ${name} (${(bytes.length / 1048576).toFixed(1)} MB)`);
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await fetchGeodata();
}
