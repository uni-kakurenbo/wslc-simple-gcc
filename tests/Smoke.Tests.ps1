#requires -Version 7.3
<#
.SYNOPSIS
Run integration checks against real WSL Containers. No test framework is needed.
#>
[CmdletBinding()]
param(
    [string] $RunnerPath = (Join-Path (Split-Path $PSScriptRoot -Parent) 'Run-Gcc16.ps1')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$repoRoot = Split-Path $PSScriptRoot -Parent
$runner = (Get-Item -LiteralPath $RunnerPath).FullName
$tempRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot '.tmp'))
$testRoot = Join-Path $tempRoot ('smoke-' + [guid]::NewGuid().ToString('N'))
$fixtureRoot = Join-Path $testRoot 'test project 日本語'

function Assert-Run {
    param(
        [string] $Name,
        [hashtable] $Parameters,
        [int] $ExpectedExit = 0,
        [string[]] $ExpectedLines = @()
    )

    $result = @(& $runner @Parameters)
    $actualExit = $LASTEXITCODE
    if ($actualExit -ne $ExpectedExit) {
        throw "$Name : expected exit $ExpectedExit, got $actualExit"
    }
    foreach ($line in $ExpectedLines) {
        if ($line -cnotin $result) {
            throw "$Name : missing output <$line>; got <$($result -join '`n')>"
        }
    }
    Write-Host "PASS $Name"
}

try {
    if (-not $IsWindows) { throw 'These integration tests require Windows and WSL Containers.' }
    New-Item -ItemType Directory -Path (Join-Path $fixtureRoot 'include') -Force | Out-Null

    Assert-Run -Name 'C23 sample' -Parameters @{
        Source = Join-Path $repoRoot 'examples/hello.c'
        CompilerArgs = @('-Werror')
        RunArgs = @('C sample')
    } -ExpectedLines @('argv[1]=<C sample>')

    Assert-Run -Name 'C++23 and exact argument boundaries' -Parameters @{
        Source = Join-Path $repoRoot 'examples/hello.cpp'
        CompilerArgs = @('-O2', '-Werror')
        RunArgs = @('hello world', '', 'quote"test', 'semi;colon', '$(not-a-command)')
    } -ExpectedLines @(
        'argv[1]=<hello world>',
        'argv[2]=<>',
        'argv[3]=<quote"test>',
        'argv[4]=<semi;colon>',
        'argv[5]=<$(not-a-command)>'
    )

    Set-Content -LiteralPath (Join-Path $fixtureRoot 'exit code.c') -Value 'int main(void) { return 37; }' -Encoding utf8NoBOM
    Assert-Run -Name 'Program exit code and Unicode/space path' -Parameters @{
        Source = Join-Path $fixtureRoot 'exit code.c'
    } -ExpectedExit 37

    Set-Content -LiteralPath (Join-Path $fixtureRoot 'bad.c') -Value '#error expected_compile_failure' -Encoding utf8NoBOM
    Assert-Run -Name 'Compile failure exit code (diagnostic expected)' -Parameters @{
        Source = Join-Path $fixtureRoot 'bad.c'
    } -ExpectedExit 1

    Set-Content -LiteralPath (Join-Path $fixtureRoot 'include/value.h') -Value 'int value(void);' -Encoding utf8NoBOM
    Set-Content -LiteralPath (Join-Path $fixtureRoot 'value.c') -Value 'int value(void) { return 42; }' -Encoding utf8NoBOM
    Set-Content -LiteralPath (Join-Path $fixtureRoot 'main.c') -Value @'
#include <value.h>
#include <stdio.h>
#include <string.h>
int main(void) {
    if (value() != 42 || strcmp(MESSAGE, "hello world") != 0) return 90;
    FILE *file = fopen("must-not-be-created.txt", "w");
    if (file) { fclose(file); return 91; }
    puts("multiple sources, include path, quoted macro, read-only mount: OK");
    return 0;
}
'@ -Encoding utf8NoBOM
    Assert-Run -Name 'Multiple sources, include path, quoted flag, read-only mount' -Parameters @{
        Source = @((Join-Path $fixtureRoot 'main.c'), (Join-Path $fixtureRoot 'value.c'))
        ProjectDirectory = $fixtureRoot
        CompilerArgs = @('-Iinclude', '-DMESSAGE="hello world"', '-Werror')
    } -ExpectedLines @('multiple sources, include path, quoted macro, read-only mount: OK')
    if (Test-Path -LiteralPath (Join-Path $fixtureRoot 'must-not-be-created.txt')) {
        throw 'The read-only project directory was modified.'
    }

    Assert-Run -Name 'C++26 sample' -Parameters @{
        Source = Join-Path $repoRoot 'examples/hello.cpp'
        Standard = 'c++26'
        RunArgs = @('C++26')
    } -ExpectedLines @('argv[1]=<C++26>')

    Write-Host 'All 6 integration checks passed.'
} finally {
    # Only remove this run's unique fixture directory beneath the repository.
    $cleanupPath = [IO.Path]::GetFullPath($testRoot)
    $allowedPrefix = $tempRoot + [IO.Path]::DirectorySeparatorChar
    if (-not $cleanupPath.StartsWith($allowedPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to remove a path outside the test directory: $cleanupPath"
    }
    if (Test-Path -LiteralPath $cleanupPath) {
        Remove-Item -LiteralPath $cleanupPath -Recurse -Force
    }
}
