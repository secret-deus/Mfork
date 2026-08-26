# Downloads the latest stable mihomo core (windows-amd64) from the official
# MetaCubeX/mihomo GitHub releases into src-tauri/resources/mihomo/mihomo.exe
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$destDir = Join-Path $root "src-tauri\resources\mihomo"
New-Item -ItemType Directory -Force $destDir | Out-Null

$release = Invoke-RestMethod "https://api.github.com/repos/MetaCubeX/mihomo/releases/latest"
$asset = $release.assets | Where-Object { $_.name -match "^mihomo-windows-amd64-v[\d\.]+\.zip$" } | Select-Object -First 1
if ($null -eq $asset) { throw "no windows-amd64 asset found in release $($release.tag_name)" }

Write-Host "Downloading $($asset.name) ($([math]::Round($asset.size/1MB,1)) MB) from $($release.tag_name)..."
$zip = Join-Path $env:TEMP $asset.name
Invoke-WebRequest $asset.browser_download_url -OutFile $zip

$tmp = Join-Path $env:TEMP "mihomo-extract"
if (Test-Path $tmp) { Remove-Item -Recurse -Force $tmp }
Expand-Archive $zip -DestinationPath $tmp
$exe = Get-ChildItem $tmp -Filter "*.exe" -Recurse | Select-Object -First 1
Copy-Item $exe.FullName (Join-Path $destDir "mihomo.exe") -Force
Set-Content (Join-Path $destDir "VERSION") $release.tag_name -Encoding utf8
Remove-Item $zip; Remove-Item -Recurse -Force $tmp
Write-Host "OK: $destDir\mihomo.exe ($($release.tag_name))"
