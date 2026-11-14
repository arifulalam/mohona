# Builds the Mohona installer for Windows (run on Windows, in PowerShell, from anywhere).
#
#   .\scripts\build-windows.ps1
#
# It builds the engine and the browser host in release mode, copies them next to the desktop app's
# resources, then builds the app and its installer. The installer ends up in
# app\src-tauri\target\release\bundle\nsis\.
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

Write-Host "1/4 Engine and browser host (release)..."
cargo build --release -p mohona -p mohona-host
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

Write-Host "2/4 Copying the programs for the installer..."
$res = Join-Path $root "app\src-tauri\resources"
New-Item -ItemType Directory -Force $res | Out-Null
Copy-Item "target\release\mohona.exe" $res -Force
Copy-Item "target\release\mohona-host.exe" $res -Force

Write-Host "2b/4 The browser extension (it goes into the installer)..."
Set-Location (Join-Path $root "extension")
npm ci
if ($LASTEXITCODE -ne 0) { throw "npm ci failed (extension)" }
npm run build
if ($LASTEXITCODE -ne 0) { throw "the extension build failed" }
$ext = Join-Path $res "extension"
if (Test-Path $ext) { Remove-Item $ext -Recurse -Force }
New-Item -ItemType Directory -Force $ext | Out-Null
Copy-Item "dist\chromium" (Join-Path $ext "chromium") -Recurse -Force
Copy-Item "dist\firefox" (Join-Path $ext "firefox") -Recurse -Force
Set-Location $root

Write-Host "3/4 App dependencies and checks..."
Set-Location (Join-Path $root "app")
npm ci
if ($LASTEXITCODE -ne 0) { throw "npm ci failed" }
npm run check
if ($LASTEXITCODE -ne 0) { throw "the app's checks failed" }

Write-Host "4/4 The app and its installer (the first time this takes a long while)..."
npm run tauri -- build --config src-tauri/tauri.bundle.conf.json
if ($LASTEXITCODE -ne 0) { throw "tauri build failed" }

$out = Join-Path $root "app\src-tauri\target\release\bundle\nsis"
Write-Host ""
Write-Host "Done. The installer:"
Get-ChildItem $out -Filter *.exe | ForEach-Object { Write-Host "  $($_.FullName)" }
