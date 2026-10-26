param(
    [switch]$NoOneCoreFallback,
    [switch]$EnvironmentOnly
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

function Get-MsvcToolsPathForInstall {
    param(
        [Parameter(Mandatory = $true)]
        [string]$VsInstallPath
    )

    $toolsRoot = Join-Path $VsInstallPath 'VC\Tools\MSVC'
    if (-not (Test-Path -LiteralPath $toolsRoot)) {
        return $null
    }

    $latestToolsDir = Get-ChildItem -LiteralPath $toolsRoot -Directory |
        Sort-Object Name -Descending |
        Select-Object -First 1

    if (-not $latestToolsDir) {
        return $null
    }

    return $latestToolsDir.FullName
}

function Test-VsInstallHasCompleteMsvc {
    param(
        [Parameter(Mandatory = $true)]
        [string]$VsInstallPath
    )

    $toolsPath = Get-MsvcToolsPathForInstall -VsInstallPath $VsInstallPath
    if (-not $toolsPath) {
        return $false
    }

    $requiredPaths = @(
        (Join-Path $toolsPath 'bin\HostX64\x64\cl.exe'),
        (Join-Path $toolsPath 'include\vcruntime.h')
    )

    foreach ($path in $requiredPaths) {
        if (-not (Test-Path -LiteralPath $path)) {
            return $false
        }
    }

    return (
        (Test-Path -LiteralPath (Join-Path $toolsPath 'lib\x64\msvcrt.lib')) -or
        (Test-Path -LiteralPath (Join-Path $toolsPath 'lib\onecore\x64\msvcrt.lib'))
    )
}

function Get-VsInstallPath {
    $vswhere = Get-VsWherePath
    $candidates = @()
    if ($vswhere) {
        $installationPaths = & $vswhere -all -products * -property installationPath
        if ($LASTEXITCODE -eq 0) {
            $candidates += $installationPaths |
                Where-Object { -not [string]::IsNullOrWhiteSpace($_) } |
                ForEach-Object { $_.Trim() }
        }
    }

    $candidates += @(
        'C:\Program Files\Microsoft Visual Studio\18\Community',
        'C:\Program Files\Microsoft Visual Studio\2022\Community',
        'C:\Program Files\Microsoft Visual Studio\17\Community',
        'C:\Program Files\Microsoft Visual Studio\18\BuildTools',
        'C:\Program Files\Microsoft Visual Studio\2022\BuildTools',
        'C:\Program Files\Microsoft Visual Studio\17\BuildTools'
    )

    foreach ($path in ($candidates | Select-Object -Unique)) {
        if ((Test-Path -LiteralPath $path) -and (Test-VsInstallHasCompleteMsvc -VsInstallPath $path)) {
            return $path
        }

        if (Test-Path -LiteralPath $path) {
            Write-Warning "Skipping incomplete MSVC installation: $path"
        }
    }

    throw 'No complete Visual Studio C++ installation was found. Install or repair the MSVC x64 compiler, headers, and libraries.'
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

    $toolsPath = Get-MsvcToolsPathForInstall -VsInstallPath $VsInstallPath
    if (-not $toolsPath) {
        throw "No MSVC tools directory was found under '$VsInstallPath\VC\Tools\MSVC'."
    }

    return $toolsPath
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

function Sync-ReleaseAsset {
    param(
        [Parameter(Mandatory = $true)]
        [string]$SourcePath,
        [Parameter(Mandatory = $true)]
        [string]$DestinationPath
    )

    if (Test-Path -LiteralPath $SourcePath) {
        $destinationDir = Split-Path -Parent $DestinationPath
        if (-not [string]::IsNullOrWhiteSpace($destinationDir)) {
            New-Item -ItemType Directory -Force -Path $destinationDir | Out-Null
        }

        Copy-Item -LiteralPath $SourcePath -Destination $DestinationPath -Force
        Write-Host 'Staged release asset:' $DestinationPath
    } elseif (Test-Path -LiteralPath $DestinationPath) {
        Remove-Item -LiteralPath $DestinationPath -Force
        Write-Host 'Removed stale release asset:' $DestinationPath
    }
}

function Sync-ReleaseAssets {
    param(
        [Parameter(Mandatory = $true)]
        [string]$RepoRoot
    )

    $releaseDir = Join-Path $RepoRoot 'target\release'
    $sourceAssetsDir = Join-Path $RepoRoot 'assets'
    $releaseAssetsDir = Join-Path $releaseDir 'assets'

    New-Item -ItemType Directory -Force -Path $releaseAssetsDir | Out-Null

    Sync-ReleaseAsset `
        -SourcePath (Join-Path $sourceAssetsDir 'tray-icon.ico') `
        -DestinationPath (Join-Path $releaseAssetsDir 'tray-icon.ico')
    Sync-ReleaseAsset `
        -SourcePath (Join-Path $sourceAssetsDir 'tray-icon.png') `
        -DestinationPath (Join-Path $releaseAssetsDir 'tray-icon.png')
    Sync-ReleaseAsset `
        -SourcePath (Join-Path $sourceAssetsDir 'app-icon.ico') `
        -DestinationPath (Join-Path $releaseDir 'app-icon.ico')
}

function Get-ReleaseSourceSnapshot {
    param([Parameter(Mandatory = $true)][string]$Root)

    $files = @('Cargo.toml', 'Cargo.lock', 'build.rs') | ForEach-Object { Join-Path $Root $_ }
    foreach ($directory in @('src', 'assets')) {
        $files += Get-ChildItem -LiteralPath (Join-Path $Root $directory) -File -Recurse |
            Select-Object -ExpandProperty FullName
    }
    @($files | Sort-Object -Unique | ForEach-Object {
        [ordered]@{
            path = $_.Substring($Root.Length + 1).Replace('\', '/')
            sha256 = (Get-FileHash -LiteralPath $_ -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    })
}

$repoRoot = $PSScriptRoot
$releaseExe = Join-Path $repoRoot 'target\release\wakemate-companion.exe'

try {
    Push-Location $repoRoot

    $vsInstallPath = Get-VsInstallPath
    $vsDevCmdPath = Get-VsDevCmdPath -VsInstallPath $vsInstallPath

    Write-Host 'Using Visual Studio:' $vsInstallPath
    $vswherePath = Get-VsWherePath
    if ($vswherePath) {
        $vswhereDir = Split-Path -Parent $vswherePath
        if (($env:PATH -split ';') -notcontains $vswhereDir) {
            $env:PATH = $vswhereDir + ';' + $env:PATH
        }
    }
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

    if ($EnvironmentOnly) { return }

    $sourceBefore = @(Get-ReleaseSourceSnapshot -Root $repoRoot)
    & cargo build --release --locked

    if ($LASTEXITCODE -ne 0) {
        throw "cargo build --release failed with exit code $LASTEXITCODE."
    }

    Sync-ReleaseAssets -RepoRoot $repoRoot
    $sourceAfter = @(Get-ReleaseSourceSnapshot -Root $repoRoot)
    if ((ConvertTo-Json -InputObject $sourceBefore -Depth 4 -Compress) -ne
        (ConvertTo-Json -InputObject $sourceAfter -Depth 4 -Compress)) {
        throw 'Sources changed during the release build. Re-run after source edits finish; no build provenance was published.'
    }
    $metadataText = & cargo metadata --no-deps --locked --format-version 1
    if ($LASTEXITCODE -ne 0) { throw 'Could not read locked Cargo metadata.' }
    $metadata = $metadataText | ConvertFrom-Json
    $package = @($metadata.packages | Where-Object name -eq 'wakemate-companion')[0]
    $expectedVersion = "$($package.version).0"
    $binaryInfo = [Diagnostics.FileVersionInfo]::GetVersionInfo($releaseExe)
    $binaryVersion = "$($binaryInfo.FileMajorPart).$($binaryInfo.FileMinorPart).$($binaryInfo.FileBuildPart).$($binaryInfo.FilePrivatePart)"
    if ($binaryVersion -ne $expectedVersion) {
        throw "Release executable version '$binaryVersion' does not match '$expectedVersion'."
    }
    $revision = (& git rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Could not resolve the source revision.' }
    $dirty = [bool](& git status --porcelain)
    if ($LASTEXITCODE -ne 0) { throw 'Could not inspect the source worktree.' }
    $provenance = [ordered]@{
        schema_version = 1
        package = 'wakemate-companion'
        version = $package.version
        target = 'x86_64-pc-windows-msvc'
        built_at_utc = [DateTime]::UtcNow.ToString('o')
        git_revision = $revision
        worktree_dirty = $dirty
        executable_sha256 = (Get-FileHash -LiteralPath $releaseExe -Algorithm SHA256).Hash.ToLowerInvariant()
        source_files = $sourceAfter
    }
    $provenance | ConvertTo-Json -Depth 6 |
        Set-Content -LiteralPath (Join-Path $repoRoot 'target\release\wakemate-companion.build.json') -Encoding UTF8
    Write-Host 'Release build completed:' $releaseExe
} finally {
    Pop-Location
}
