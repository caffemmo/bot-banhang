param(
    [string]$SignatureSecret = $env:BOT_MANAGER_ARTIFACT_SIGNATURE_SECRET,
    [string]$Image = "rust:1.88-bookworm",
    [string]$ReleaseDescription = $env:BOTBANHANG_RELEASE_DESCRIPTION,
    [switch]$NoVersionBump
)

$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($SignatureSecret)) {
    throw "Missing signature secret. Pass -SignatureSecret or set BOT_MANAGER_ARTIFACT_SIGNATURE_SECRET."
}

$repo = Resolve-Path $PSScriptRoot
$managerRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
$artifactDir = Join-Path $managerRoot "artifacts"
$targetDir = Join-Path $repo "target-docker-artifact"
$bundleDir = Join-Path $targetDir "upload-bundle"
$bundleZip = Join-Path $artifactDir "botbanhang-upload.zip"
$manifestPath = Join-Path $artifactDir "artifact-manifest.json"
$signature = "BOT_MANAGER_ARTIFACT_SIGNATURE=$SignatureSecret"
$cargoTomlPath = Join-Path $repo "Cargo.toml"

function Bump-PatchVersion {
    param([Parameter(Mandatory = $true)][string]$Version)

    if ($Version -notmatch '^(\d+)\.(\d+)\.(\d+)$') {
        throw "Cargo.toml version must be in major.minor.patch format to auto-bump: $Version"
    }

    $major = [int]$Matches[1]
    $minor = [int]$Matches[2]
    $patch = [int]$Matches[3] + 1
    return "$major.$minor.$patch"
}

$cargoTomlText = [System.IO.File]::ReadAllText($cargoTomlPath)
$versionMatch = [regex]::Match($cargoTomlText, '(?m)^(version\s*=\s*")([^"]+)(")')
if (-not $versionMatch.Success) {
    throw "Could not find package version in $cargoTomlPath"
}
$currentVersion = $versionMatch.Groups[2].Value
$version = $currentVersion
if (-not $NoVersionBump) {
    $version = Bump-PatchVersion $currentVersion
    $nextCargoTomlText = $cargoTomlText.Remove($versionMatch.Groups[2].Index, $versionMatch.Groups[2].Length).Insert($versionMatch.Groups[2].Index, $version)
    [System.IO.File]::WriteAllText($cargoTomlPath, $nextCargoTomlText, [System.Text.UTF8Encoding]::new($false))
    Write-Host "Bumped botbanhang version: $currentVersion -> $version" -ForegroundColor Cyan
} else {
    Write-Host "Using botbanhang version without bump: $version" -ForegroundColor Cyan
}
$gitSha = "unknown"
try {
    $candidate = (git -C $repo rev-parse --short=12 HEAD 2>$null)
    if (-not [string]::IsNullOrWhiteSpace($candidate)) {
        $gitSha = $candidate.Trim()
    }
} catch {
    $gitSha = "unknown"
}
if ([string]::IsNullOrWhiteSpace($ReleaseDescription)) {
    $ReleaseDescription = "botbanhang $version ($gitSha)"
}

New-Item -ItemType Directory -Force -Path $artifactDir | Out-Null
New-Item -ItemType Directory -Force -Path $targetDir | Out-Null

Write-Host "Building Linux bot artifact in Docker..." -ForegroundColor Cyan
docker run --rm `
    -e "BOT_MANAGER_ARTIFACT_SIGNATURE=$signature" `
    -e "BOTBANHANG_GIT_SHA=$gitSha" `
    -v "${repo}:/work" `
    -v "${targetDir}:/work/target" `
    -w /work `
    $Image `
    bash -lc "export PATH=/usr/local/cargo/bin:`$PATH; apt-get update && apt-get install -y musl-tools && rustup target add x86_64-unknown-linux-musl && cargo build --release --target x86_64-unknown-linux-musl && strip target/x86_64-unknown-linux-musl/release/botbanhang || true"

Copy-Item -LiteralPath (Join-Path $targetDir "x86_64-unknown-linux-musl\\release\\botbanhang") -Destination (Join-Path $artifactDir "botbanhang") -Force
$binaryPath = Join-Path $artifactDir "botbanhang"
$md5 = (Get-FileHash -LiteralPath $binaryPath -Algorithm MD5).Hash.ToLowerInvariant()
$sha256 = (Get-FileHash -LiteralPath $binaryPath -Algorithm SHA256).Hash.ToLowerInvariant()
$manifest = [ordered]@{
    app = "botbanhang"
    version = $version
    description = $ReleaseDescription
    built_at = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    git_sha = $gitSha
    target = "x86_64-unknown-linux-musl"
    binary_md5 = $md5
    binary_sha256 = $sha256
}
$manifestJson = $manifest | ConvertTo-Json
[System.IO.File]::WriteAllText($manifestPath, $manifestJson, [System.Text.UTF8Encoding]::new($false))
$publicArtifact = Join-Path $artifactDir "public"
if (Test-Path -LiteralPath $publicArtifact) {
    Remove-Item -LiteralPath $publicArtifact -Recurse -Force
}
Copy-Item -LiteralPath (Join-Path $repo "public") -Destination $publicArtifact -Recurse -Force
$i18nArtifact = Join-Path $artifactDir "i18n"
if (Test-Path -LiteralPath $i18nArtifact) {
    Remove-Item -LiteralPath $i18nArtifact -Recurse -Force
}
Copy-Item -LiteralPath (Join-Path $repo "i18n") -Destination $i18nArtifact -Recurse -Force

if (Test-Path -LiteralPath $bundleDir) {
    Remove-Item -LiteralPath $bundleDir -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $bundleDir | Out-Null
Copy-Item -LiteralPath $manifestPath -Destination (Join-Path $bundleDir "artifact-manifest.json") -Force
Copy-Item -LiteralPath (Join-Path $artifactDir "public") -Destination (Join-Path $bundleDir "public") -Recurse -Force
Copy-Item -LiteralPath (Join-Path $artifactDir "i18n") -Destination (Join-Path $bundleDir "i18n") -Recurse -Force
if (Test-Path -LiteralPath $bundleZip) {
    Remove-Item -LiteralPath $bundleZip -Force
}
Compress-Archive -LiteralPath (Join-Path $bundleDir "artifact-manifest.json"), (Join-Path $bundleDir "public"), (Join-Path $bundleDir "i18n") -DestinationPath $bundleZip -CompressionLevel Optimal

Write-Host "Copied signed Linux artifacts to:" -ForegroundColor Green
Write-Host " - $artifactDir\\botbanhang"
Write-Host " - $manifestPath"
Write-Host " - $artifactDir\\public"
Write-Host " - $artifactDir\\i18n"
Write-Host " - $bundleZip"
