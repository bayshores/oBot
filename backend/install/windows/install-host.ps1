<#
  oBot: turn this Windows PC into an always-on "computer" for your bots.

  What it does:
    1. Copies obot-server.exe to %LOCALAPPDATA%\oBot\
    2. Checks that Claude Code is installed and logged in
    3. Keeps oBot private: it only listens on this PC (127.0.0.1:7777). No firewall port is
       opened. If Tailscale is installed and signed in, it turns on Tailscale Serve so your own
       devices (and nobody else) can reach it at a private https://<pc>.<tailnet>.ts.net address.
    4. Creates a scheduled task that starts obot-server when Windows boots
       (even before anyone signs in) and restarts it if it crashes
    5. Optional (-KeepAwake): stops the PC from sleeping while plugged in
    Then it prints the address and a 6-digit pairing code for the oBot app.

  How to run (in PowerShell opened with "Run as administrator"):
    Set-ExecutionPolicy -Scope Process Bypass
    .\install-host.ps1 -ServerExe C:\path\to\obot-server.exe -KeepAwake

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
$TaskName = "obot-server"
$InstallDir = Join-Path $env:LOCALAPPDATA "oBot"
$Target = Join-Path $InstallDir "obot-server.exe"
$LogFile = Join-Path $InstallDir "obot-server.log"
# Before the rename to oBot, the task, program and folder were called openbot.
$OldTaskName = "openbot-server"
$OldInstallDir = Join-Path $env:LOCALAPPDATA "openbot"

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

# Stop and remove an install from before the rename to oBot. The new server moves the old data
# folder (%APPDATA%\openbot) and Bot folders (%USERPROFILE%\OpenBot) by itself when it starts.
function Remove-OldName {
  Stop-ScheduledTask -TaskName $OldTaskName -ErrorAction SilentlyContinue
  Unregister-ScheduledTask -TaskName $OldTaskName -Confirm:$false -ErrorAction SilentlyContinue
  Remove-NetFirewallRule -DisplayName "openbot ($Port)" -ErrorAction SilentlyContinue
  for ($i = 0; $i -lt 20; $i++) {
    $old = Get-Process openbot-server -ErrorAction SilentlyContinue
    if (-not $old) { break }
    $old | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 500
  }
  # Bot runs the old server started (Claude Code with a settings file in the old data folder) would
  # keep the old folders in use, so they couldn't be moved.
  Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
    Where-Object { $_.CommandLine -like "*\AppData\Roaming\openbot\*" } |
    ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
  if (Test-Path $OldInstallDir) { Remove-Item -Recurse -Force $OldInstallDir -ErrorAction SilentlyContinue }
}

Assert-Admin
Remove-OldName

if ($Uninstall) {
  Stop-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
  Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
  Get-Process obot-server -ErrorAction SilentlyContinue | Stop-Process -Force
  $ts = Find-Tailscale
  if ($ts) {
    $served = (& $ts serve status 2>$null | Out-String)
    if ($served -match "127\.0\.0\.1:$Port|localhost:$Port") {
      & $ts serve --https=443 off 2>$null | Out-Null
      Write-Host "Turned off Tailscale Serve for oBot." -ForegroundColor Green
    }
  }
  Write-Host "obot-server was removed. Your bots' data is still in $env:APPDATA\oBot" -ForegroundColor Green
  exit 0
}

# 1. Find and copy the server program
if (-not $ServerExe) {
  $candidates = @(
    (Join-Path $PSScriptRoot "obot-server.exe"),
    (Join-Path $PSScriptRoot "..\..\..\target\release\obot-server.exe")
  )
  $ServerExe = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $ServerExe -or -not (Test-Path $ServerExe)) {
  Write-Host "Couldn't find obot-server.exe. Build it first (cargo build --release -p obot-server)" -ForegroundColor Red
  Write-Host "or pass its location: .\install-host.ps1 -ServerExe C:\path\to\obot-server.exe"
  exit 1
}
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
# Updating: stop the running copy first (the startup task, the server and its helper processes),
# then wait until Windows lets go of the file.
Stop-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
for ($i = 0; $i -lt 20; $i++) {
  $running = Get-Process obot-server -ErrorAction SilentlyContinue
  if (-not $running) { break }
  $running | Stop-Process -Force -ErrorAction SilentlyContinue
  Start-Sleep -Milliseconds 500
}
$copied = $false
for ($i = 0; $i -lt 10 -and -not $copied; $i++) {
  try { Copy-Item $ServerExe $Target -Force -ErrorAction Stop; $copied = $true }
  catch { Start-Sleep -Seconds 1 }
}
if (-not $copied) {
  Write-Host "Couldn't replace $Target (it's still in use). Restart the PC and run this script again." -ForegroundColor Red
  exit 1
}
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
    Write-Host "      Run 'claude' once in a normal PowerShell and log in (or add an API key later in oBot)."
  }
}

# 3. Private access. oBot listens only on 127.0.0.1, so nothing on your Wi-Fi or the internet
#    can reach it. Your own devices get in through Tailscale Serve (encrypted, tailnet only).
#    Older versions (still called openbot) opened a firewall port: remove that rule.
Remove-NetFirewallRule -DisplayName "openbot ($Port)" -ErrorAction SilentlyContinue
$ServeUrl = $null
$ts = Find-Tailscale
if (-not $ts) {
  Write-Host "[3/5] Tailscale isn't installed. oBot only works on this PC for now." -ForegroundColor Yellow
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
      Write-Host "      Not changing it. To share oBot on another port run:  tailscale serve --bg --https=8443 $Port"
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
        Write-Host "[3/5] Your devices can reach oBot privately at $ServeUrl (Tailscale Serve, HTTPS)." -ForegroundColor Green
        Write-Host "      Nothing is opened to your Wi-Fi or the internet. Never use 'tailscale funnel' for oBot."
      }
    }
  }
}

# 4. Start at boot, as you (so it can use your Claude login), restart on crash
# The real account name (env vars like USERDOMAIN can be wrong in SSH sessions).
$user = [Security.Principal.WindowsIdentity]::GetCurrent().Name
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
  -Settings $settings -Description "oBot always-on computer" | Out-Null
Start-ScheduledTask -TaskName $TaskName
Write-Host "[4/5] obot-server starts automatically when this PC boots (log: $LogFile)." -ForegroundColor Green

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
  Write-Host "All set! In the oBot app go to Settings > Computer > Remote computer and enter:" -ForegroundColor Cyan
  Write-Host "   Address:       $ServeUrl"
  Write-Host "   Pairing code:  $code   (works once, for 10 minutes)"
} else {
  Write-Host "oBot is running on this PC: http://127.0.0.1:$Port" -ForegroundColor Cyan
  Write-Host "Set up Tailscale (step 3 above) to use it from your other devices."
}
Write-Host ""
Write-Host "Need another code later (for a new device)?  & '$Target' pair"
Write-Host "See or remove paired devices:                  & '$Target' devices   /   & '$Target' unpair <id>"
