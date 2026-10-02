#requires -Version 7.3
<#
.SYNOPSIS
Check installation and updates with a local download fixture.
.DESCRIPTION
Does not access the network or change the Windows user PATH.
Pass -RunSmokeTests to also compile and run samples with the installed command.
#>
[CmdletBinding()]
param([switch] $RunSmokeTests)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path $PSScriptRoot -Parent
$installer = Join-Path $repoRoot 'install.ps1'
$tempRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot '.tmp'))
$testRoot = Join-Path $tempRoot ('install-' + [guid]::NewGuid().ToString('N'))
$installDirectory = Join-Path $testRoot 'bin 日本語 with spaces'
$installedScript = Join-Path $installDirectory 'wslc-simple-gcc.ps1'
$originalProcessPath = $env:Path
$originalUserPath = [Environment]::GetEnvironmentVariable('Path', 'User')
$downloadState = @{
    Mode = 'valid'
    Source = (Get-Content -LiteralPath (Join-Path $repoRoot 'Run-Gcc16.ps1') -Raw).Replace("`r`n", "`n")
    Uris = [Collections.Generic.List[string]]::new()
}

# The installer resolves this function from the test's scope instead of issuing HTTP.
function Invoke-RestMethod {
    [CmdletBinding()]
    param([string] $Uri)

    $downloadState.Uris.Add($Uri)
    switch ($downloadState.Mode) {
        'valid' { return $downloadState.Source }
        'offline' { throw 'Simulated download failure' }
        'invalid' { return 'param(' }
        default { throw 'Unexpected test download mode' }
    }
}

try {
    if (-not $IsWindows) { throw 'Installer tests require Windows.' }
    New-Item -ItemType Directory -Path $installDirectory -Force | Out-Null
    $unrelatedFile = Join-Path $installDirectory 'unrelated.txt'
    Set-Content -LiteralPath $unrelatedFile -Value 'Keep this file.'

    & $installer -InstallDirectory $installDirectory -NoPath
    if ((Get-Content -LiteralPath $installedScript -Raw) -cne $downloadState.Source) {
        throw 'Installed content differs from the download.'
    }
    $license = (Get-Content -LiteralPath (Join-Path $repoRoot 'LICENSE') -Raw).Replace("`r`n", "`n").TrimEnd()
    if (-not $downloadState.Source.Contains($license)) { throw 'Installed script must include the full MIT license.' }
    if ($downloadState.Uris[0] -cne 'https://raw.githubusercontent.com/uni-kakurenbo/wslc-simple-gcc/main/Run-Gcc16.ps1') {
        throw 'Unexpected download URL.'
    }
    if ($env:Path -cne $originalProcessPath -or [Environment]::GetEnvironmentVariable('Path', 'User') -cne $originalUserPath) {
        throw '-NoPath changed PATH.'
    }
    Write-Host 'PASS install in a Unicode/space path, embedded license, and -NoPath'

    $downloadState.Source += "`n# Updated installation test`n"
    & $installer -InstallDirectory $installDirectory -NoPath -Ref 'v-test'
    if ((Get-Content -LiteralPath $installedScript -Raw) -cne $downloadState.Source) { throw 'Update failed.' }
    if ($downloadState.Uris[-1] -cne 'https://raw.githubusercontent.com/uni-kakurenbo/wslc-simple-gcc/v-test/Run-Gcc16.ps1') {
        throw '-Ref was not used.'
    }
    if ((Get-Content -LiteralPath $unrelatedFile -Raw).Trim() -cne 'Keep this file.') { throw 'An unrelated file was changed.' }
    Write-Host 'PASS update, explicit revision, and unrelated-file preservation'

    foreach ($mode in @('offline', 'invalid')) {
        $downloadState.Mode = $mode
        $caught = $false
        try { & $installer -InstallDirectory $installDirectory -NoPath }
        catch { $caught = $true }
        if (-not $caught) { throw "Expected installation failure for $mode." }
        if ((Get-Content -LiteralPath $installedScript -Raw) -cne $downloadState.Source) {
            throw "The existing installation changed after $mode download."
        }
        Write-Host "PASS $mode download preserves the existing installation"
    }

    $env:Path = $installDirectory + ';' + $env:Path
    $command = Get-Command wslc-simple-gcc -CommandType ExternalScript
    if ($command.Source -ine $installedScript) { throw 'PowerShell did not find the installed command by name.' }
    Write-Host 'PASS extensionless command lookup through PATH'

    if ($RunSmokeTests) {
        & (Join-Path $PSScriptRoot 'Smoke.Tests.ps1') -RunnerPath $command.Source
    }
    Write-Host 'All 5 installer checks passed.'
} finally {
    $env:Path = $originalProcessPath
    $cleanupPath = [IO.Path]::GetFullPath($testRoot)
    if (-not $cleanupPath.StartsWith($tempRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to remove a path outside the test directory: $cleanupPath"
    }
    if (Test-Path -LiteralPath $cleanupPath) {
        Remove-Item -LiteralPath $cleanupPath -Recurse -Force
    }
}
