<#
  openbot: turn this Windows PC into an always-on "computer" for your bots.

  What it does:
    1. Copies openbot-server.exe to %LOCALAPPDATA%\openbot\
    2. Checks that Claude Code is installed and logged in
    3. Opens TCP port 7777 in Windows Firewall (Private networks only)
    4. Creates a scheduled task that starts openbot-server when Windows boots
       (even before anyone signs in) and restarts it if it crashes
    5. Optional (-KeepAwake): stops the PC from sleeping while plugged in

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

Assert-Admin

if ($Uninstall) {
  Stop-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
  Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
  Remove-NetFirewallRule -DisplayName "openbot ($Port)" -ErrorAction SilentlyContinue
  Get-Process openbot-server -ErrorAction SilentlyContinue | Stop-Process -Force
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

# 3. Firewall: allow other computers on your private network / Tailscale to connect
Remove-NetFirewallRule -DisplayName "openbot ($Port)" -ErrorAction SilentlyContinue
New-NetFirewallRule -DisplayName "openbot ($Port)" -Direction Inbound -Protocol TCP -LocalPort $Port `
  -Action Allow -Profile Private | Out-Null
Write-Host "[3/5] Firewall allows port $Port on Private networks." -ForegroundColor Green

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
$token = & $Target token
$ip = (Get-NetIPAddress -AddressFamily IPv4 | Where-Object { $_.InterfaceAlias -notmatch 'Loopback' -and $_.IPAddress -notmatch '^169\.' } | Select-Object -First 1).IPAddress
Write-Host ""
Write-Host "All set! In the openbot app go to Settings > Computer > Remote computer and enter:" -ForegroundColor Cyan
Write-Host "   Address: http://${ip}:$Port   (or your Tailscale address, e.g. http://my-pc:$Port)"
Write-Host "   Token:   $token"
