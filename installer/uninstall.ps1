# Removes Glance: the app folder, Start menu shortcut, Installed apps entry and its photo/video
# registration. Your Glance settings, ratings and thumbnail cache (in AppData) are kept.
# Windows runs this from Settings > Apps > Installed apps > Glance > Uninstall.
$ErrorActionPreference = 'Stop'

$me = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
if (-not $me.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Start-Process powershell.exe -Verb RunAs -ArgumentList "-NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`""
    return
}

try {
    $dir = Join-Path $env:ProgramFiles 'Glance'
    $exe = Join-Path $dir 'glance.exe'

    Get-Process glance -ErrorAction SilentlyContinue | ForEach-Object { $_.CloseMainWindow() | Out-Null; $_.WaitForExit(5000) | Out-Null }
    Get-Process glance -ErrorAction SilentlyContinue | Stop-Process -Force

    if (Test-Path $exe) { Start-Process $exe -ArgumentList '--unregister' -Wait }
    Remove-Item (Join-Path $env:ProgramData 'Microsoft\Windows\Start Menu\Programs\Glance.lnk') -ErrorAction SilentlyContinue
    Remove-Item 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Glance' -Recurse -ErrorAction SilentlyContinue
    # Only Glance's own folder in Program Files.
    if (Test-Path $dir) { Remove-Item $dir -Recurse -Force }

    Write-Host ''
    Write-Host 'Glance is uninstalled. Your settings are kept in case you install it again.' -ForegroundColor Green
} catch {
    Write-Host ''
    Write-Host "Uninstall failed: $_" -ForegroundColor Red
}
Write-Host ''
Read-Host 'Press Enter to close'
