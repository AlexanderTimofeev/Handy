# Package an existing Handy Windows release build as an NSIS installer + portable ZIP.
# Run `bun run tauri build` first so Tauri has produced the release artifacts.
[CmdletBinding()]
param(
    [string]$OutputDir = (Join-Path (Resolve-Path "$PSScriptRoot\..") "dist\own-build")
)

$ErrorActionPreference = "Stop"

$projectRoot = (Resolve-Path "$PSScriptRoot\..").Path
$tauriDir = Join-Path $projectRoot "src-tauri"
$releaseDir = Join-Path $tauriDir "target\release"
$bundleNsisDir = Join-Path $releaseDir "bundle\nsis"
$runtimeDllDir = Join-Path $tauriDir "transcribe-libs"
$tauriConfPath = Join-Path $tauriDir "tauri.conf.json"

if (-not (Test-Path $tauriConfPath)) {
    throw "Could not find Tauri config: $tauriConfPath"
}

$tauriConf = Get-Content -Raw -Path $tauriConfPath | ConvertFrom-Json
$version = [string]$tauriConf.version
$productName = [string]$tauriConf.productName

$exePath = Join-Path $releaseDir "handy.exe"
$resourcesPath = Join-Path $tauriDir "resources"
$onnxRuntimePath = Join-Path $runtimeDllDir "onnxruntime.dll"

if (-not (Test-Path $exePath)) {
    throw "Release executable missing: $exePath. Run 'bun run tauri build' first."
}
if (-not (Test-Path $bundleNsisDir)) {
    throw "NSIS bundle directory missing: $bundleNsisDir. Run 'bun run tauri build' first."
}
if (-not (Test-Path $onnxRuntimePath)) {
    throw "Windows runtime dependency missing: $onnxRuntimePath. The current build must stage onnxruntime.dll before packaging."
}

$installer = Get-ChildItem -LiteralPath $bundleNsisDir -Filter "*.exe" -File |
    Sort-Object LastWriteTimeUtc -Descending |
    Select-Object -First 1
if (-not $installer) {
    throw "No NSIS installer found in $bundleNsisDir. Run 'bun run tauri build' first."
}

$resolvedOutputDir = [System.IO.Path]::GetFullPath($OutputDir)
New-Item -ItemType Directory -Path $resolvedOutputDir -Force | Out-Null

$installerName = "${productName}_${version}_x64-setup.exe"
$targetInstaller = Join-Path $resolvedOutputDir $installerName
Copy-Item -LiteralPath $installer.FullName -Destination $targetInstaller -Force

$tempPortable = Join-Path $resolvedOutputDir ".handy-portable-temp"
if (Test-Path $tempPortable) {
    Remove-Item -LiteralPath $tempPortable -Recurse -Force
}
New-Item -ItemType Directory -Path $tempPortable -Force | Out-Null

try {
    Copy-Item -LiteralPath $exePath -Destination $tempPortable -Force

    if (Test-Path $resourcesPath) {
        Copy-Item -LiteralPath $resourcesPath -Destination (Join-Path $tempPortable "resources") -Recurse -Force
    }

    $runtimeDlls = Get-ChildItem -LiteralPath $runtimeDllDir -Filter "*.dll" -File
    if (-not $runtimeDlls) {
        throw "No staged Windows runtime DLLs found in $runtimeDllDir"
    }
    foreach ($dll in $runtimeDlls) {
        Copy-Item -LiteralPath $dll.FullName -Destination $tempPortable -Force
    }

    $zipPath = Join-Path $resolvedOutputDir "${productName}_${version}_Portable.zip"
    if (Test-Path $zipPath) {
        Remove-Item -LiteralPath $zipPath -Force
    }
    Compress-Archive -Path (Join-Path $tempPortable "*") -DestinationPath $zipPath -Force
}
finally {
    if (Test-Path $tempPortable) {
        Remove-Item -LiteralPath $tempPortable -Recurse -Force
    }
}

Write-Host "Packaged Handy artifacts:"
Write-Host "  Installer: $targetInstaller"
Write-Host "  Portable:  $zipPath"
