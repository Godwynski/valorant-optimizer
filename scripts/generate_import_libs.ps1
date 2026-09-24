param(
    [string]$LibDir = "lib64"
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path $LibDir)) {
    New-Item -ItemType Directory -Path $LibDir -Force | Out-Null
}

$dumpbin = "C:\Program Files\Microsoft Visual Studio\18\Community\VC\Tools\MSVC\14.51.36231\bin\Hostx64\x64\dumpbin.exe"
$libexe  = "C:\Program Files\Microsoft Visual Studio\18\Community\VC\Tools\MSVC\14.51.36231\bin\Hostx64\x64\lib.exe"

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
