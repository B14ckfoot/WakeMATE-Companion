param(
    [switch]$NoOneCoreFallback
)

$ErrorActionPreference = 'Stop'

function Get-VsWherePath {
    $candidates = @(
        'C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe',
        'C:\Program Files\Microsoft Visual Studio\Installer\vswhere.exe'
    )

    foreach ($path in $candidates) {
        if (Test-Path -LiteralPath $path) {
            return $path
        }
    }

    return $null
}

function Get-VsInstallPath {
    $vswhere = Get-VsWherePath
    if ($vswhere) {
        $installationPath = & $vswhere -latest -products * -property installationPath
        if ($LASTEXITCODE -eq 0 -and -not [string]::IsNullOrWhiteSpace($installationPath)) {
            return $installationPath.Trim()
        }
    }

    $commonPaths = @(
        'C:\Program Files\Microsoft Visual Studio\18\Community',
        'C:\Program Files\Microsoft Visual Studio\17\Community',
        'C:\Program Files\Microsoft Visual Studio\18\BuildTools',
        'C:\Program Files\Microsoft Visual Studio\17\BuildTools'
    )

    foreach ($path in $commonPaths) {
        if (Test-Path -LiteralPath $path) {
            return $path
        }
    }

    throw 'Visual Studio was not found. Install Visual Studio or Build Tools with the MSVC x64 toolchain.'
}

function Get-VsDevCmdPath {
    param(
        [Parameter(Mandatory = $true)]
        [string]$VsInstallPath
    )

    $candidates = @(
        (Join-Path $VsInstallPath 'Common7\Tools\VsDevCmd.bat'),
        (Join-Path $VsInstallPath 'VC\Auxiliary\Build\vcvars64.bat')
    )

    foreach ($path in $candidates) {
        if (Test-Path -LiteralPath $path) {
            return $path
        }
    }

    throw "Could not find a Visual Studio developer command script under '$VsInstallPath'."
}

function Import-BatchEnvironment {
    param(
        [Parameter(Mandatory = $true)]
        [string]$BatchPath,
        [string]$Arguments = ''
    )

    $quotedBatchPath = '"' + $BatchPath + '"'
    $command = if ([string]::IsNullOrWhiteSpace($Arguments)) {
        "call $quotedBatchPath >nul && set"
    } else {
        "call $quotedBatchPath $Arguments >nul && set"
    }

    $environmentDump = & cmd.exe /d /c $command
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to import the Visual Studio developer environment from '$BatchPath'."
    }

    foreach ($line in $environmentDump) {
        if ($line -match '^(.*?)=(.*)$') {
            [Environment]::SetEnvironmentVariable($matches[1], $matches[2], 'Process')
        }
    }
}

function Get-MsvcToolsPath {
    param(
        [Parameter(Mandatory = $true)]
        [string]$VsInstallPath
    )

    if (-not [string]::IsNullOrWhiteSpace($env:VCToolsInstallDir) -and (Test-Path -LiteralPath $env:VCToolsInstallDir)) {
        return $env:VCToolsInstallDir.TrimEnd('\')
    }

    $toolsRoot = Join-Path $VsInstallPath 'VC\Tools\MSVC'
    $latestToolsDir = Get-ChildItem -LiteralPath $toolsRoot -Directory |
        Sort-Object Name -Descending |
        Select-Object -First 1

    if (-not $latestToolsDir) {
        throw "No MSVC tools directory was found under '$toolsRoot'."
    }

    return $latestToolsDir.FullName
}

function Test-LibraryOnLibPath {
    param(
        [Parameter(Mandatory = $true)]
        [string]$LibraryName
    )

    if ([string]::IsNullOrWhiteSpace($env:LIB)) {
        return $false
    }

    foreach ($path in ($env:LIB -split ';')) {
        if (-not [string]::IsNullOrWhiteSpace($path) -and (Test-Path -LiteralPath (Join-Path $path $LibraryName))) {
            return $true
        }
    }

    return $false
}

function Prepend-LibPath {
    param(
        [Parameter(Mandatory = $true)]
        [string]$PathToAdd
    )

    $existing = @()
    if (-not [string]::IsNullOrWhiteSpace($env:LIB)) {
        $existing = $env:LIB -split ';' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
    }

    if ($existing -contains $PathToAdd) {
        return
    }

    $env:LIB = if ($existing.Count -gt 0) {
        $PathToAdd + ';' + ($existing -join ';')
    } else {
        $PathToAdd
    }
}

$repoRoot = $PSScriptRoot
$releaseExe = Join-Path $repoRoot 'target\release\wakemate-companion.exe'

try {
    Push-Location $repoRoot

    $vsInstallPath = Get-VsInstallPath
    $vsDevCmdPath = Get-VsDevCmdPath -VsInstallPath $vsInstallPath

    Write-Host 'Using Visual Studio:' $vsInstallPath
    Import-BatchEnvironment -BatchPath $vsDevCmdPath -Arguments '-arch=x64'

    $msvcToolsPath = Get-MsvcToolsPath -VsInstallPath $vsInstallPath
    $desktopLibPath = Join-Path $msvcToolsPath 'lib\x64'
    $oneCoreLibPath = Join-Path $msvcToolsPath 'lib\onecore\x64'

    if (-not (Test-LibraryOnLibPath -LibraryName 'msvcrt.lib')) {
        if (Test-Path -LiteralPath (Join-Path $desktopLibPath 'msvcrt.lib')) {
            Prepend-LibPath -PathToAdd $desktopLibPath
            Write-Host 'Added MSVC desktop library path:' $desktopLibPath
        } elseif (-not $NoOneCoreFallback -and (Test-Path -LiteralPath (Join-Path $oneCoreLibPath 'msvcrt.lib'))) {
            Prepend-LibPath -PathToAdd $oneCoreLibPath
            Write-Warning "Using MSVC onecore library fallback: $oneCoreLibPath"
            Write-Warning 'Repair the Visual Studio C++ desktop libraries if you want the standard lib\x64 layout.'
        }
    }

    if (-not (Test-LibraryOnLibPath -LibraryName 'msvcrt.lib')) {
        throw "msvcrt.lib is still unavailable. Repair the Visual Studio C++ x64 desktop libraries and re-run this script."
    }

    & cargo build --release

    if ($LASTEXITCODE -ne 0) {
        throw "cargo build --release failed with exit code $LASTEXITCODE."
    }

    Write-Host 'Release build completed:' $releaseExe
} finally {
    Pop-Location
}
