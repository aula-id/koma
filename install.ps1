# koma installer — https://koma.run
# Usage:
#   irm https://koma.run/install.ps1 | iex
#
# Environment overrides:
#   KOMA_RELEASE_BASE   override the base download URL

$ErrorActionPreference = 'Stop'

# ---------------------------------------------------------------------------
# Banner
# ---------------------------------------------------------------------------
Write-Host ""
Write-Host "koma installer"
Write-Host ""

# ---------------------------------------------------------------------------
# TLS 1.2 — required for GitHub releases downloads
# ---------------------------------------------------------------------------
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

# ---------------------------------------------------------------------------
# Architecture check — only x64 is supported
# ---------------------------------------------------------------------------
$arch = $env:PROCESSOR_ARCHITECTURE
if ($arch -ne 'AMD64') {
    Write-Error "unsupported architecture: $arch`nkoma on Windows currently supports x64 only."
    exit 1
}

# ---------------------------------------------------------------------------
# Build asset URL
# ---------------------------------------------------------------------------
$base = if ($env:KOMA_RELEASE_BASE) { $env:KOMA_RELEASE_BASE } else { 'https://github.com/aula-id/koma/releases/latest/download' }
$asset = 'koma-x64.msi'
$url = "$base/$asset"
$msiPath = Join-Path $env:TEMP 'koma-installer.msi'

Write-Host "  arch:    $arch"
Write-Host "  url:     $url"
Write-Host "  target:  $msiPath"
Write-Host ""

# ---------------------------------------------------------------------------
# Download + install + cleanup
# ---------------------------------------------------------------------------
try {
    Write-Host "Downloading koma..."
    Invoke-WebRequest -Uri $url -OutFile $msiPath -UseBasicParsing

    $installed = $null
    try {
        $installed = Get-ItemProperty `
            'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*', `
            'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*' `
            -ErrorAction SilentlyContinue |
            Where-Object { $_.DisplayName -eq 'koma' }
    } catch {
        $installed = $null
    }
    if ($installed) {
        Write-Host "Upgrading koma..."
    } else {
        Write-Host "Installing koma..."
    }
    # 3010: success, reboot requested (a file was in use and Windows will finish the replace).
    $proc = Start-Process msiexec.exe -ArgumentList @('/i', $msiPath) -Wait -NoNewWindow -PassThru
    if ($proc.ExitCode -ne 0 -and $proc.ExitCode -ne 3010) {
        Write-Error "MSI installer exited with code $($proc.ExitCode)"
        exit 1
    }

    Write-Host ""
    if ($installed) {
        Write-Host "koma upgraded successfully."
    } else {
        Write-Host "koma installed successfully."
    }
    Write-Host ""
    Write-Host "  Run 'koma' to start."
    Write-Host ""
    Write-Host "  https://koma.run"
} finally {
    if (Test-Path $msiPath) {
        Remove-Item $msiPath -Force -ErrorAction SilentlyContinue
    }
}
