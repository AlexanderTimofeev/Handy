$ErrorActionPreference = 'Stop'
$packageScript = Join-Path $PSScriptRoot 'package.ps1'
if (-not (Test-Path $packageScript)) { throw 'package.ps1 must exist' }

$errors = $null
$tokens = $null
[System.Management.Automation.Language.Parser]::ParseFile($packageScript, [ref]$tokens, [ref]$errors) | Out-Null
if ($errors.Count -ne 0) { throw "package.ps1 has PowerShell parse errors: $($errors -join '; ')" }

$content = Get-Content -Raw -Path $packageScript
if ($content -match 'DirectML\.dll') { throw 'package.ps1 must not depend on removed DirectML.dll' }
if ($content -notmatch 'onnxruntime\.dll') { throw 'package.ps1 must package current onnxruntime.dll runtime' }
if ($content -notmatch '\[string\]\s*\$OutputDir') { throw 'package.ps1 must expose an OutputDir parameter' }
if ($content -match 'D:\\Projects\\Handy-own-build') { throw 'package.ps1 must not hard-code the old D:\Projects\Handy-own-build destination' }
if ($content -notmatch 'Compress-Archive') { throw 'package.ps1 must still create a portable ZIP' }
if ($content -notmatch 'bundle\\nsis') { throw 'package.ps1 must use the current Tauri NSIS bundle output' }
if ($content -notmatch 'Join-Path \$tauriDir \"resources\"') { throw 'package.ps1 must package resources from the current Tauri source directory' }

Write-Host 'package.ps1 contract: OK'
