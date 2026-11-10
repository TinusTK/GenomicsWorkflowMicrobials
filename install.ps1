# NTG pathology genomics pipeline - installer for Windows 10/11 (WSL2).
#   irm https://raw.githubusercontent.com/TinusTK/GenomicsWorkflowMicrobials/main/install.ps1 | iex
# Run it in normal PowerShell (not as administrator). It needs Ubuntu on WSL2:
#   wsl --install -d Ubuntu-24.04   (as administrator, then restart and open Ubuntu once)
# It installs the tools inside Ubuntu, puts the pipeline in Documents, and makes the
# desktop shortcuts. Running it again is safe. Another Ubuntu: set $env:NTG_DISTRO first.
$ErrorActionPreference = "Stop"
$Distro = if ($env:NTG_DISTRO) { $env:NTG_DISTRO } else { "Ubuntu-24.04" }
$Raw = "https://raw.githubusercontent.com/TinusTK/GenomicsWorkflowMicrobials/main"

if (-not (Get-Command wsl.exe -ErrorAction SilentlyContinue)) {
    Write-Host "WSL isn't installed. In PowerShell as administrator run:  wsl --install -d $Distro" -ForegroundColor Yellow
    Write-Host "then restart the computer, open '$Distro' from the Start menu once, and run this again."
    return
}
# wsl -l prints UTF-16; drop the NULs before comparing names
$distros = (& wsl.exe -l -q) -replace "`0", "" | Where-Object { $_.Trim() } | ForEach-Object { $_.Trim() }
if ($distros -notcontains $Distro) {
    Write-Host "Ubuntu '$Distro' isn't installed in WSL (found: $($distros -join ', '))." -ForegroundColor Yellow
    Write-Host "In PowerShell as administrator run:  wsl --install -d $Distro"
    Write-Host "then restart, open '$Distro' from the Start menu once to set a user name, and run this again."
    Write-Host "Using another Ubuntu? Set it first, e.g.  `$env:NTG_DISTRO = 'Ubuntu'"
    return
}

$win = Join-Path ([Environment]::GetFolderPath("MyDocuments")) "GenomicsWorkflowMicrobials"
$dest = (& wsl.exe -d $Distro wslpath -a ($win -replace '\\', '/')).Trim()
Write-Host "Installing into $win (inside Ubuntu: $dest). Ubuntu asks for your Ubuntu password." -ForegroundColor Cyan
& wsl.exe -d $Distro -- bash -c "curl -fsSL '$Raw/install.sh' | bash -s -- '$dest'"
if ($LASTEXITCODE -ne 0) { Write-Host "The install inside Ubuntu failed (see above)." -ForegroundColor Red; return }

& powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $win "windows\create-desktop-shortcut.ps1") -Distro $Distro
Write-Host ""
Write-Host "Done. Double-click 'Genomics Workflow Microbials' on the desktop, then go to Databases" -ForegroundColor Green
Write-Host "and install CheckM and Kraken2 Standard-8."
