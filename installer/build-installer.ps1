param(
    # Compatibility names never bypass artifact/source verification.
    [switch]$SkipPrereqCheck,
    [switch]$BuildRelease,
    [switch]$UseVerifiedBuild
)

$ErrorActionPreference = 'Stop'

function Find-Iscc {
    $commonPaths = @(
        'C:\Program Files (x86)\Inno Setup 6\ISCC.exe',
        'C:\Program Files\Inno Setup 6\ISCC.exe',
        (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe')
    )
    foreach ($path in $commonPaths) {
        if (Test-Path -LiteralPath $path) { return $path }
    }
    $command = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    if ($command) { return $command.Source }
    throw 'Inno Setup compiler (ISCC.exe) was not found. Install Inno Setup 6 before packaging.'
}

$installerDir = $PSScriptRoot
$repoRoot = (Resolve-Path -LiteralPath (Join-Path $installerDir '..')).Path
$issFile = Join-Path $installerDir 'wakemate-companion.iss'
$releaseExe = Join-Path $repoRoot 'target\release\wakemate-companion.exe'
$buildScript = Join-Path $repoRoot 'build-release.ps1'
$buildRecord = Join-Path $repoRoot 'target\release\wakemate-companion.build.json'
$vcRedist = Join-Path $installerDir 'redist\VC_redist.x64.exe'
$iscc = Find-Iscc

if ($SkipPrereqCheck) {
    Write-Warning '-SkipPrereqCheck is deprecated. Existing builds now require matching source provenance.'
    $UseVerifiedBuild = $true
}
if (-not $UseVerifiedBuild -or $BuildRelease) {
    & $buildScript
    if ($LASTEXITCODE -ne 0) { throw "Release build failed with exit code $LASTEXITCODE." }
}
foreach ($required in @($issFile, $releaseExe, $buildRecord, $vcRedist)) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
        throw "Required packaging input is missing: $required"
    }
}

$record = Get-Content -LiteralPath $buildRecord -Raw | ConvertFrom-Json
if ($record.schema_version -ne 1 -or $record.package -ne 'wakemate-companion') {
    throw 'Unsupported release-build provenance. Run build-release.ps1 first.'
}
$metadataText = & cargo metadata --manifest-path (Join-Path $repoRoot 'Cargo.toml') --no-deps --locked --format-version 1
if ($LASTEXITCODE -ne 0) { throw 'Could not read locked package metadata.' }
$metadata = $metadataText | ConvertFrom-Json
$version = (@($metadata.packages | Where-Object name -eq 'wakemate-companion')[0]).version
$installerMatch = [regex]::Match((Get-Content -LiteralPath $issFile -Raw), '(?m)^#define\s+MyAppVersion\s+"([^"]+)"\s*$')
if (-not $installerMatch.Success -or $installerMatch.Groups[1].Value -ne $version -or $record.version -ne $version) {
    throw 'Cargo, installer, and release-build provenance versions must match.'
}
$binaryHash = (Get-FileHash -LiteralPath $releaseExe -Algorithm SHA256).Hash.ToLowerInvariant()
if ($binaryHash -ne $record.executable_sha256) {
    throw 'The release executable changed since its verified build. Rebuild before packaging.'
}
$currentPaths = @('Cargo.toml', 'Cargo.lock', 'build.rs')
foreach ($directory in @('src', 'assets')) {
    $currentPaths += Get-ChildItem -LiteralPath (Join-Path $repoRoot $directory) -File -Recurse |
        ForEach-Object { $_.FullName.Substring($repoRoot.Length + 1).Replace('\', '/') }
}
$recordedPaths = @($record.source_files | ForEach-Object path)
if (@(Compare-Object ($currentPaths | Sort-Object) ($recordedPaths | Sort-Object)).Count -ne 0) {
    throw 'The release source file set changed. Rebuild before packaging.'
}
foreach ($source in $record.source_files) {
    $sourcePath = Join-Path $repoRoot $source.path
    if ((Get-FileHash -LiteralPath $sourcePath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $source.sha256) {
        throw "Source changed after the release build: $($source.path). Rebuild before packaging."
    }
}
$signature = Get-AuthenticodeSignature -LiteralPath $vcRedist
if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch '(^|,\s*)O=Microsoft Corporation(,|$)') {
    throw 'The bundled Visual C++ redistributable must have a valid Microsoft signature.'
}

# A fresh directory preserves every previous installer and prevents a stale
# same-version artifact from being mistaken for this run's output.
$runName = "$version-$([DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfffZ'))-$([guid]::NewGuid().ToString('N').Substring(0,8))"
$outputDir = Join-Path $repoRoot "dist\installer\$runName"
New-Item -ItemType Directory -Path $outputDir | Out-Null
& $iscc "/O$outputDir" $issFile
if ($LASTEXITCODE -ne 0) { throw "Installer compilation failed with exit code $LASTEXITCODE." }
$installer = Join-Path $outputDir "WakeMATE-Companion-Setup-v$version.exe"
if (-not (Test-Path -LiteralPath $installer -PathType Leaf)) { throw 'ISCC did not produce the expected installer.' }
$installerHash = (Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash.ToLowerInvariant()
$checksumPath = "$installer.sha256"
"$installerHash  $([IO.Path]::GetFileName($installer))" | Set-Content -LiteralPath $checksumPath -Encoding ASCII
$provenancePath = "$installer.provenance.json"
[ordered]@{
    schema_version = 1
    version = $version
    packaged_at_utc = [DateTime]::UtcNow.ToString('o')
    installer_sha256 = $installerHash
    installer_source_sha256 = (Get-FileHash -LiteralPath $issFile -Algorithm SHA256).Hash.ToLowerInvariant()
    redist_sha256 = (Get-FileHash -LiteralPath $vcRedist -Algorithm SHA256).Hash.ToLowerInvariant()
    executable = $record
    signing = 'unsigned development artifact'
} | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $provenancePath -Encoding UTF8
$latest = [ordered]@{ installer = $installer; checksum = $checksumPath; provenance = $provenancePath }
$latest | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $repoRoot 'target\installer-latest.json') -Encoding UTF8
if ($env:GITHUB_OUTPUT) {
    "installer_path=$installer" | Add-Content -LiteralPath $env:GITHUB_OUTPUT
    "checksum_path=$checksumPath" | Add-Content -LiteralPath $env:GITHUB_OUTPUT
    "provenance_path=$provenancePath" | Add-Content -LiteralPath $env:GITHUB_OUTPUT
}
Write-Host "Installer completed: $installer"
Write-Host "SHA-256: $installerHash"
