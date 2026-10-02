#requires -Version 7.3
<#
.SYNOPSIS
Compile and run C or C++ using GCC 16 in a disposable WSL container.
.DESCRIPTION
Requires Windows, PowerShell 7.3+, and WSL Containers (wslc.exe).
Docker Desktop, dockerd, and a user-installed Linux distribution are not required.
The image is downloaded on first use and cached. The container and Linux binary
are removed on exit. The project is mounted read-only at /src, which is also
the working directory; programs can write temporary files under /tmp.

All paths in Source and ProjectDirectory are resolved from the caller's directory.
ProjectDirectory defaults to the first source's parent; every source must be inside
it. CompilerArgs paths are Linux paths relative to /src (for example -Iinclude).
Use homogeneous C or C++ source sets; mixed-language builds require a build system.
The default standards are C23 and C++23. The actual compiler must have major version 16.

The exit code is the compiler's code on build failure, or the program's code after
a successful build. Wrapper validation errors return 2. WSL errors propagate.
Use -Interactive for console stdin and -Tty when a terminal is needed.
.EXAMPLE
./Run-Gcc16.ps1 ./examples/hello.c
.EXAMPLE
./Run-Gcc16.ps1 ./examples/hello.cpp -Standard c++26 -CompilerArgs @('-O2','-pthread') -RunArgs @('hello world','42')
.EXAMPLE
./Run-Gcc16.ps1 -Source @('./src/main.cpp','./src/util.cpp') -ProjectDirectory . -CompilerArgs @('-Iinclude','-O2')
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory, Position = 0)]
    [ValidateNotNullOrEmpty()]
    [string[]] $Source,

    [string] $ProjectDirectory,

    [ValidateSet('auto', 'c', 'cpp')]
    [string] $Language = 'auto',

    [string] $Standard,
    [string[]] $CompilerArgs = @(),
    [AllowEmptyString()]
    [string[]] $RunArgs = @(),
    [ValidateNotNullOrEmpty()]
    [string] $Image = 'docker.io/library/gcc:16.2.0',
    [string] $WslcPath,
    [switch] $Interactive,
    [switch] $Tty
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
# Preserve argument boundaries, quotes, and empty arguments across the native CLI.
$PSNativeCommandArgumentPassing = 'Standard'
$PSNativeCommandUseErrorActionPreference = $false

try {
    if (-not $IsWindows) { throw 'Run this script from PowerShell on Windows.' }

    if ($WslcPath) {
        $wslc = (Get-Item -LiteralPath $WslcPath).FullName
    } else {
        $command = Get-Command wslc.exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($command) {
            $wslc = $command.Source
        } else {
            # WSL's installation directory is not always on PATH.
            $wslc = Join-Path $env:ProgramFiles 'WSL/wslc.exe'
        }
    }
    if (-not (Test-Path -LiteralPath $wslc -PathType Leaf)) {
        throw 'wslc.exe was not found. Install/update WSL with wsl --update, or specify -WslcPath.'
    }

    $sourceFiles = @(foreach ($file in $Source) {
        $item = Get-Item -LiteralPath $file
        if ($item.PSIsContainer -or $item.PSProvider.Name -ne 'FileSystem') {
            throw "Source is not a file: $file"
        }
        $item
    })

    if (-not $ProjectDirectory) { $ProjectDirectory = $sourceFiles[0].DirectoryName }
    $root = Get-Item -LiteralPath $ProjectDirectory
    if (-not $root.PSIsContainer -or $root.PSProvider.Name -ne 'FileSystem') {
        throw 'ProjectDirectory must be a filesystem directory.'
    }
    # The volume syntax handles a Windows drive colon and paths with spaces.
    # Colons elsewhere would be ambiguous to the volume parser.
    if ($root.FullName -notmatch '^[A-Za-z]:\\' -or $root.FullName.Substring(2).Contains(':')) {
        throw 'ProjectDirectory must be a local Windows drive path, such as C:\projects\demo.'
    }

    $containerSources = @(foreach ($file in $sourceFiles) {
        $relative = [IO.Path]::GetRelativePath($root.FullName, $file.FullName)
        if ($relative -eq '..' -or $relative.StartsWith('..\') -or [IO.Path]::IsPathRooted($relative)) {
            throw "Source is outside ProjectDirectory: $($file.FullName)"
        }
        '/src/' + $relative.Replace('\', '/')
    })

    if ($Language -eq 'auto') {
        $languages = @(foreach ($file in $sourceFiles) {
            if ($file.Extension -ceq '.c') { 'c' }
            elseif ($file.Extension -ceq '.C' -or $file.Extension -in @('.cc', '.cpp', '.cxx', '.c++')) { 'cpp' }
            else { throw "Cannot infer language for $($file.Name). Specify -Language c or cpp." }
        })
        $languages = @($languages | Select-Object -Unique)
        if ($languages.Count -ne 1) {
            throw 'Mixed C/C++ sources are not supported in auto mode. Compile them separately with a build system.'
        }
        $Language = $languages[0]
    }

    $compiler = if ($Language -eq 'c') { 'gcc' } else { 'g++' }
    $gccLanguage = if ($Language -eq 'c') { 'c' } else { 'c++' }
    if (-not $Standard) { $Standard = if ($Language -eq 'c') { 'c23' } else { 'c++23' } }

    # This script is constant. Paths, flags, and application arguments travel only
    # as positional arguments, never as interpolated shell code.
    $runner = @'
set -euo pipefail
compiler=$1
language=$2
standard=$3
shift 3
version=$("$compiler" -dumpfullversion -dumpversion)
if [[ ${version%%.*} != 16 ]]; then
    printf 'Expected GCC 16, found %s\n' "$version" >&2
    exit 2
fi
printf '[GCC %s | %s]\n' "$version" "$standard" >&2
count=$1
shift
sources=("${@:1:count}")
shift "$count"
count=$1
shift
flags=("${@:1:count}")
shift "$count"
build_dir=$(mktemp -d /tmp/wsl-gcc16.XXXXXXXX)
"$compiler" -x "$language" "-std=$standard" -Wall -Wextra "${sources[@]}" "${flags[@]}" -o "$build_dir/program"
exec "$build_dir/program" "$@"
'@
    $runner = $runner.Replace("`r`n", "`n")

    $runOptions = @('run', '--rm', '--pull', 'missing', '--volume', "$($root.FullName):/src:ro", '--workdir', '/src')
    if ($Interactive -or $Tty) { $runOptions += '--interactive' }
    if ($Tty) { $runOptions += '--tty' }
    $runOptions += @('--entrypoint', '/bin/bash', $Image, '-c', $runner, 'gcc16-runner', $compiler, $gccLanguage, $Standard)
    $runOptions += [string]$containerSources.Count
    $runOptions += $containerSources
    $runOptions += [string]$CompilerArgs.Count
    $runOptions += $CompilerArgs
    $runOptions += $RunArgs

    & $wslc @runOptions
    exit $LASTEXITCODE
} catch {
    Write-Error $_ -ErrorAction Continue
    exit 2
}
