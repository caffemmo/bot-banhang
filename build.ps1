param(
    [string]$SignatureSecret = $env:BOT_MANAGER_ARTIFACT_SIGNATURE_SECRET,
    [switch]$CopyToManagerArtifacts
)

$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($SignatureSecret)) {
    throw "Missing signature secret. Pass -SignatureSecret or set BOT_MANAGER_ARTIFACT_SIGNATURE_SECRET."
}

$env:BOT_MANAGER_ARTIFACT_SIGNATURE = "BOT_MANAGER_ARTIFACT_SIGNATURE=$SignatureSecret"

Write-Host "Building Windows (native) with artifact signature..." -ForegroundColor Cyan
cargo build --release

Write-Host "Done. Artifacts:"
Write-Host " - target\\release\\botbanhang.exe (Windows)"

if ($CopyToManagerArtifacts) {
    $managerRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
    $artifactDir = Join-Path $managerRoot "artifacts"
    New-Item -ItemType Directory -Force -Path $artifactDir | Out-Null
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot "target\\release\\botbanhang.exe") -Destination (Join-Path $artifactDir "botbanhang.exe") -Force
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot "public") -Destination (Join-Path $artifactDir "public") -Recurse -Force
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot "i18n") -Destination (Join-Path $artifactDir "i18n") -Recurse -Force
    Write-Host "Copied Windows artifact to:"
    Write-Host " - $artifactDir\\botbanhang.exe"
    Write-Host " - $artifactDir\\public"
    Write-Host " - $artifactDir\\i18n"
}
