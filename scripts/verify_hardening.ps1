# scripts/verify_hardening.ps1
# Static Security Analysis & Binary Hardening Verification Tool
param (
    [string]$TargetDir = "$PSScriptRoot\..\target\release"
)

$ErrorActionPreference = "Stop"

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "   VALORANT Optimizer: Binary Hardening Verification      " -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan

$dumpbinPaths = @(
    "C:\Program Files\Microsoft Visual Studio\18\Community\VC\Tools\MSVC\14.51.36231\bin\Hostx64\x64\dumpbin.exe",
    "dumpbin.exe"
)

$dumpbin = $null
foreach ($p in $dumpbinPaths) {
    if (Test-Path $p) {
        $dumpbin = $p
        break
    }
    $cmd = Get-Command $p -ErrorAction SilentlyContinue
    if ($cmd) {
        $dumpbin = $cmd.Source
        break
    }
}

$binaries = @(
    "val-opt-core.exe",
    "val-opt-cli.exe",
    "val-opt-gui.exe"
)

$results = @()
$allPass = $true

foreach ($bin in $binaries) {
    $fullPath = Join-Path $TargetDir $bin
    if (-not (Test-Path $fullPath)) {
        Write-Warning "Binary not found at $fullPath (skipping until built)"
        continue
    }

    Write-Host "`nInspecting $bin..." -ForegroundColor Yellow

    $hasASLR = $false
    $hasHighEntropy = $false
    $hasDEP = $false
    $hasCFG = $false
    $hasCET = $false

    if ($dumpbin) {
        $headerOutput = & $dumpbin /headers $fullPath 2>&1
        $headerText = $headerOutput -join "`n"

        if ($headerText -match "Dynamic base") { $hasASLR = $true }
        if ($headerText -match "High [Ee]ntropy") { $hasHighEntropy = $true }
        if ($headerText -match "NX compatible") { $hasDEP = $true }
        if ($headerText -match "Control Flow Guard") { $hasCFG = $true }
        if ($headerText -match "CET compatible" -or $headerText -match "CET / Shadow stack") { $hasCET = $true }
    } else {
        # Fallback to direct PE header inspection via .NET binary reader
        $bytes = [System.IO.File]::ReadAllBytes($fullPath)
        $peOffset = [BitConverter]::ToInt32($bytes, 0x3C)
        $dllCharacteristicsOffset = $peOffset + 0x18 + 0x46 # Standard PE32+ OptionalHeader DllCharacteristics
        $dllChars = [BitConverter]::ToUInt16($bytes, $dllCharacteristicsOffset)

        $IMAGE_DLLCHARACTERISTICS_HIGH_ENTROPY_VA = 0x0020
        $IMAGE_DLLCHARACTERISTICS_DYNAMIC_BASE    = 0x0040
        $IMAGE_DLLCHARACTERISTICS_NX_COMPAT       = 0x0100
        $IMAGE_DLLCHARACTERISTICS_GUARD_CF        = 0x4000

        if (($dllChars -band $IMAGE_DLLCHARACTERISTICS_DYNAMIC_BASE) -ne 0) { $hasASLR = $true }
        if (($dllChars -band $IMAGE_DLLCHARACTERISTICS_HIGH_ENTROPY_VA) -ne 0) { $hasHighEntropy = $true }
        if (($dllChars -band $IMAGE_DLLCHARACTERISTICS_NX_COMPAT) -ne 0) { $hasDEP = $true }
        if (($dllChars -band $IMAGE_DLLCHARACTERISTICS_GUARD_CF) -ne 0) { $hasCFG = $true }
    }

    $pass = $hasASLR -and $hasHighEntropy -and $hasDEP -and $hasCFG
    if (-not $pass) { $allPass = $false }

    $result = [PSCustomObject]@{
        Binary      = $bin
        ASLR        = if ($hasASLR) { "PASS" } else { "FAIL" }
        HighEntropy = if ($hasHighEntropy) { "PASS" } else { "FAIL" }
        DEP_NX      = if ($hasDEP) { "PASS" } else { "FAIL" }
        CFG         = if ($hasCFG) { "PASS" } else { "FAIL" }
        CET         = if ($hasCET) { "PASS" } else { "N/A" }
        Status      = if ($pass) { "HARDENED" } else { "DEFICIENT" }
    }
    $results += $result
}

Write-Host "`nHardening Summary:" -ForegroundColor Cyan
$results | Format-Table -AutoSize

if ($allPass -and $results.Count -gt 0) {
    Write-Host "`n[SUCCESS] All inspected release binaries are hardened according to production security specs." -ForegroundColor Green
    exit 0
} else {
    Write-Host "`n[WARNING] Some binaries failed hardening checks or were missing." -ForegroundColor Red
    exit 1
}
