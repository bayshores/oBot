<#
  openbot: turn this Windows PC into an always-on "computer" for your bots.

  What it does:
    1. Copies openbot-server.exe to %LOCALAPPDATA%\openbot\
    2. Checks that Claude Code is installed and logged in
    3. Keeps openbot private: it only listens on this PC (127.0.0.1:7777). No firewall port is
       opened. If Tailscale is installed and signed in, it turns on Tailscale Serve so your own
       devices (and nobody else) can reach it at a private https://<pc>.<tailnet>.ts.net address.
    4. Creates a scheduled task that starts openbot-server when Windows boots
       (even before anyone signs in) and restarts it if it crashes
    5. Optional (-KeepAwake): stops the PC from sleeping while plugged in
    Then it prints the address and a 6-digit pairing code for the openbot app.

  How to run (in PowerShell opened with "Run as administrator"):
    Set-ExecutionPolicy -Scope Process Bypass
    .\install-host.ps1 -ServerExe C:\path\to\openbot-server.exe -KeepAwake

  To remove it later:
    .\install-host.ps1 -Uninstall
#>
param(
  [string]$ServerExe = "",
  [int]$Port = 7777,
  [switch]$KeepAwake,
  [switch]$Uninstall
)

$ErrorActionPreference = "Stop"
$TaskName = "openbot-server"
$InstallDir = Join-Path $env:LOCALAPPDATA "openbot"
$Target = Join-Path $InstallDir "openbot-server.exe"
$LogFile = Join-Path $InstallDir "openbot-server.log"

function Assert-Admin {
  $id = [Security.Principal.WindowsIdentity]::GetCurrent()
  $p = New-Object Security.Principal.WindowsPrincipal($id)
  if (-not $p.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Write-Host "Please run PowerShell as Administrator (right-click > Run as administrator)." -ForegroundColor Red
    exit 1
  }
}

function Find-Tailscale {
  $c = Get-Command tailscale -ErrorAction SilentlyContinue
  if ($c) { return $c.Source }
  $p = Join-Path $env:ProgramFiles "Tailscale\tailscale.exe"
  if (Test-Path $p) { return $p }
  return $null
}

Assert-Admin

if ($Uninstall) {
  Stop-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
  Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
  Remove-NetFirewallRule -DisplayName "openbot ($Port)" -ErrorAction SilentlyContinue
  Get-Process openbot-server -ErrorAction SilentlyContinue | Stop-Process -Force
  $ts = Find-Tailscale
  if ($ts) {
    $served = (& $ts serve status 2>$null | Out-String)
    if ($served -match "127\.0\.0\.1:$Port|localhost:$Port") {
      & $ts serve --https=443 off 2>$null | Out-Null
      Write-Host "Turned off Tailscale Serve for openbot." -ForegroundColor Green
    }
  }
  Write-Host "openbot-server was removed. Your bots' data is still in $env:APPDATA\openbot" -ForegroundColor Green
  exit 0
}

# 1. Find and copy the server program
if (-not $ServerExe) {
  $candidates = @(
    (Join-Path $PSScriptRoot "openbot-server.exe"),
    (Join-Path $PSScriptRoot "..\..\target\release\openbot-server.exe")
  )
  $ServerExe = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $ServerExe -or -not (Test-Path $ServerExe)) {
  Write-Host "Couldn't find openbot-server.exe. Build it first (cargo build --release -p openbot-server)" -ForegroundColor Red
  Write-Host "or pass its location: .\install-host.ps1 -ServerExe C:\path\to\openbot-server.exe"
  exit 1
}
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
Get-Process openbot-server -ErrorAction SilentlyContinue | Stop-Process -Force
Copy-Item $ServerExe $Target -Force
Write-Host "[1/5] Installed $Target" -ForegroundColor Green

# 2. Claude Code
$claude = Get-Command claude -ErrorAction SilentlyContinue
if (-not $claude) {
  Write-Host "[2/5] Claude Code isn't installed yet." -ForegroundColor Yellow
  Write-Host "      Install it:  irm https://claude.ai/install.ps1 | iex"
  Write-Host "      Then run 'claude' once in a normal (non-admin) PowerShell and log in with your Claude account."
} else {
  $status = & claude auth status 2>$null | Out-String
  if ($status -match '"loggedIn":\s*true') {
    Write-Host "[2/5] Claude Code is installed and logged in." -ForegroundColor Green
  } else {
    Write-Host "[2/5] Claude Code is installed but NOT logged in." -ForegroundColor Yellow
    Write-Host "      Run 'claude' once in a normal PowerShell and log in (or add an API key later in openbot)."
  }
}

