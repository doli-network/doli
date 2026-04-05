# DOLI Installer for Windows
# Usage: irm https://doli.network/install.ps1 | iex
# Or:    powershell -ExecutionPolicy Bypass -File install.ps1

$ErrorActionPreference = "Stop"

$Repo = "e-weil/doli"
$GitHub = "https://github.com/$Repo"
$Api = "https://api.github.com/repos/$Repo/releases/latest"
$Target = "x86_64-pc-windows-msvc"
$InstallDir = "$env:ProgramFiles\DOLI"

function Info($msg) { Write-Host "==> $msg" -ForegroundColor Cyan }
function Ok($msg) { Write-Host "==> $msg" -ForegroundColor Green }
function Err($msg) { Write-Host "error: $msg" -ForegroundColor Red; exit 1 }

# Check admin
$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdmin) {
    Err "Administrator privileges required. Run PowerShell as Administrator."
}

Info "Platform: Windows x86_64"
Info "Fetching latest release..."

try {
    $release = Invoke-RestMethod -Uri $Api -Headers @{ "User-Agent" = "doli-installer" }
} catch {
    Err "Failed to fetch release info. Check $GitHub/releases"
}

$Version = $release.tag_name
if (-not $Version) { Err "Could not determine latest version" }

Info "Latest version: $Version"

$File = "doli-${Version}-${Target}.zip"
$Url = "$GitHub/releases/download/$Version/$File"

$TmpDir = Join-Path $env:TEMP "doli-install-$(Get-Random)"
New-Item -ItemType Directory -Path $TmpDir -Force | Out-Null

try {
    # Download
    Info "Downloading $File..."
    $ZipPath = Join-Path $TmpDir $File
    try {
        Invoke-WebRequest -Uri $Url -OutFile $ZipPath -UseBasicParsing
    } catch {
        Err "Download failed. Check $GitHub/releases/tag/$Version"
    }

    # Extract
    Info "Extracting..."
    Expand-Archive -Path $ZipPath -DestinationPath $TmpDir -Force
    $ExtractDir = Get-ChildItem -Path $TmpDir -Directory -Filter "doli-*" | Select-Object -First 1
    if (-not $ExtractDir) { Err "Failed to extract archive" }

    # Install
    Info "Installing to $InstallDir..."
    if (-not (Test-Path $InstallDir)) {
        New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    }
    Copy-Item (Join-Path $ExtractDir.FullName "doli-node.exe") $InstallDir -Force
    Copy-Item (Join-Path $ExtractDir.FullName "doli.exe") $InstallDir -Force

    # Add to PATH if not already there
    $machinePath = [Environment]::GetEnvironmentVariable("Path", "Machine")
    if ($machinePath -notlike "*$InstallDir*") {
        Info "Adding $InstallDir to system PATH..."
        [Environment]::SetEnvironmentVariable("Path", "$machinePath;$InstallDir", "Machine")
        $env:Path = "$env:Path;$InstallDir"
    }

    # Create data directories
    $DataDir = Join-Path $env:ProgramData "DOLI"
    $MainnetDir = Join-Path $DataDir "mainnet"
    $LogDir = Join-Path $DataDir "logs"
    foreach ($dir in @($DataDir, $MainnetDir, $LogDir)) {
        if (-not (Test-Path $dir)) {
            New-Item -ItemType Directory -Path $dir -Force | Out-Null
        }
    }

    # Verify
    $NodeVersion = & "$InstallDir\doli-node.exe" --version 2>&1
    $CliVersion = & "$InstallDir\doli.exe" --version 2>&1

    Write-Host ""
    Ok "DOLI $Version installed"
    Write-Host ""
    Write-Host "  doli-node: $InstallDir\doli-node.exe ($NodeVersion)"
    Write-Host "  doli CLI:  $InstallDir\doli.exe ($CliVersion)"
    Write-Host "  Data:      $DataDir"
    Write-Host ""
    Write-Host "  Run a node:"
    Write-Host "    doli-node --network mainnet run --yes"
    Write-Host ""
    Write-Host "  Check balance:"
    Write-Host "    doli balance"
    Write-Host ""
    if ($machinePath -notlike "*$InstallDir*") {
        Write-Host "  NOTE: Restart your terminal for PATH changes to take effect." -ForegroundColor Yellow
        Write-Host ""
    }

} finally {
    # Cleanup
    Remove-Item -Path $TmpDir -Recurse -Force -ErrorAction SilentlyContinue
}
