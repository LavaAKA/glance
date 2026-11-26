# Installs Glance into Program Files, like a normal Windows app:
#   - C:\Program Files\Glance\glance.exe
#   - a Start menu shortcut
#   - an entry in Settings > Apps > Installed apps (with Uninstall)
#   - Glance listed as a photo and video app, so you can make it the default
# Run it by double-clicking "Install Glance.cmd" next to this file.
$ErrorActionPreference = 'Stop'

# Program Files needs administrator rights: ask Windows once, then carry on as administrator.
$me = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
if (-not $me.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Start-Process powershell.exe -Verb RunAs -ArgumentList "-NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`""
    return
}

try {
    $src = Join-Path $PSScriptRoot '..\target\release\glance.exe'
    if (-not (Test-Path $src)) { throw "glance.exe isn't built yet. Run 'cargo build --release' in the photo-viewer folder first." }
    $dir = Join-Path $env:ProgramFiles 'Glance'
    $exe = Join-Path $dir 'glance.exe'

    # Close Glance politely (so it saves its settings) before replacing it.
    Get-Process glance -ErrorAction SilentlyContinue | ForEach-Object { $_.CloseMainWindow() | Out-Null; $_.WaitForExit(5000) | Out-Null }
    Get-Process glance -ErrorAction SilentlyContinue | Stop-Process -Force

    New-Item -ItemType Directory -Force $dir | Out-Null
    Copy-Item $src $exe -Force
    Copy-Item (Join-Path $PSScriptRoot 'uninstall.ps1') $dir -Force

    # Start menu shortcut for everyone on this PC.
    $lnk = Join-Path $env:ProgramData 'Microsoft\Windows\Start Menu\Programs\Glance.lnk'
    $s = (New-Object -ComObject WScript.Shell).CreateShortcut($lnk)
    $s.TargetPath = $exe
    $s.WorkingDirectory = $dir
    $s.Description = 'Photo and Video Viewer'
    $s.Save()

    # Settings > Apps > Installed apps entry.
    $key = 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Glance'
    New-Item $key -Force | Out-Null
    $props = @{
        DisplayName     = 'Glance'
        DisplayIcon     = "`"$exe`",0"
        DisplayVersion  = (Get-Item $exe).VersionInfo.ProductVersion
        Publisher       = 'Glance'
        InstallLocation = $dir
        UninstallString = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$dir\uninstall.ps1`""
    }
    foreach ($p in $props.GetEnumerator()) { Set-ItemProperty $key -Name $p.Key -Value $p.Value }
    foreach ($p in @{ NoModify = 1; NoRepair = 1; EstimatedSize = [int]((Get-Item $exe).Length / 1KB) }.GetEnumerator()) {
        New-ItemProperty $key -Name $p.Key -Value $p.Value -PropertyType DWord -Force | Out-Null
    }

    # List Glance as a photo and video app (for the person installing).
    Start-Process $exe -ArgumentList '--register' -Wait

    Write-Host ''
    Write-Host 'Glance is installed.' -ForegroundColor Green
    Write-Host "  App:        $exe"
    Write-Host '  Start menu: Glance'
    Write-Host '  Uninstall:  Settings > Apps > Installed apps > Glance'
    Write-Host ''
    Write-Host 'Opening Default apps so you can choose Glance for photos and videos...'
    Start-Process 'ms-settings:defaultapps?registeredAppUser=Glance'
} catch {
    Write-Host ''
    Write-Host "Install failed: $_" -ForegroundColor Red
}
Write-Host ''
Read-Host 'Press Enter to close'
