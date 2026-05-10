param(
    [switch] $SkipFrontend,
    [switch] $SkipNpmInstall,
    [switch] $Release,
    [string] $Config = ""
)

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$frontend = Join-Path $root "frontend"

function Find-CommandPath([string[]] $Names, [string] $InstallHint) {
    foreach ($name in $Names) {
        $cmd = Get-Command $name -ErrorAction SilentlyContinue
        if ($cmd) {
            return $cmd.Source
        }
    }
    throw "$($Names -join '/') not found. $InstallHint"
}

function Find-Npm {
    $preferred = "D:\Program Files\node.js\npm.cmd"
    if (Test-Path $preferred) {
        return $preferred
    }
    return Find-CommandPath @("npm.cmd", "npm") "Install Node.js 20+ and add npm to PATH."
}

function Test-PortListening([int] $port) {
    $conn = Get-NetTCPConnection -State Listen -LocalPort $port -ErrorAction SilentlyContinue
    return $null -ne $conn
}

function Test-LfsPointer([string] $path) {
    if (-not (Test-Path $path)) {
        return $false
    }
    $item = Get-Item $path
    if ($item.Length -gt 1024) {
        return $false
    }
    $head = Get-Content -Path $path -TotalCount 1 -ErrorAction SilentlyContinue
    return $head -like "version https://git-lfs.github.com/spec/v1*"
}

function Ensure-GitLfsFiles {
    $weights = @(
        (Join-Path $root "mjai_bot\mortal\mortal.pth"),
        (Join-Path $root "mjai_bot\mortal3p\mortal.pth")
    )
    $missing = @()
    foreach ($path in $weights) {
        if ((-not (Test-Path $path)) -or (Test-LfsPointer $path)) {
            $missing += $path
        }
    }
    if ($missing.Count -eq 0) {
        return
    }

    $git = Find-CommandPath @("git") "Install Git first."
    Write-Host "Model weights are missing or still Git LFS pointers. Running git lfs pull ..."
    & $git lfs pull
    if ($LASTEXITCODE -ne 0) {
        throw "git lfs pull failed. Install Git LFS, run 'git lfs install', then run this script again."
    }

    foreach ($path in $missing) {
        if ((-not (Test-Path $path)) -or (Test-LfsPointer $path)) {
            throw "Model weight is still unavailable: $path"
        }
    }
}

function Ensure-FrontendDeps {
    if ($SkipNpmInstall) {
        return
    }
    $nodeModules = Join-Path $frontend "node_modules"
    if (Test-Path $nodeModules) {
        return
    }
    $npm = Find-Npm
    Write-Host "Installing frontend dependencies with npm ci ..."
    & $npm ci --prefix $frontend
    if ($LASTEXITCODE -ne 0) {
        throw "npm ci failed."
    }
}

function Start-Frontend {
    if ($SkipFrontend) {
        return
    }
    $frontendPort = 1420
    if (Test-PortListening $frontendPort) {
        Write-Host "Frontend dev server already listening on http://localhost:$frontendPort."
        return
    }

    Ensure-FrontendDeps
    $npm = Find-Npm
    Write-Host "Starting frontend dev server on http://localhost:$frontendPort ..."
    Start-Process -FilePath $npm `
        -ArgumentList @("run", "dev", "--", "--host", "127.0.0.1", "--port", "$frontendPort") `
        -WorkingDirectory $frontend `
        -WindowStyle Hidden

    $ready = $false
    for ($i = 0; $i -lt 60; $i++) {
        Start-Sleep -Milliseconds 500
        if (Test-PortListening $frontendPort) {
            $ready = $true
            break
        }
    }
    if (-not $ready) {
        throw "Frontend dev server did not start on port $frontendPort."
    }
}

Set-Location $root
Ensure-GitLfsFiles
Start-Frontend

$cargoArgs = @("run")
if ($Release) {
    $cargoArgs += "--release"
}
if ($Config) {
    $cargoArgs += @("--", "--config", $Config)
}

Write-Host "Starting Akagi backend: cargo $($cargoArgs -join ' ')"
& cargo @cargoArgs
