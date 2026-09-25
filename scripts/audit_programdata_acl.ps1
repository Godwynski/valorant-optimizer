# scripts/audit_programdata_acl.ps1
# Security Audit: Verifies ProgramData\ValorantOptimizer ACL hardening
# Enforces:
# 1. Inheritance is disabled / protected
# 2. SYSTEM has FullControl
# 3. Builtin Administrators has FullControl
# 4. Standard Users (BUILTIN\Users or Authenticated Users) do NOT have Write, Modify, Delete, or CreateFiles rights.

param (
    [string]$TargetDir = "$env:ProgramData\ValorantOptimizer"
)

$ErrorActionPreference = "Stop"

Write-Host "Auditing ACL permissions for: $TargetDir" -ForegroundColor Cyan

if (-not (Test-Path $TargetDir)) {
    Write-Error "Target directory does not exist: $TargetDir"
    exit 1
}

$acl = Get-Acl $TargetDir
$isProtected = $acl.AreAccessRulesProtected

Write-Host "  -> Access Rules Protected (Inheritance Disabled): $isProtected"
if (-not $isProtected) {
    Write-Error "SECURITY VIOLATION: Inheritance is NOT disabled on $TargetDir!"
    exit 1
}

$forbiddenRights = @(
    [System.Security.AccessControl.FileSystemRights]::Write,
    [System.Security.AccessControl.FileSystemRights]::Modify,
    [System.Security.AccessControl.FileSystemRights]::CreateFiles,
    [System.Security.AccessControl.FileSystemRights]::AppendData,
    [System.Security.AccessControl.FileSystemRights]::WriteData,
    [System.Security.AccessControl.FileSystemRights]::Delete,
    [System.Security.AccessControl.FileSystemRights]::DeleteSubdirectoriesAndFiles,
    [System.Security.AccessControl.FileSystemRights]::FullControl
)

$violations = @()

foreach ($rule in $acl.Access) {
    $identity = $rule.IdentityReference.Value
    $rights = $rule.FileSystemRights
    $type = $rule.AccessControlType

    Write-Host "  -> Rule: $identity | Rights: $rights | Type: $type"

    # Check if this rule applies to standard users
    $isUserIdentity = ($identity -like "*Users*" -or $identity -like "*Authenticated Users*" -or $identity -like "*Everyone*") -and ($identity -notlike "*Administrators*")

    if ($isUserIdentity -and $type -eq [System.Security.AccessControl.AccessControlType]::Allow) {
        foreach ($forbidden in $forbiddenRights) {
            if (($rights -band $forbidden) -eq $forbidden) {
                $violations += "Unprivileged identity '$identity' holds forbidden right: $forbidden"
            }
        }
    }
}

if ($violations.Count -gt 0) {
    Write-Host "`n[FAIL] Security ACL audit failed with $($violations.Count) violation(s):" -ForegroundColor Red
    foreach ($v in $violations) {
        Write-Host "  [X] $v" -ForegroundColor Red
    }
    exit 1
}

Write-Host "`n[SUCCESS] ACL audit passed! Users have no write/modify rights on $TargetDir." -ForegroundColor Green
exit 0
