$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$frontend = Join-Path $root "frontend"

function Find-Npm {
    $preferred = "D:\Program Files\node.js\npm.cmd"
    if (Test-Path $preferred) {
        return $preferred
    }
    $cmd = Get-Command npm.cmd -ErrorAction SilentlyContinue
    if ($cmd) {
        return $cmd.Source
    }
    $cmd = Get-Command npm -ErrorAction SilentlyContinue
    if ($cmd) {
        return $cmd.Source
    }
    throw "npm not found. Install Node.js first, or add npm to PATH."
}

function Test-PortListening([int] $port) {
    $conn = Get-NetTCPConnection -State Listen -LocalPort $port -ErrorAction SilentlyContinue
    return $null -ne $conn
}

if (-not (Test-PortListening 1420)) {
    $npm = Find-Npm
    Write-Host "Starting frontend dev server on http://localhost:1420 ..."
    Start-Process -FilePath $npm `
        -ArgumentList @("run", "dev", "--", "--host", "127.0.0.1") `
        -WorkingDirectory $frontend `
        -WindowStyle Hidden

    $ready = $false
    for ($i = 0; $i -lt 30; $i++) {
        Start-Sleep -Milliseconds 500
        if (Test-PortListening 1420) {
            $ready = $true
            break
        }
    }
    if (-not $ready) {
        throw "Frontend dev server did not start on port 1420."
    }
} else {
    Write-Host "Frontend dev server already listening on http://localhost:1420."
}

Write-Host "Starting Akagi backend ..."
Set-Location $root
cargo run
