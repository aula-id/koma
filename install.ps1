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

    # The release publishes SHA256SUMS next to the installer. Check the MSI
    # against it before msiexec runs it.
    if ($env:KOMA_SKIP_CHECKSUM -eq '1') {
        Write-Host "Checksum check skipped (KOMA_SKIP_CHECKSUM=1)."
    } else {
        $sums = $null
        try {
            $response = Invoke-WebRequest -Uri "$base/SHA256SUMS" -UseBasicParsing
            $sums = if ($response.Content -is [byte[]]) {
                [System.Text.Encoding]::UTF8.GetString($response.Content)
            } else {
                [string]$response.Content
            }
        } catch {
            $sums = $null
        }
        if (-not $sums) {
            # Releases before SHA256SUMS existed have nothing to check against.
            Write-Warning "This release publishes no SHA256SUMS; installing unverified."
        } else {
            $want = $null
            foreach ($line in ($sums -split "`n")) {
                $parts = $line.Trim() -split '\s+', 2
                if ($parts.Count -eq 2 -and $parts[1].TrimStart('*') -eq $asset) {
                    $want = $parts[0].ToLowerInvariant()
                    break
                }
            }
            if (-not $want) {
                Write-Error "SHA256SUMS does not list $asset; refusing to install it."
                exit 1
            }
            $got = (Get-FileHash -Path $msiPath -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($got -ne $want) {
                Write-Error "Checksum mismatch for $asset; nothing was installed. Expected $want, got $got."
                exit 1
            }
            Write-Host "Verified sha256 $got"
        }
    }

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
    # /passive: progress bar + UAC, no Welcome / InstallDir / PATH wizard.
    # PATH is written by the WiX Environment component; the wizard never needed
    # to opt into it.
    $proc = Start-Process msiexec.exe -ArgumentList @('/i', $msiPath, '/passive', '/norestart') -Wait -NoNewWindow -PassThru
    if ($proc.ExitCode -ne 0 -and $proc.ExitCode -ne 3010) {
        Write-Error "MSI installer exited with code $($proc.ExitCode)"
        exit 1
    }

    # MSI updates HKCU/HKLM PATH; this PowerShell still has the old value.
    $machinePath = [Environment]::GetEnvironmentVariable('Path', 'Machine')
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $parts = @()
    if ($userPath) { $parts += $userPath }
    if ($machinePath) { $parts += $machinePath }
    if ($parts.Count -gt 0) {
        $env:Path = $parts -join ';'
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
