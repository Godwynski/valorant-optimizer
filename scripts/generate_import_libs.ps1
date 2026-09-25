param(
    [string]$LibDir = "lib64"
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path $LibDir)) {
    New-Item -ItemType Directory -Path $LibDir -Force | Out-Null
}

$dumpbin = $null
$libexe  = $null

$cmdDumpbin = Get-Command "dumpbin.exe" -ErrorAction SilentlyContinue
if ($cmdDumpbin) { $dumpbin = $cmdDumpbin.Source }
$cmdLib = Get-Command "lib.exe" -ErrorAction SilentlyContinue
if ($cmdLib) { $libexe = $cmdLib.Source }

if (-not $dumpbin -or -not $libexe) {
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    if (Test-Path $vswhere) {
        $vsPath = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
        if ($vsPath) {
            $msvcRoot = Join-Path $vsPath "VC\Tools\MSVC"
            if (Test-Path $msvcRoot) {
                $latestMsvc = Get-ChildItem $msvcRoot | Sort-Object Name -Descending | Select-Object -First 1
                if ($latestMsvc) {
                    if (-not $dumpbin) {
                        $candidate = Join-Path $latestMsvc.FullName "bin\Hostx64\x64\dumpbin.exe"
                        if (Test-Path $candidate) { $dumpbin = $candidate }
                    }
                    if (-not $libexe) {
                        $candidate = Join-Path $latestMsvc.FullName "bin\Hostx64\x64\lib.exe"
                        if (Test-Path $candidate) { $libexe = $candidate }
                    }
                }
            }
        }
    }
}

if (-not $dumpbin -or -not $libexe) {
    Write-Error "dumpbin.exe or lib.exe could not be located via PATH or Visual Studio installation."
    exit 1
}

$dlls = @(
    "kernel32",
    "ntdll",
    "userenv",
    "ws2_32",
    "dbghelp",
    "advapi32",
    "user32",
    "ole32",
    "shell32",
    "iphlpapi",
    "dxgi",
    "gdi32",
    "ucrtbase"
)

foreach ($dll in $dlls) {
    $dllPath = "C:\Windows\System32\$dll.dll"
    if (-not (Test-Path $dllPath)) {
        Write-Warning "DLL not found: $dllPath"
        continue
    }

    Write-Host "Generating import lib for $dll..."
    $dumpOutput = & $dumpbin /exports $dllPath
    $defLines = [System.Collections.Generic.List[string]]::new()
    $defLines.Add("LIBRARY $dll")
    $defLines.Add("EXPORTS")

    $parsing = $false
    foreach ($line in $dumpOutput) {
        if ($line -match "ordinal\s+hint\s+RVA\s+name") {
            $parsing = $true
            continue
        }
        if ($parsing) {
            if ($line -match "^\s*Summary" -or $line -match "^\s*$") {
                if ($defLines.Count -gt 2) {
                    # Done with exports section
                    break
                }
                continue
            }
            # Match ordinal hint RVA name (handling forwarded exports where RVA might be blank or name is at end)
            if ($line -match "^\s*\d+\s+[0-9A-Fa-f]+\s+(?:[0-9A-Fa-f]+\s+)?([A-Za-z0-9_@?$]+)") {
                $funcName = $Matches[1]
                $defLines.Add("    $funcName")
            }
        }
    }

    $defPath = Join-Path $LibDir "$dll.def"
    $libPath = Join-Path $LibDir "$dll.lib"
    [System.IO.File]::WriteAllLines($defPath, $defLines)
    & $libexe "/def:$defPath" "/machine:x64" "/out:$libPath" | Out-Null
    Remove-Item $defPath -Force -ErrorAction SilentlyContinue

    if (Test-Path $libPath) {
        $size = (Get-Item $libPath).Length
        Write-Host "Successfully generated $dll.lib ($size bytes)"
    } else {
        Write-Error "Failed to generate $dll.lib"
    }
}

if (Test-Path (Join-Path $LibDir "ucrtbase.lib")) {
    Copy-Item (Join-Path $LibDir "ucrtbase.lib") (Join-Path $LibDir "ucrt.lib") -Force
    Write-Host "Created ucrt.lib alias from ucrtbase.lib"
}

Write-Host "Import library generation complete."
