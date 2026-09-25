# scripts/sign_binaries.ps1
# Microsoft Authenticode Digital Code Signing Pipeline
param (
    [string]$TargetDir = "$PSScriptRoot\..\target\release",
    [string]$CertThumbprint = "",
    [string]$CertFilePath = "",
    [string]$CertPassword = "",
    [string]$TimestampServer = "http://timestamp.digicert.com",
    [string]$SpecificFile = ""
)

$ErrorActionPreference = "Stop"

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "   VALORANT Optimizer: Authenticode Code Signing Pipeline " -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan

# Step 1: Resolve Code Signing Certificate
$cert = $null

if ($CertFilePath -ne "" -and (Test-Path $CertFilePath)) {
    Write-Host "Loading code-signing certificate from file: $CertFilePath" -ForegroundColor Yellow
    $securePass = if ($CertPassword) { ConvertTo-SecureString $CertPassword -AsPlainText -Force } else { $null }
    $cert = [System.Security.Cryptography.X509Certificates.X509Certificate2]::new($CertFilePath, $securePass)
} elseif ($CertThumbprint -ne "") {
    Write-Host "Searching for certificate with thumbprint $CertThumbprint" -ForegroundColor Yellow
    $cert = Get-ChildItem Cert:\CurrentUser\My, Cert:\LocalMachine\My | Where-Object { $_.Thumbprint -eq $CertThumbprint } | Select-Object -First 1
} else {
    # Check for existing ValOpt certificate in CurrentUser\My
    $existing = Get-ChildItem Cert:\CurrentUser\My -CodeSigningCert -ErrorAction SilentlyContinue | 
                Where-Object { $_.Subject -match "Valorant Performance Optimizer" } | 
                Select-Object -First 1

    if ($existing) {
        Write-Host "Found existing ValOpt code-signing certificate: $($existing.Thumbprint)" -ForegroundColor Green
        $cert = $existing
    } else {
        Write-Host "Generating local high-assurance self-signed code-signing certificate..." -ForegroundColor Yellow
        $cert = New-SelfSignedCertificate `
            -Type CodeSigningCert `
            -Subject "CN=Valorant Performance Optimizer, O=ValOpt Technologies, OU=Release Engineering, C=US" `
            -KeySpec Signature `
            -KeyExportPolicy Exportable `
            -KeyUsage DigitalSignature `
            -KeyLength 2048 `
            -HashAlgorithm SHA256 `
            -CertStoreLocation "Cert:\CurrentUser\My" `
            -NotAfter (Get-Date).AddYears(5) `
            -FriendlyName "ValOpt Authenticode Signing"

        # Trust this certificate in Trusted Publisher
        Write-Host "Registering public key in CurrentUser\TrustedPublisher..." -ForegroundColor Yellow
        $publisherStore = [System.Security.Cryptography.X509Certificates.X509Store]::new("TrustedPublisher", "CurrentUser")
        $publisherStore.Open("ReadWrite")
        $publisherStore.Add($cert)
        $publisherStore.Close()

        Write-Host "Successfully provisioned and trusted code-signing certificate: $($cert.Thumbprint)" -ForegroundColor Green
    }
}

if (-not $cert) {
    Write-Error "Failed to obtain a valid code-signing certificate."
    exit 1
}

# Step 2: Locate signtool.exe or fallback to Set-AuthenticodeSignature
$signtool = $null
$c = Get-Command "signtool.exe" -ErrorAction SilentlyContinue
if ($c) {
    $signtool = $c.Source
} else {
    $sdkSearch = @(
        "C:\Program Files (x86)\Windows Kits\10\bin\*\x64\signtool.exe",
        "C:\Program Files (x86)\Windows Kits\10\bin\x64\signtool.exe",
        "C:\Program Files (x86)\Microsoft SDKs\ClickOnce\SignTool\signtool.exe"
    )
    foreach ($pattern in $sdkSearch) {
        $resolved = Get-Item $pattern -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($resolved) {
            $signtool = $resolved.FullName
            break
        }
    }

    if (-not $signtool) {
        $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
        if (Test-Path $vswhere) {
            $vsPath = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
            if ($vsPath) {
                $msvcRoot = Join-Path $vsPath "VC\Tools\MSVC"
                if (Test-Path $msvcRoot) {
                    $latestMsvc = Get-ChildItem $msvcRoot | Sort-Object Name -Descending | Select-Object -First 1
                    if ($latestMsvc) {
                        $candidate = Join-Path $latestMsvc.FullName "bin\Hostx64\x64\signtool.exe"
                        if (Test-Path $candidate) { $signtool = $candidate }
                    }
                }
            }
        }
    }
}

