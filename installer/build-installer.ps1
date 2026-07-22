param(
    [switch]$SkipPrereqCheck,
    [switch]$BuildRelease
)

$ErrorActionPreference = 'Stop'

function Find-Iscc {
    $commonPaths = @(
        'C:\Program Files (x86)\Inno Setup 6\ISCC.exe',
        'C:\Program Files\Inno Setup 6\ISCC.exe'
    )

    foreach ($path in $commonPaths) {
        if (Test-Path -LiteralPath $path) {
            return $path
        }
    }

    $command = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    if ($command) {
        return $command.Source
    }

    return $null
}

$installerDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Resolve-Path (Join-Path $installerDir '..')
$issFile = Join-Path $installerDir 'wakemate-companion.iss'
$releaseExe = Join-Path $repoRoot 'target\release\wakemate-companion.exe'
$buildReleaseScript = Join-Path $repoRoot 'build-release.ps1'
$vcRedist = Join-Path $installerDir 'redist\VC_redist.x64.exe'
$iscc = Find-Iscc

if (-not (Test-Path -LiteralPath $issFile)) {
    throw "Installer script not found: $issFile"
}

if (-not $SkipPrereqCheck) {
    if ($BuildRelease -or -not (Test-Path -LiteralPath $releaseExe)) {
        if (-not (Test-Path -LiteralPath $buildReleaseScript)) {
            throw "Release binary not found at '$releaseExe', and build helper was not found at '$buildReleaseScript'."
        }

        Write-Host "Building release binary:" $buildReleaseScript
        & $buildReleaseScript

        if ($LASTEXITCODE -ne 0) {
            throw "Release build failed with exit code $LASTEXITCODE."
        }
    }

    if (-not (Test-Path -LiteralPath $releaseExe)) {
        throw "Release binary not found at '$releaseExe'. Build the app first with '.\\build-release.ps1'."
    }

    if (-not (Test-Path -LiteralPath $vcRedist)) {
        throw "Visual C++ redistributable not found at '$vcRedist'."
    }
}

if (-not $iscc) {
    throw "Inno Setup compiler (ISCC.exe) was not found. Install Inno Setup 6 and re-run this script."
}

Write-Host "Using ISCC:" $iscc
Write-Host "Compiling installer:" $issFile

& $iscc $issFile

if ($LASTEXITCODE -ne 0) {
    throw "Installer build failed with exit code $LASTEXITCODE."
}

Write-Host "Installer build completed."