# 3. Private access. openbot listens only on 127.0.0.1, so nothing on your Wi-Fi or the internet
#    can reach it. Your own devices get in through Tailscale Serve (encrypted, tailnet only).
#    Older openbot versions opened a firewall port: remove that rule.
Remove-NetFirewallRule -DisplayName "openbot ($Port)" -ErrorAction SilentlyContinue
$ServeUrl = $null
$ts = Find-Tailscale
if (-not $ts) {
  Write-Host "[3/5] Tailscale isn't installed. openbot only works on this PC for now." -ForegroundColor Yellow
  Write-Host "      To use it from your Mac or phone: install Tailscale (https://tailscale.com/download) on"
  Write-Host "      this PC and your other devices, sign in to the same account, then run this script again."
} else {
  $st = $null
  try { $st = (& $ts status --json 2>$null | Out-String) | ConvertFrom-Json } catch {}
  if (-not $st -or $st.BackendState -ne "Running") {
    Write-Host "[3/5] Tailscale is installed but not signed in. Open Tailscale, sign in, then run this script again." -ForegroundColor Yellow
  } else {
    $existing = (& $ts serve status 2>$null | Out-String)
    if ($existing -match "https://" -and $existing -notmatch "127\.0\.0\.1:$Port|localhost:$Port") {
      Write-Host "[3/5] Tailscale Serve is already used for something else on this PC:" -ForegroundColor Yellow
      Write-Host $existing
      Write-Host "      Not changing it. To share openbot on another port run:  tailscale serve --bg --https=8443 $Port"
    } else {
      $serveOk = $false
      try {
        $ErrorActionPreference = "Continue"   # tailscale prints notes on stderr; don't treat them as errors
        & $ts serve --bg --yes $Port 2>&1 | ForEach-Object { "$_" } | Out-Host
        $serveOk = ($LASTEXITCODE -eq 0)
      } catch {
        $serveOk = $false
      } finally {
        $ErrorActionPreference = "Stop"
      }
      if (-not $serveOk) {
        Write-Host "[3/5] Tailscale Serve didn't start. It may need HTTPS turned on for your tailnet:" -ForegroundColor Yellow
        Write-Host "      open https://login.tailscale.com/admin/dns , enable MagicDNS and HTTPS Certificates, then run this script again."
      } else {
        $ServeUrl = "https://" + ($st.Self.DNSName.TrimEnd('.'))
        Write-Host "[3/5] Your devices can reach openbot privately at $ServeUrl (Tailscale Serve, HTTPS)." -ForegroundColor Green
        Write-Host "      Nothing is opened to your Wi-Fi or the internet. Never use 'tailscale funnel' for openbot."
      }
    }
  }
}

# 4. Start at boot, as you (so it can use your Claude login), restart on crash
$user = "$env:USERDOMAIN\$env:USERNAME"
$cmd = "& '$Target' --port $Port *>> '$LogFile'"
$action = New-ScheduledTaskAction -Execute "powershell.exe" `
  -Argument "-NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -Command `"$cmd`""
$trigger = New-ScheduledTaskTrigger -AtStartup
$principal = New-ScheduledTaskPrincipal -UserId $user -LogonType S4U -RunLevel Limited
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries `
  -StartWhenAvailable -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1) `
  -ExecutionTimeLimit ([TimeSpan]::Zero)
Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger -Principal $principal `
  -Settings $settings -Description "openbot always-on computer" | Out-Null
Start-ScheduledTask -TaskName $TaskName
Write-Host "[4/5] openbot-server starts automatically when this PC boots (log: $LogFile)." -ForegroundColor Green

# 5. Keep awake
if ($KeepAwake) {
  powercfg /change standby-timeout-ac 0 | Out-Null
  powercfg /change hibernate-timeout-ac 0 | Out-Null
  Write-Host "[5/5] This PC won't go to sleep while plugged in." -ForegroundColor Green
} else {
  Write-Host "[5/5] Tip: re-run with -KeepAwake so the PC doesn't fall asleep (bots can't work while it sleeps)." -ForegroundColor Yellow
}

Start-Sleep -Seconds 3
$ErrorActionPreference = "Continue"
$pair = (& $Target pair 2>&1 | ForEach-Object { "$_" } | Out-String)
$code = if ($pair -match 'Pairing code:\s+(\d{3}) (\d{3})') { "$($Matches[1]) $($Matches[2])" } else { "(run: $Target pair)" }
Write-Host ""
if ($ServeUrl) {
  Write-Host "All set! In the openbot app go to Settings > Computer > Remote computer and enter:" -ForegroundColor Cyan
  Write-Host "   Address:       $ServeUrl"
  Write-Host "   Pairing code:  $code   (works once, for 10 minutes)"
} else {
  Write-Host "openbot is running on this PC: http://127.0.0.1:$Port" -ForegroundColor Cyan
  Write-Host "Set up Tailscale (step 3 above) to use it from your other devices."
}
Write-Host ""
Write-Host "Need another code later (for a new device)?  & '$Target' pair"
Write-Host "See or remove paired devices:                  & '$Target' devices   /   & '$Target' unpair <id>"