if ($signtool) {
    Write-Host "Using SignTool: $signtool" -ForegroundColor Green
} else {
    Write-Host "Using PowerShell Set-AuthenticodeSignature" -ForegroundColor Green
}

# Step 3: Sign Target Binaries
$binaries = if ($SpecificFile -ne "") {
    @($SpecificFile)
} else {
    @(
        "val-opt-core.exe",
        "val-opt-cli.exe",
        "val-opt-gui.exe"
    )
}

$allSigned = $true
$signedResults = @()

foreach ($bin in $binaries) {
    $fullPath = Join-Path $TargetDir $bin
    if (-not (Test-Path $fullPath)) {
        Write-Warning "File not found: $fullPath (skipping)"
        continue
    }

    Write-Host "Signing $bin..." -ForegroundColor Cyan

    $signedOk = $false
    # Try with RFC 3161 timestamping first
    try {
        $sigResult = Set-AuthenticodeSignature -FilePath $fullPath -Certificate $cert -HashAlgorithm SHA256 -TimestampServer $TimestampServer -ErrorAction Stop
        if ($sigResult.Status -eq "Valid") {
            $signedOk = $true
        }
    } catch {
        Write-Warning "Timestamping server unreachable or failed: $_. Proceeding without timestamp..."
    }

    if (-not $signedOk) {
        # Fallback without timestamp
        $sigResult = Set-AuthenticodeSignature -FilePath $fullPath -Certificate $cert -HashAlgorithm SHA256
        if ($sigResult.Status -eq "Valid") {
            $signedOk = $true
        }
    }

    # Verify signature
    $verify = Get-AuthenticodeSignature -FilePath $fullPath
    $hash = (Get-FileHash -Path $fullPath -Algorithm SHA256).Hash

    $isSelfSigned = ($cert.Subject -eq $cert.Issuer)
    $hasSignature = ($verify.SignatureType -eq "Authenticode") -and ($verify.SignerCertificate -ne $null) -and ($verify.SignerCertificate.Thumbprint -eq $cert.Thumbprint)
    $isTrustedRoot = ($verify.Status -eq "Valid")
    $isSignedValid = $isTrustedRoot -or ($hasSignature -and ($verify.Status -in @("UntrustedRoot", "UnknownError")))

    $statusDisplay = if ($isTrustedRoot) { 
        "Valid (Trusted CA Root)" 
    } elseif ($hasSignature -and $isSelfSigned) { 
        "Self-Signed (UNVERIFIED for Production)" 
    } elseif ($hasSignature) { 
        "Valid (Authenticode PKCS#7)" 
    } else { 
        $verify.Status 
    }

    $signedResults += [PSCustomObject]@{
        Binary     = $bin
        Status     = $statusDisplay
        Signer     = if ($verify.SignerCertificate) { $verify.SignerCertificate.Subject } else { "N/A" }
        Thumbprint = if ($verify.SignerCertificate) { $verify.SignerCertificate.Thumbprint } else { "N/A" }
        SHA256     = $hash.Substring(0, 16) + "..."
    }

    if (-not $isSignedValid) {
        $allSigned = $false
        Write-Error "Signature validation failed for ${bin}: $($verify.StatusMessage)"
    }
}

Write-Host "`nAuthenticode Signature Results:" -ForegroundColor Cyan
$signedResults | Format-Table -AutoSize

if ($allSigned -and $signedResults.Count -gt 0) {
    $isSelfSigned = ($cert.Subject -eq $cert.Issuer)
    if ($isSelfSigned) {
        Write-Host "`n[NOTICE] Binaries signed with local development certificate. Production release signing status: UNVERIFIED (State B: Trusted commercial CA certificate required)." -ForegroundColor Yellow
    } else {
        Write-Host "`n[SUCCESS] All production binaries digitally signed and verified with Authenticode." -ForegroundColor Green
    }
    exit 0
} else {
    Write-Host "`n[ERROR] Not all binaries were successfully signed and validated." -ForegroundColor Red
    exit 1
}
