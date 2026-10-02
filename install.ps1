#requires -Version 7.3
<#
.SYNOPSIS
Install wslc-simple-gcc from GitHub without cloning the repository.
.DESCRIPTION
Installs a self-contained PowerShell script into ~/.local/bin by default.
Adds the directory to the current process PATH and the Windows user PATH.
Run again to update. Use -NoPath to leave both PATH values unchanged.
.EXAMPLE
./install.ps1
.EXAMPLE
./install.ps1 -InstallDirectory 'C:\tools\bin' -NoPath -Ref main
#>
[CmdletBinding()]
param(
    [ValidateNotNullOrEmpty()]
    [string] $InstallDirectory = (Join-Path ([Environment]::GetFolderPath('UserProfile')) '.local/bin'),

    [ValidateNotNullOrEmpty()]
    [string] $Ref = 'main',

    [switch] $NoPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

if (-not $IsWindows) { throw 'Run this installer from PowerShell 7.3+ on Windows.' }
$installPath = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($InstallDirectory)
if (-not $NoPath -and $installPath.Contains(';')) {
    throw 'InstallDirectory cannot contain a semicolon when adding it to PATH. Use -NoPath for this directory.'
}
if ((Test-Path -LiteralPath $installPath) -and -not (Test-Path -LiteralPath $installPath -PathType Container)) {
    throw "InstallDirectory is not a directory: $installPath"
}

# Download and parse before replacing an existing installation.
$encodedRef = [Uri]::EscapeDataString($Ref)
$uri = "https://raw.githubusercontent.com/uni-kakurenbo/wslc-simple-gcc/$encodedRef/Run-Gcc16.ps1"
$source = Invoke-RestMethod -Uri $uri -ErrorAction Stop
if ($source -isnot [string] -or [string]::IsNullOrWhiteSpace($source)) {
    throw 'GitHub did not return a PowerShell script.'
}
$tokens = $null
$parseErrors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseInput($source, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count -ne 0 -or $null -eq $ast.ParamBlock -or 'Source' -notin $ast.ParamBlock.Parameters.Name.VariablePath.UserPath) {
    throw 'The downloaded script is invalid; the existing installation has not been changed.'
}
if (-not $source.Contains('MIT License') -or -not $source.Contains('Copyright (c)')) {
    throw 'The downloaded script has no embedded license. Select a revision that supports this installer.'
}

[IO.Directory]::CreateDirectory($installPath) | Out-Null
$targetPath = Join-Path $installPath 'wslc-simple-gcc.ps1'
$stagingPath = Join-Path $installPath ('.wslc-simple-gcc-' + [guid]::NewGuid().ToString('N') + '.tmp')
try {
    [IO.File]::WriteAllText($stagingPath, $source.Replace("`r`n", "`n"), [Text.UTF8Encoding]::new($false))
    # Stage in the same directory so replacement is a single filesystem rename.
    [IO.File]::Move($stagingPath, $targetPath, $true)
} finally {
    if (Test-Path -LiteralPath $stagingPath) {
        Remove-Item -LiteralPath $stagingPath -Force
    }
}

function Add-PathEntry {
    param([AllowNull()][string] $CurrentPath, [string] $Entry)

    $normalizedEntry = $Entry.TrimEnd('\', '/')
    foreach ($part in ($CurrentPath -split ';')) {
        $expanded = [Environment]::ExpandEnvironmentVariables($part.Trim().Trim('"')).TrimEnd('\', '/')
        if ($expanded.Equals($normalizedEntry, [StringComparison]::OrdinalIgnoreCase)) {
            return $CurrentPath
        }
    }
    if ([string]::IsNullOrEmpty($CurrentPath)) { return $Entry }
    return $CurrentPath.TrimEnd(';') + ';' + $Entry
}

if (-not $NoPath) {
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $updatedUserPath = Add-PathEntry -CurrentPath $userPath -Entry $installPath
    if ($updatedUserPath -cne $userPath) {
        [Environment]::SetEnvironmentVariable('Path', $updatedUserPath, 'User')
    }
    $env:Path = Add-PathEntry -CurrentPath $env:Path -Entry $installPath
}

Write-Host "Installed: $targetPath"
Write-Host "Source revision: $Ref"
if ($NoPath) {
    Write-Host 'PATH was not changed. Invoke the installed script by its full path or add the directory to PATH.'
} else {
    Write-Host 'Ready: wslc-simple-gcc .\main.cpp'
}
