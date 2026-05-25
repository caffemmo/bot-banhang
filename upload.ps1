$ErrorActionPreference = "Stop"

param(
    [string]$User = "root",
    [string]$Host = "[IP_ADDRESS]",
    [string]$RemoteDir = "/opt/botbanhang"
)

function Invoke-Scp {
    param(
        [string[]]$Source,
        [string]$Destination,
        [switch]$Recursive
    )

    $scpArgs = @()
    if ($Recursive) {
        $scpArgs += "-r"
    }
    $scpArgs += $Source
    $scpArgs += $Destination

    Write-Host "Running: scp $($scpArgs -join ' ')" -ForegroundColor Cyan
    & scp @scpArgs
}

$target = "$User@$Host:$RemoteDir"

Invoke-Scp -Source "bot_clone.sh" -Destination $target
Invoke-Scp -Source "bot_update.sh" -Destination $target
Invoke-Scp -Source "bot_list.sh" -Destination $target
Invoke-Scp -Source ".env" -Destination $target
Invoke-Scp -Source "public" -Destination "$target/public" -Recursive
