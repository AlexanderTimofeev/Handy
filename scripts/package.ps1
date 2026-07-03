# PowerShell Script to Package Handy (Installer + Portable ZIP)
# Usage: powershell -File scripts/package.ps1

$ErrorActionPreference = "Stop"

# 1. Paths and Config
$projectRoot = Resolve-Path "$PSScriptRoot\.."
$releaseDir = "$projectRoot\src-tauri\target\release"
$nsisWorkingDir = "$releaseDir\nsis\x64"
$tauriConfPath = "$projectRoot\src-tauri\tauri.conf.json"

if (-not (Test-Path $tauriConfPath)) {
    Write-Error "Could not find tauri.conf.json at $tauriConfPath"
}

$tauriConf = Get-Content -Raw -Path $tauriConfPath | ConvertFrom-Json
$version = $tauriConf.version
$productName = $tauriConf.productName

Write-Host "Packaging $productName v$version..." -ForegroundColor Cyan

# 2. Verify build artifacts exist
$exePath = "$releaseDir\handy.exe"
$dllPath = "$releaseDir\DirectML.dll"
$resourcesPath = "$releaseDir\resources"

if (-not (Test-Path $exePath)) { Write-Error "Build artifact missing: $exePath. Please run 'cargo build --release' or 'bun run tauri build' first." }
if (-not (Test-Path $dllPath)) { Write-Error "Build dependency missing: $dllPath." }
if (-not (Test-Path $resourcesPath)) { Write-Error "Resources directory missing: $resourcesPath." }

# 3. Locate makensis.exe and plugins
Write-Host "Locating NSIS compiler and plugins..." -ForegroundColor Yellow

$makensisPath = $null
$pluginDir = $null

# Search locations for makensis
$searchPaths = @(
    "$env:LOCALAPPDATA\tauri\NSIS\Bin\makensis.exe",
    "$env:LOCALAPPDATA\Packages\*\LocalCache\Local\tauri\NSIS\Bin\makensis.exe"
)

foreach ($pattern in $searchPaths) {
    $matched = Resolve-Path $pattern -ErrorAction SilentlyContinue
    if ($matched) {
        $makensisPath = $matched[0].Path
        break
    }
}

if (-not $makensisPath) {
    # Broad search in AppData
    $matched = Get-ChildItem -Path "$env:LOCALAPPDATA" -Filter "makensis.exe" -Recurse -ErrorAction SilentlyContinue
    if ($matched) {
        $makensisPath = $matched[0].FullName
    }
}

if (-not $makensisPath) {
    Write-Error "Could not find makensis.exe in AppData. Make sure Tauri's NSIS has been cached by running a build attempt once."
}

# Resolve the corresponding plugin dir (which contains nsis_tauri_utils.dll)
$nsisRoot = Split-Path (Split-Path $makensisPath -Parent) -Parent
$pluginDir = "$nsisRoot\Plugins\x86-unicode\additional"

if (-not (Test-Path "$pluginDir\nsis_tauri_utils.dll")) {
    Write-Error "Could not find nsis_tauri_utils.dll at $pluginDir"
}

Write-Host "Using makensis: $makensisPath" -ForegroundColor Gray
Write-Host "Using plugin path: $pluginDir" -ForegroundColor Gray

# 4. Modify generated installer.nsi to point to actual plugin path
$nsiScriptPath = "$nsisWorkingDir\installer.nsi"
if (-not (Test-Path $nsiScriptPath)) {
    Write-Error "NSIS script not found at $nsiScriptPath. Run 'bun run tauri build' at least once to generate it."
}

Write-Host "Patching generated installer.nsi..." -ForegroundColor Yellow
$nsiContent = Get-Content -Path $nsiScriptPath -Raw

# Replace ADDITIONALPLUGINSPATH with the correct path
$nsiContent = $nsiContent -replace '!define ADDITIONALPLUGINSPATH ".*?"', "!define ADDITIONALPLUGINSPATH `"$pluginDir`""

# Double check if DirectML.dll packaging is present (it should be if src-tauri/nsis/installer.nsi template was used)
if ($nsiContent -notmatch "DirectML.dll") {
    Write-Host "Injecting DirectML.dll instructions into installer.nsi..." -ForegroundColor Gray
    
    # Inject File copy
    $targetFileInstruction = 'File "${MAINBINARYSRCPATH}"'
    $replacementFileInstruction = "$targetFileInstruction`r`n  File `"\`${MAINBINARYSRCPATH}\..\DirectML.dll`""
    $nsiContent = $nsiContent.Replace($targetFileInstruction, $replacementFileInstruction)

    # Inject Delete on uninstall
    $targetDeleteInstruction = 'Delete "$INSTDIR\${MAINBINARYNAME}.exe"'
    $replacementDeleteInstruction = "$targetDeleteInstruction`r`n  Delete `"\`$INSTDIR\DirectML.dll`""
    $nsiContent = $nsiContent.Replace($targetDeleteInstruction, $replacementDeleteInstruction)
}

Set-Content -Path $nsiScriptPath -Value $nsiContent -NoNewline

# 5. Compile NSIS installer
Write-Host "Compiling NSIS installer..." -ForegroundColor Yellow
Push-Location $nsisWorkingDir
try {
    & $makensisPath installer.nsi
} finally {
    Pop-Location
}

# 6. Copy results to own-build folder
$distDir = "D:\Projects\Handy-own-build"
Write-Host "Copying packaged files to $distDir..." -ForegroundColor Yellow

if (-not (Test-Path $distDir)) {
    New-Item -ItemType Directory -Path $distDir -Force | Out-Null
}

# Copy installer
$compiledInstaller = "$nsisWorkingDir\nsis-output.exe"
$targetInstaller = "$distDir\${productName}_${version}_x64-setup.exe"
Copy-Item -Path $compiledInstaller -Destination $targetInstaller -Force

# Create portable ZIP
Write-Host "Creating portable ZIP..." -ForegroundColor Yellow
$tempPortable = "$distDir\temp_portable"
if (Test-Path $tempPortable) { Remove-Item -Path $tempPortable -Recurse -Force }

New-Item -ItemType Directory -Path $tempPortable -Force | Out-Null
Copy-Item -Path $exePath -Destination "$tempPortable\"
Copy-Item -Path $dllPath -Destination "$tempPortable\"
Copy-Item -Path $resourcesPath -Destination "$tempPortable\resources" -Recurse -Force

$zipPath = "$distDir\${productName}_${version}_Portable.zip"
if (Test-Path $zipPath) { Remove-Item -Path $zipPath -Force }

Compress-Archive -Path "$tempPortable\*" -DestinationPath $zipPath -Force
Remove-Item -Path $tempPortable -Recurse -Force

Write-Host "Successfully packaged Handy!" -ForegroundColor Green
Write-Host "1. Installer: $targetInstaller" -ForegroundColor Green
Write-Host "2. Portable ZIP: $zipPath" -ForegroundColor Green
