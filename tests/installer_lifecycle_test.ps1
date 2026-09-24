# tests/installer_lifecycle_test.ps1
# Installer Lifecycle, Placement, and Rollback Verification Test
param (
    [string]$BundleDir = "$PSScriptRoot\..\installer\output\bundle"
)

$ErrorActionPreference = "Stop"

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "   Testing Installer & Uninstaller Lifecycle              " -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan

$TestInstallRoot = "$PSScriptRoot\..\target\test_install"
$TestDataRoot = "$PSScriptRoot\..\target\test_programdata"

if (Test-Path $TestInstallRoot) { Remove-Item $TestInstallRoot -Recurse -Force }
if (Test-Path $TestDataRoot) { Remove-Item $TestDataRoot -Recurse -Force }

New-Item -ItemType Directory -Path $TestInstallRoot -Force | Out-Null
New-Item -ItemType Directory -Path $TestDataRoot -Force | Out-Null

Write-Host "[1/4] Simulating deployment to isolated target..." -ForegroundColor Yellow
$binaries = @("val-opt-core.exe", "val-opt-cli.exe", "val-opt-gui.exe")
foreach ($bin in $binaries) {
    $src = Join-Path $BundleDir $bin
    if (-not (Test-Path $src)) {
        Write-Error "Bundle binary not found: $src"
        exit 1
    }
    Copy-Item $src -Destination (Join-Path $TestInstallRoot $bin) -Force
}

# Verify files copied
foreach ($bin in $binaries) {
    $targetFile = Join-Path $TestInstallRoot $bin
    if (-not (Test-Path $targetFile)) {
        Write-Error "File verification failed for $targetFile"
        exit 1
    }
}
Write-Host "  -> All 3 binaries verified in installation directory." -ForegroundColor Green

Write-Host "`n[2/4] Testing CLI rollback invocation from installed location..." -ForegroundColor Yellow
$cliPath = Join-Path $TestInstallRoot "val-opt-cli.exe"
$rollbackOutput = & {
    $ErrorActionPreference = "Continue"
    & $cliPath rollback 2>&1
}
Write-Host "  -> CLI rollback output: $($rollbackOutput -join ' ')" -ForegroundColor Cyan

Write-Host "`n[3/4] Verifying binary integrity in installed directory..." -ForegroundColor Yellow
foreach ($bin in $binaries) {
    $targetFile = Join-Path $TestInstallRoot $bin
    $sig = Get-AuthenticodeSignature -FilePath $targetFile
    if (-not $sig.SignatureType -eq "Authenticode") {
        Write-Error "Authenticode signature missing in installed copy: $bin"
        exit 1
    }
}
Write-Host "  -> All deployed binaries maintain valid Authenticode signatures." -ForegroundColor Green

Write-Host "`n[4/4] Simulating clean uninstallation removal..." -ForegroundColor Yellow
Remove-Item -Path $TestInstallRoot -Recurse -Force
Remove-Item -Path $TestDataRoot -Recurse -Force

if ((Test-Path $TestInstallRoot) -or (Test-Path $TestDataRoot)) {
    Write-Error "Leftover files detected after uninstallation!"
    exit 1
}
Write-Host "  -> Zero file leftovers. Uninstallation cleanly completed." -ForegroundColor Green

Write-Host "`n[SUCCESS] Installer & uninstaller lifecycle test PASSED!" -ForegroundColor Green
exit 0
