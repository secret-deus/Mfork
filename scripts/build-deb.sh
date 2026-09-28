#!/usr/bin/env bash
# Run in a Linux checkout. CI targets Ubuntu 26.04 amd64.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

if [[ $(uname -s) != Linux ]]; then
  echo 'A native Linux environment is required to build the deb package.' >&2
  exit 1
fi
for command in node pnpm cargo pkg-config; do
  if ! command -v "$command" >/dev/null; then
    echo "Missing build tool: $command" >&2
    exit 1
  fi
done
node -e 'if (+process.versions.node.split(".")[0] < 22) throw new Error("Node.js 22+ required")'
if ! pkg-config --exists webkit2gtk-4.1 gtk+-3.0 ayatana-appindicator3-0.1; then
  echo 'Install the Ubuntu build dependencies first:' >&2
  echo 'sudo apt-get install build-essential pkg-config libssl-dev libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev patchelf' >&2
  exit 1
fi

pnpm install --frozen-lockfile
node scripts/fetch-core.mjs
node tests/geodata-offline-smoke.mjs

# Explicit Linux resources avoid including Windows binaries from a shared tree.
# Local installers do not require the remote updater signing key.
config='{
  "build": {
    "beforeBuildCommand": "node scripts/prepare-service.mjs && node node_modules/typescript/bin/tsc && node node_modules/vite/bin/vite.js build"
  },
  "bundle": {
    "createUpdaterArtifacts": false,
    "resources": [
      "resources/mihomo/mihomo",
      "resources/mihomo/VERSION",
      "resources/mihomo/geoip.metadb",
      "resources/mihomo/geosite.dat",
      "resources/mihomo/geoip.dat",
      "resources/mihomo/ASN.mmdb",
      "resources/service/mfork-service"
    ]
  }
}'
node node_modules/@tauri-apps/cli/tauri.js build --bundles deb --config "$config"

version=$(node -p 'JSON.parse(require("node:fs").readFileSync("package.json", "utf8")).version')
output="releases/$version/linux"
mkdir -p "$output"
shopt -s nullglob
packages=(src-tauri/target/release/bundle/deb/*_"$version"_*.deb)
if [[ ${#packages[@]} -ne 1 ]]; then
  echo "Expected one deb for version $version, found ${#packages[@]}" >&2
  exit 1
fi
dpkg-deb --info "${packages[0]}"
dpkg-deb --contents "${packages[0]}"
cp -- "${packages[0]}" "$output/"
(cd "$output" && sha256sum -- "$(basename "${packages[0]}")" > SHA256SUMS.txt)
echo "Package and checksum: $output"
