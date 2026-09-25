# installer/build_installer.ps1
# Production Installer Build & Packaging Orchestrator
param (
    [switch]$SkipBuild = $false
)

$ErrorActionPreference = "Stop"

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "   VALORANT Optimizer: Production Installer Packaging     " -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan

$WorkspaceRoot = (Resolve-Path "$PSScriptRoot\..").Path
$ReleaseDir = Join-Path $WorkspaceRoot "target\release"
$OutputDir = Join-Path $PSScriptRoot "output"
$BundleDir = Join-Path $OutputDir "bundle"

if (-not (Test-Path $OutputDir)) {
    New-Item -ItemType Directory -Path $OutputDir -Force | Out-Null
}
if (-not (Test-Path $BundleDir)) {
    New-Item -ItemType Directory -Path $BundleDir -Force | Out-Null
}

# Step 1: Ensure Binaries are Built
$binaries = @("val-opt-core.exe", "val-opt-cli.exe", "val-opt-gui.exe")
foreach ($bin in $binaries) {
    $binPath = Join-Path $ReleaseDir $bin
    if (-not (Test-Path $binPath)) {
        Write-Error "Required binary $bin not found in $ReleaseDir. Build release profile first."
        exit 1
    }
}

# Step 2: Static Security Hardening Verification
Write-Host "`n[1/5] Verifying binary hardening flags (ASLR, DEP, CFG)..." -ForegroundColor Yellow
& powershell -ExecutionPolicy Bypass -File (Join-Path $WorkspaceRoot "scripts\verify_hardening.ps1") -TargetDir $ReleaseDir
if ($LASTEXITCODE -ne 0) {
    Write-Error "Binary hardening verification failed."
    exit 1
}

# Step 3: Authenticode Digital Code Signing
Write-Host "`n[2/5] Applying Microsoft Authenticode digital signatures..." -ForegroundColor Yellow
& powershell -ExecutionPolicy Bypass -File (Join-Path $WorkspaceRoot "scripts\sign_binaries.ps1") -TargetDir $ReleaseDir
if ($LASTEXITCODE -ne 0) {
    Write-Error "Authenticode signing failed."
    exit 1
}

# Step 4: Assemble Production Distribution Bundle
Write-Host "`n[3/5] Assembling distribution payload..." -ForegroundColor Yellow
foreach ($bin in $binaries) {
    Copy-Item (Join-Path $ReleaseDir $bin) -Destination $BundleDir -Force
}

# Generate standalone clean installer script: install.ps1
$installScriptContent = @'
# VALORANT Optimizer Standalone One-Click Installer
# Elevates to Administrator, installs to Program Files, sets up auto-rollback uninstaller
[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdmin) {
    Write-Host "Elevating installer to Administrator..." -ForegroundColor Yellow
    Start-Process powershell.exe -ArgumentList "-NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`"" -Verb RunAs
    exit 0
}

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "   Installing VALORANT Performance Optimizer (v0.1.0)     " -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan

$InstallPath = "$env:ProgramFiles\ValorantOptimizer"
$DataPath = "$env:ProgramData\ValorantOptimizer"

if (-not (Test-Path $InstallPath)) { New-Item -ItemType Directory -Path $InstallPath -Force | Out-Null }
if (-not (Test-Path $DataPath)) { New-Item -ItemType Directory -Path $DataPath -Force | Out-Null }

