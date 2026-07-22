<#
.SYNOPSIS
    Single entry point for formatting, linting, type-checking, and tests --
    the same gate CI runs on every push, runnable locally before you commit.

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\quality-check.ps1
#>

$ErrorActionPreference = 'Stop'
$repoRoot = Resolve-Path (Join-Path $PSScriptRoot '..')

Push-Location $repoRoot
try {
    Write-Host '==> cargo fmt --check' -ForegroundColor Cyan
    cargo fmt --all -- --check
    if ($LASTEXITCODE -ne 0) { throw 'cargo fmt found unformatted code. Run "cargo fmt --all" to fix it.' }

    Write-Host '==> cargo clippy (warnings are errors)' -ForegroundColor Cyan
    cargo clippy --release --all-targets --locked -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw 'cargo clippy reported issues.' }

    Write-Host '==> cargo test' -ForegroundColor Cyan
    cargo test --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'cargo test failed.' }

    Write-Host '==> cargo build --release' -ForegroundColor Cyan
    cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed.' }

    Write-Host ''
    Write-Host 'All quality checks passed.' -ForegroundColor Green
} finally {
    Pop-Location
}
