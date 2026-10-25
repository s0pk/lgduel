# One command to host an LGDuel server on Windows: .\host.cmd [server options, e.g. --port 27960 --map arena_tiers]
# Uses lgduel-server.exe from the release zip if it is next to this script, else builds it with Rust.
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

$port = 27960
for ($i = 0; $i -lt $args.Count; $i++) {
    if ($args[$i] -eq '--port' -and $i + 1 -lt $args.Count) { $port = $args[$i + 1] }
}

if (Test-Path .\lgduel-server.exe) {
    $exe = (Resolve-Path .\lgduel-server.exe).Path
} else {
    $env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        Write-Host "No lgduel-server.exe here and Rust is not installed."
        Write-Host "Easiest: download LGDuel-windows-x64.zip from the Releases page, unzip it and run host.cmd inside it."
        Write-Host "Or install Rust (https://rustup.rs, needs the Visual Studio Build Tools) and run this again."
        exit 1
    }
    Write-Host "Building the server (first time takes a minute or two)..."
    cargo build --release -p lgduel-server
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    $target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'target' }
    $exe = (Resolve-Path "$target\release\lgduel-server.exe").Path
}

$lan = $null
try {
    $lan = (Get-NetIPConfiguration | Where-Object { $_.IPv4DefaultGateway -ne $null } | Select-Object -First 1).IPv4Address.IPAddress
} catch {}
$pub = $null
try { $pub = (Invoke-RestMethod -Uri 'https://api.ipify.org' -TimeoutSec 4).ToString().Trim() } catch {}
if (-not $lan) { $lan = '(not found)' }
if (-not $pub) { $pub = '(not found, look up your public IP)' }

Write-Host ""
Write-Host "==================== LGDuel server ===================="
Write-Host " Players connect with the address below. In the game:"
Write-Host '   Play online -> "Add a server" -> paste the address -> Join'
Write-Host " or from a terminal:  lgduel.exe --connect ADDRESS"
Write-Host ""
Write-Host "   This PC:        127.0.0.1:$port"
Write-Host "   Same network:   ${lan}:$port"
Write-Host "   Internet:       ${pub}:$port"
Write-Host ""
Write-Host " Allow the Windows Firewall prompt (UDP). At home the server asks your router to open the"
Write-Host " port by itself (UPnP / NAT-PMP); if that works it prints a DIRECT CODE below: friends paste"
Write-Host " it under Play online -> Join code. If your router says no, forward UDP $port there yourself"
Write-Host " (nothing uses TCP). On a VPS also open it in the provider's firewall panel."
Write-Host " Type 'help' here for server commands (status, kick, say, set ...). Ctrl+C stops the server."
Write-Host "======================================================="
Write-Host ""
& $exe --upnp @args
exit $LASTEXITCODE