# Harden ProgramData directory ACL: SYSTEM (Full), Administrators (Full), Users (ReadAndExecute only)
# Block inheritance to prevent unprivileged write access from parent directory (TASK-SEC-02)
$acl = New-Object System.Security.AccessControl.DirectorySecurity
$acl.SetAccessRuleProtection($true, $false)
$adminRule = New-Object System.Security.AccessControl.FileSystemAccessRule(
    [System.Security.Principal.SecurityIdentifier]::new([System.Security.Principal.WellKnownSidType]::BuiltinAdministratorsSid, $null),
    "FullControl",
    "ContainerInherit, ObjectInherit",
    "None",
    "Allow"
)
$systemRule = New-Object System.Security.AccessControl.FileSystemAccessRule(
    [System.Security.Principal.SecurityIdentifier]::new([System.Security.Principal.WellKnownSidType]::LocalSystemSid, $null),
    "FullControl",
    "ContainerInherit, ObjectInherit",
    "None",
    "Allow"
)
$usersRule = New-Object System.Security.AccessControl.FileSystemAccessRule(
    [System.Security.Principal.SecurityIdentifier]::new([System.Security.Principal.WellKnownSidType]::BuiltinUsersSid, $null),
    "ReadAndExecute",
    "ContainerInherit, ObjectInherit",
    "None",
    "Allow"
)
$acl.AddAccessRule($systemRule)
$acl.AddAccessRule($adminRule)
$acl.AddAccessRule($usersRule)
Set-Acl $DataPath $acl


$binaries = @("val-opt-core.exe", "val-opt-cli.exe", "val-opt-gui.exe")
foreach ($bin in $binaries) {
    $src = Join-Path $PSScriptRoot $bin
    if (Test-Path $src) {
        Copy-Item $src -Destination (Join-Path $InstallPath $bin) -Force
    }
}

# Create Start Menu Shortcuts
$StartMenuDir = "$env:ProgramData\Microsoft\Windows\Start Menu\Programs\VALORANT Performance Optimizer"
if (-not (Test-Path $StartMenuDir)) { New-Item -ItemType Directory -Path $StartMenuDir -Force | Out-Null }

$WshShell = New-Object -ComObject WScript.Shell

$shortcutGui = $WshShell.CreateShortcut((Join-Path $StartMenuDir "VALORANT Performance Optimizer.lnk"))
$shortcutGui.TargetPath = Join-Path $InstallPath "val-opt-gui.exe"
$shortcutGui.Description = "VALORANT Performance Optimizer Native Dashboard"
$shortcutGui.Save()

$shortcutCli = $WshShell.CreateShortcut((Join-Path $StartMenuDir "VALORANT Optimizer CLI.lnk"))
$shortcutCli.TargetPath = Join-Path $InstallPath "val-opt-cli.exe"
$shortcutCli.Description = "VALORANT Optimizer Command Line Interface"
$shortcutCli.Save()

# Generate uninstall script with guaranteed full rollback before file removal
$uninstallScript = @"
[CmdletBinding()]
param()
`$ErrorActionPreference = 'SilentlyContinue'
Write-Host 'Rolling back all VALORANT optimizations to system baseline...' -ForegroundColor Yellow
& `"$InstallPath\val-opt-cli.exe`" rollback
Start-Sleep -Seconds 1
Stop-Process -Name 'val-opt-core', 'val-opt-gui' -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 1
Remove-Item -Path `"$InstallPath`" -Recurse -Force
Remove-Item -Path `"$DataPath`" -Recurse -Force
Remove-Item -Path `"$StartMenuDir`" -Recurse -Force
Remove-ItemProperty -Path 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\ValorantOptimizer' -Name '*' -ErrorAction SilentlyContinue
Remove-Item -Path 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\ValorantOptimizer' -Force -ErrorAction SilentlyContinue
Write-Host 'VALORANT Performance Optimizer successfully uninstalled.' -ForegroundColor Green
"@
Set-Content -Path (Join-Path $InstallPath "uninstall.ps1") -Value $uninstallScript -Encoding UTF8

# Register in Windows Add/Remove Programs
$regPath = "HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\ValorantOptimizer"
if (-not (Test-Path $regPath)) { New-Item -Path $regPath -Force | Out-Null }
Set-ItemProperty -Path $regPath -Name "DisplayName" -Value "VALORANT Performance Optimizer"
Set-ItemProperty -Path $regPath -Name "DisplayVersion" -Value "0.1.0"
Set-ItemProperty -Path $regPath -Name "Publisher" -Value "ValOpt Technologies"
Set-ItemProperty -Path $regPath -Name "InstallLocation" -Value $InstallPath
Set-ItemProperty -Path $regPath -Name "UninstallString" -Value "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$InstallPath\uninstall.ps1`""
Set-ItemProperty -Path $regPath -Name "DisplayIcon" -Value (Join-Path $InstallPath "val-opt-gui.exe")
Set-ItemProperty -Path $regPath -Name "NoModify" -Value 1
Set-ItemProperty -Path $regPath -Name "NoRepair" -Value 1

Write-Host "`n[SUCCESS] Installation complete! Shortcuts created in Start Menu." -ForegroundColor Green
'@
Set-Content -Path (Join-Path $BundleDir "install.ps1") -Value $installScriptContent -Encoding UTF8

# Create one-click launcher Install.cmd
$cmdLauncher = "@echo off`r`npowershell.exe -NoProfile -ExecutionPolicy Bypass -File `"%~dp0install.ps1`"`r`npause"
Set-Content -Path (Join-Path $BundleDir "Install.cmd") -Value $cmdLauncher -Encoding ASCII

# Step 5: Check for Inno Setup (ISCC) and compile standalone installer
Write-Host "`n[4/5] Searching for Inno Setup compiler (ISCC)..." -ForegroundColor Yellow

$isccPaths = @(
    "C:\Program Files (x86)\Inno Setup 6\iscc.exe",
    "C:\Program Files\Inno Setup 6\iscc.exe",
    "iscc.exe"
)

$iscc = $null
foreach ($p in $isccPaths) {
    if (Test-Path $p) { $iscc = $p; break }
    $cmd = Get-Command $p -ErrorAction SilentlyContinue
    if ($cmd) { $iscc = $cmd.Source; break }
}

$installerExe = $null

if ($iscc) {
    Write-Host "Compiling Inno Setup package via $iscc..." -ForegroundColor Green
    & $iscc (Join-Path $PSScriptRoot "setup.iss")
    $setupPath = Join-Path $OutputDir "ValorantOptimizer_Setup_0.1.0.exe"
    if (Test-Path $setupPath) {
        $installerExe = $setupPath
    }
} else {
    Write-Host "Inno Setup compiler not found. Generating self-contained release zip archive..." -ForegroundColor Yellow
    $zipPath = Join-Path $OutputDir "ValorantOptimizer_v0.1.0_Production.zip"
    if (Test-Path $zipPath) { Remove-Item $zipPath -Force }
    Start-Sleep -Seconds 1
    
    $archived = $false
    for ($attempt = 1; $attempt -le 3; $attempt++) {
        try {
            Compress-Archive -Path "$BundleDir\*" -DestinationPath $zipPath -Force -ErrorAction Stop
            $archived = $true
            break
        } catch {
            Write-Warning "Attempt $attempt archive failed ($($_.Exception.Message)). Retrying in 1s..."
            Start-Sleep -Seconds 1
        }
    }
    if (-not $archived) {
        Write-Error "Failed to compress distribution bundle into $zipPath"
        exit 1
    }
    Write-Host "Distribution zip archive created: $zipPath" -ForegroundColor Green
}

# Step 6: Final Verification & SHA-256 Manifest
Write-Host "`n[5/5] Generating Release Manifest & Cryptographic Hashes..." -ForegroundColor Yellow

$manifestFiles = Get-ChildItem -Path $OutputDir -File -Recurse
$manifest = @()

foreach ($file in $manifestFiles) {
    $hash = (Get-FileHash -Path $file.FullName -Algorithm SHA256).Hash
    $sig = Get-AuthenticodeSignature -FilePath $file.FullName -ErrorAction SilentlyContinue
    $manifest += [PSCustomObject]@{
        Name       = $file.Name
        SizeBytes  = $file.Length
        Signature  = if ($sig -and $sig.Status) { $sig.Status } else { "N/A" }
        SHA256     = $hash
    }
}

Write-Host "`n==========================================================" -ForegroundColor Cyan
Write-Host "   VALORANT Optimizer: Release Manifest Generated         " -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan
$manifest | Format-Table -AutoSize

Write-Host "`n[SUCCESS] Production Packaging Complete!" -ForegroundColor Green
exit 0
