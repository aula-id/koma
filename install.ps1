# koma installer for Windows - https://koma.run
#
# Usage (Windows PowerShell 5.1 or PowerShell 7, normal or administrator window):
#   irm https://koma.run/install.ps1 | iex
#
# It downloads koma-x64.msi, checks it against the release's SHA256SUMS and
# installs it silently. Windows asks for permission at most once (one UAC
# prompt, none if this window is already an administrator). No installer windows.
#
# Options (set before the command, e.g.  $env:KOMA_VERSION = 'v0.2.27'):
#   KOMA_VERSION         install this release tag instead of the latest (v0.2.27 or 0.2.27)
#   KOMA_RELEASE_BASE    download from this base URL instead of GitHub releases
#   KOMA_SKIP_CHECKSUM=1 install without checking SHA256SUMS (not recommended)
#   KOMA_UNINSTALL=1     remove koma instead of installing it
#   KOMA_FORCE_CLOSE=1   stop a running koma without asking (for scripts)
#   KOMA_WAIT_PID        wait up to 60 s for this process id to exit first (used by koma update)
#   KOMA_INSTALL_NO_RUN=1  only define the functions (for tests), install nothing
#
# Everything lives inside one script block, so your session keeps its own
# settings and variables. Windows PowerShell 5.1 syntax only.

$komaBody = {
    param([switch]$DefineOnly)

    # ---------------------------------------------------------------------
    # Pure functions (no I/O; unit-tested in tests/windows/install.Tests.ps1)
    # ---------------------------------------------------------------------

    function Resolve-KomaArch {
        param([string]$ProcArch, [string]$ProcArchW6432, [int]$OsBuild)
        # A 32-bit host on 64-bit Windows reports the real CPU in PROCESSOR_ARCHITEW6432.
        $native = $ProcArch
        if ($ProcArchW6432) { $native = $ProcArchW6432 }
        $ok = $false
        $emulated = $false
        $msg = ''
        if ($OsBuild -lt 17763) {
            $msg = "koma needs Windows 10 version 1809 or newer (this PC reports build $OsBuild). Update Windows and run the command again."
        } elseif ($native -eq 'AMD64') {
            $ok = $true
        } elseif ($native -eq 'ARM64') {
            if ($OsBuild -ge 22000) {
                $ok = $true
                $emulated = $true
            } else {
                $msg = 'koma runs on Windows on ARM only with Windows 11 (it runs as an x64 app there). This PC has ARM64 with an older Windows 10. Update to Windows 11 and run the command again.'
            }
        } else {
            $msg = "koma on Windows needs a 64-bit PC (x64, or ARM64 with Windows 11). This PC reports '$native'."
        }
        return [pscustomobject]@{ Ok = $ok; Native = $native; Emulated = $emulated; Message = $msg }
    }

    function Test-KomaVersionTag {
        param([string]$Version)
        if (-not $Version) { return $false }
        return [bool]($Version -match '^v?\d+\.\d+\.\d+([-+.][0-9A-Za-z.-]+)?$')
    }

    function Get-KomaBaseUrl {
        param([string]$Base, [string]$Version)
        if ($Base) { return $Base.Trim().TrimEnd('/') }
        if ($Version) {
            $tag = $Version.Trim()
            if (-not $tag.StartsWith('v')) { $tag = 'v' + $tag }
            return "https://github.com/aula-id/koma/releases/download/$tag"
        }
        return 'https://github.com/aula-id/koma/releases/latest/download'
    }

    function Get-KomaAssetUrl {
        param([string]$Base, [string]$Version)
        return (Get-KomaBaseUrl -Base $Base -Version $Version) + '/koma-x64.msi'
    }

    function Test-KomaBaseUrl {
        param([string]$Url)
        return [bool](($Url -match '^https://') -or ($Url -match '^http://(127\.0\.0\.1|localhost)(:\d+)?(/|$)'))
    }

    function ConvertFrom-KomaSums {
        # sha256sum format: "<64 hex>  <name>" or "<64 hex> *<name>". Returns name -> lowercase hex.
        param([string]$Text)
        $map = @{}
        if (-not $Text) { return $map }
        foreach ($line in ($Text -split "`n")) {
            $m = [regex]::Match($line.Trim(), '^([0-9a-fA-F]{64})\s+\*?(.+)$')
            if ($m.Success) {
                $name = $m.Groups[2].Value.Trim()
                if ($name.StartsWith('./')) { $name = $name.Substring(2) }
                $map[$name] = $m.Groups[1].Value.ToLowerInvariant()
            }
        }
        return $map
    }

    function Get-KomaMsiMessage {
        param([int]$Code, [string]$LogPath)
        $kind = 'Fail'
        $text = ''
        $admin = 'Administrator rights are required and were not granted. Run the command again and approve the prompt, or use an administrator account.'
        $busy = 'Another Windows installation is running (Windows Update or another setup). Wait for it to finish, or restart the PC, then run the command again.'
        $damaged = 'The downloaded installer is damaged or unreadable. Run the command again to fetch a fresh copy.'
        $policy = "This computer's policy blocks installing koma (AppLocker or Group Policy). Ask your IT admin to allow koma-x64.msi."
        $service = 'The Windows Installer service is not available. Restart the PC, or run "sfc /scannow" in an administrator window, then try again.'
        switch ($Code) {
            0     { $kind = 'Ok' }
            3010  { $kind = 'Reboot'; $text = 'koma is installed. Windows wants a restart to finish replacing some files; koma works now, restart when convenient.' }
            1641  { $kind = 'Reboot'; $text = 'The installer started a restart. If koma is missing after you sign in, run the command again.' }
            1602  { $text = 'The installation was cancelled before it finished. Nothing was changed. Run the command again and accept the Windows permission prompt.' }
            1618  { $text = $busy }
            1500  { $text = $busy }
            1603  { $text = 'Windows Installer hit a fatal error. Common causes: koma or a program using its folder was still open, antivirus blocked the install, or the disk is full. Close koma and other terminals, pause antivirus for a minute, then run the command again.' }
            1619  { $text = $damaged }
            1620  { $text = $damaged }
            1625  { $text = $policy }
            1260  { $text = $policy }
            1601  { $text = $service }
            1719  { $text = $service }
            1638  { $text = 'Another build of koma is blocking this one. Run the command once with $env:KOMA_UNINSTALL = ''1'' to remove the old one, then install again.' }
            1633  { $text = 'This installer is not for this version of Windows (koma needs 64-bit Windows).' }
            1925  { $text = $admin }
            740   { $text = $admin }
            5     { $text = $admin }
            1223  { $text = 'The Windows permission prompt was declined, so nothing was changed. Run the command again and choose Yes. (No administrator rights? Ask your IT admin.)' }
            90001 { $text = 'The installer file changed after it was verified, so it was not run. Run the command again. If it repeats, something on this PC is modifying files in your temp folder.' }
            90002 { $text = 'The install step failed before Windows Installer could run it.' }
            default { $text = "The installer failed with code $Code. Look the code up at https://learn.microsoft.com/windows/win32/msi/error-codes" }
        }
        if ($kind -eq 'Fail' -and $LogPath) { $text = $text + " Log: $LogPath" }
        return [pscustomobject]@{ Kind = $kind; Text = $text }
    }

    function Get-KomaPathKey {
        param([string]$Entry)
        $e = [Environment]::ExpandEnvironmentVariables($Entry.Trim().Trim('"'))
        return $e.TrimEnd('\', '/').ToLowerInvariant()
    }

    function Merge-KomaPath {
        # Returns Current unchanged when Dir is already listed (case-insensitive, trailing
        # slash tolerant, %VARS% expanded); otherwise Dir appended with empty segments dropped.
        param([string]$Current, [string]$Dir)
        $entries = @()
        if ($Current) {
            foreach ($p in ($Current -split ';')) {
                if ($p.Trim()) { $entries += $p.Trim() }
            }
        }
        $want = Get-KomaPathKey $Dir
        foreach ($e in $entries) {
            if ((Get-KomaPathKey $e) -eq $want) { return $Current }
        }
        $entries += $Dir.TrimEnd('\', '/')
        return ($entries -join ';')
    }

    function Remove-KomaPathEntry {
        param([string]$Current, [string]$Dir)
        $want = Get-KomaPathKey $Dir
        $keep = @()
        if ($Current) {
            foreach ($p in ($Current -split ';')) {
                if ($p.Trim() -and ((Get-KomaPathKey $p) -ne $want)) { $keep += $p.Trim() }
            }
        }
        return ($keep -join ';')
    }

    function Join-KomaSessionPath {
        # Machine first, then User: the order Windows itself uses.
        param([string]$Machine, [string]$User)
        $parts = @()
        if ($Machine) { $parts += $Machine.TrimEnd(';') }
        if ($User) { $parts += $User.TrimEnd(';') }
        return ($parts -join ';')
    }

    function Get-KomaMsiexecPath {
        param([string]$WinDir, [bool]$Is64Process, [bool]$Is64OS)
        # A 32-bit PowerShell on 64-bit Windows would get the 32-bit msiexec through System32.
        if ($Is64OS -and -not $Is64Process) { return "$WinDir\Sysnative\msiexec.exe" }
        return "$WinDir\System32\msiexec.exe"
    }

    function Format-KomaBytes {
        param([double]$Bytes)
        if ($Bytes -ge 1MB) { return ('{0:N1} MB' -f ($Bytes / 1MB)) }
        return ('{0:N0} KB' -f ($Bytes / 1KB))
    }

    function Format-KomaLiteral {
        param([string]$Text)
        return "'" + ($Text -replace "'", "''") + "'"
    }

    function Get-KomaOptions {
        param([Alias('Env')][hashtable]$EnvTable)
        $waitPid = 0
        $parsed = 0
        if ($EnvTable['KOMA_WAIT_PID'] -and [int]::TryParse([string]$EnvTable['KOMA_WAIT_PID'], [ref]$parsed) -and ($parsed -gt 0)) { $waitPid = $parsed }
        $ver = $null
        if ($EnvTable['KOMA_VERSION']) { $ver = ([string]$EnvTable['KOMA_VERSION']).Trim() }
        $base = $null
        if ($EnvTable['KOMA_RELEASE_BASE']) { $base = ([string]$EnvTable['KOMA_RELEASE_BASE']).Trim() }
        # Switches only count when set to exactly '1'.
        return [pscustomobject]@{
            Version      = $ver
            Base         = $base
            SkipChecksum = [bool]($EnvTable['KOMA_SKIP_CHECKSUM'] -eq '1')
            Uninstall    = [bool]($EnvTable['KOMA_UNINSTALL'] -eq '1')
            ForceClose   = [bool]($EnvTable['KOMA_FORCE_CLOSE'] -eq '1')
            WaitPid      = $waitPid
        }
    }

    function New-KomaElevatedScript {
        # Text of the one step that needs administrator rights. Every value is embedded as a
        # quoted literal. The body returns the msiexec exit code (it never calls exit).
        param(
            [string]$Mode = 'install',
            [string]$MsiPath = '',
            [string]$Sha256 = '',
            [bool]$StopKoma = $false,
            [string]$Bootstrapper = '',
            [string]$Msiexec = '',
            [string]$SafeId = '',
            [string]$ProductCode = ''
        )
        $body = @'
$ErrorActionPreference = 'Stop'
$mode = @@MODE@@
$src = @@MSI@@
$want = @@SHA@@
$stop = @@STOP@@
$boot = @@BOOT@@
$msiexec = @@MSIEXEC@@
$pcode = @@PCODE@@
$safe = Join-Path $env:windir ('Temp\koma-install-' + @@ID@@)
$code = 90002
try {
    New-Item -ItemType Directory -Path $safe -Force | Out-Null
    try { & icacls.exe $safe /grant '*S-1-5-32-545:(OI)(CI)R' | Out-Null } catch { }
    $log = Join-Path $safe 'koma-msi.log'
    if ($stop -eq '1') {
        Get-Process -Name koma -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
        for ($i = 0; $i -lt 20; $i++) {
            if (-not (Get-Process -Name koma -ErrorAction SilentlyContinue)) { break }
            Start-Sleep -Milliseconds 500
        }
    }
    $go = $true
    if ($mode -eq 'install') {
        $msi = Join-Path $safe 'koma-x64.msi'
        Copy-Item -LiteralPath $src -Destination $msi -Force
        if ($want) {
            $got = (Get-FileHash -LiteralPath $msi -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($got -ne $want) { $go = $false; $code = 90001 }
        }
        if ($go -and $boot) {
            try {
                $b = Join-Path $safe 'MicrosoftEdgeWebview2Setup.exe'
                Copy-Item -LiteralPath $boot -Destination $b -Force
                $sig = Get-AuthenticodeSignature -LiteralPath $b
                if ($sig.Status -eq 'Valid' -and $sig.SignerCertificate.Subject -match 'Microsoft Corporation') {
                    Start-Process -FilePath $b -ArgumentList '/silent', '/install' -Wait -WindowStyle Hidden | Out-Null
                }
            } catch { }
        }
        $margs = '/i "' + $msi + '" /qn /norestart /l*v "' + $log + '"'
    } else {
        $margs = '/x ' + $pcode + ' /qn /norestart /l*v "' + $log + '"'
    }
    if ($go) {
        $p = Start-Process -FilePath $msiexec -ArgumentList $margs -Wait -PassThru -WindowStyle Hidden
        $code = [int]$p.ExitCode
    }
} catch {
    $code = 90002
} finally {
    Remove-Item -LiteralPath (Join-Path $safe 'koma-x64.msi') -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath (Join-Path $safe 'MicrosoftEdgeWebview2Setup.exe') -Force -ErrorAction SilentlyContinue
    if ($code -eq 0 -or $code -eq 3010 -or $code -eq 1641) {
        Remove-Item -LiteralPath $safe -Recurse -Force -ErrorAction SilentlyContinue
    }
}
return [int]$code
'@
        $stopFlag = '0'
        if ($StopKoma) { $stopFlag = '1' }
        return $body.
            Replace('@@MODE@@', (Format-KomaLiteral $Mode)).
            Replace('@@MSI@@', (Format-KomaLiteral $MsiPath)).
            Replace('@@SHA@@', (Format-KomaLiteral $Sha256)).
            Replace('@@STOP@@', (Format-KomaLiteral $stopFlag)).
            Replace('@@BOOT@@', (Format-KomaLiteral $Bootstrapper)).
            Replace('@@MSIEXEC@@', (Format-KomaLiteral $Msiexec)).
            Replace('@@PCODE@@', (Format-KomaLiteral $ProductCode)).
            Replace('@@ID@@', (Format-KomaLiteral $SafeId))
    }

    # ---------------------------------------------------------------------
    # Output helpers
    # ---------------------------------------------------------------------

    function Write-KomaStep {
        param([string]$Label)
        $State.StepOpen = $true
        Write-Host ($Label.PadRight(30, '.') + ' ') -NoNewline
    }

    function Write-KomaResult {
        param([string]$Text, [string]$Color = 'Green')
        $State.StepOpen = $false
        Write-Host $Text -ForegroundColor $Color
    }

    function Write-KomaInfo {
        param([string]$Text, [string]$Color = 'Gray')
        if ($State.StepOpen) { Write-Host ''; $State.StepOpen = $false }
        Write-Host ((' ' * 31) + $Text) -ForegroundColor $Color
    }

    function Stop-KomaInstall {
        # Friendly failure: the message is shown as is, without a stack trace.
        param([string]$Message)
        $ex = New-Object System.Exception $Message
        $ex.Data['koma'] = $true
        throw $ex
    }

    function Show-KomaFailure {
        param($ErrorRecord, [string]$Verb)
        if ($State.StepOpen) { Write-Host 'FAILED' -ForegroundColor Red; $State.StepOpen = $false }
        Write-Host ''
        $ex = $ErrorRecord.Exception
        if ($ex.Data.Contains('koma')) {
            Write-Host "koma was NOT $Verb." -ForegroundColor Red
            Write-Host $ex.Message -ForegroundColor Red
        } else {
            Write-Host "koma was NOT $Verb. Something unexpected happened:" -ForegroundColor Red
            Write-Host $ex.Message -ForegroundColor Red
            Write-Host 'Run the command again. If it keeps happening, report it with the text above at https://github.com/aula-id/koma/issues' -ForegroundColor Red
        }
        Write-Host ''
    }

    # ---------------------------------------------------------------------
    # System helpers
    # ---------------------------------------------------------------------

    function Test-KomaAdmin {
        $id = [Security.Principal.WindowsIdentity]::GetCurrent()
        return ([Security.Principal.WindowsPrincipal]$id).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    }

    function Test-KomaInteractive {
        try { return ([Environment]::UserInteractive -and -not [Console]::IsInputRedirected) } catch { return $false }
    }

    function Save-KomaFile {
        param([string]$Url, [string]$Path, [int]$TimeoutSec = 600)
        $last = ''
        for ($i = 1; $i -le 3; $i++) {
            try {
                Invoke-WebRequest -Uri $Url -OutFile $Path -UseBasicParsing -TimeoutSec $TimeoutSec
                return
            } catch {
                $last = $_.Exception.Message
                if (Test-Path -LiteralPath $Path) { Remove-Item -LiteralPath $Path -Force -ErrorAction SilentlyContinue }
                if ($i -lt 3) { Start-Sleep -Seconds (2 * $i) }
            }
        }
        Stop-KomaInstall "Could not download $Url ($last). Check your internet connection or proxy, then run the command again."
    }

    function Get-KomaText {
        param([string]$Url)
        $r = Invoke-WebRequest -Uri $Url -UseBasicParsing -TimeoutSec 30
        if ($r.Content -is [byte[]]) { return [System.Text.Encoding]::UTF8.GetString($r.Content) }
        return [string]$r.Content
    }

    function Open-KomaRegistryKey {
        param([string]$Hive, [string]$View, [string]$SubKey)
        $h = [Microsoft.Win32.RegistryHive]::$Hive
        $v = [Microsoft.Win32.RegistryView]::$View
        $base = [Microsoft.Win32.RegistryKey]::OpenBaseKey($h, $v)
        try { return $base.OpenSubKey($SubKey) } catch { return $null }
    }

    function Find-KomaInstalled {
        # The MSI's entry in Add/Remove Programs (DisplayName "koma"), either registry view.
        foreach ($view in 'Registry64', 'Registry32') {
            $root = Open-KomaRegistryKey -Hive LocalMachine -View $view -SubKey 'SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall'
            if (-not $root) { continue }
            foreach ($name in $root.GetSubKeyNames()) {
                try {
                    $k = $root.OpenSubKey($name)
                    if ($k -and ($k.GetValue('DisplayName') -eq 'koma') -and ($name -match '^\{[0-9A-Fa-f-]{36}\}$')) {
                        return [pscustomobject]@{
                            ProductCode     = $name
                            Version         = [string]$k.GetValue('DisplayVersion')
                            InstallLocation = [string]$k.GetValue('InstallLocation')
                        }
                    }
                } catch { }
            }
        }
        return $null
    }

    function Get-KomaWebView2Version {
        $guid = '{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
        $places = @(
            @('LocalMachine', 'Registry64', "SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\$guid"),
            @('LocalMachine', 'Registry64', "SOFTWARE\Microsoft\EdgeUpdate\Clients\$guid"),
            @('LocalMachine', 'Registry32', "SOFTWARE\Microsoft\EdgeUpdate\Clients\$guid"),
            @('CurrentUser', 'Registry64', "Software\Microsoft\EdgeUpdate\Clients\$guid")
        )
        foreach ($p in $places) {
            try {
                $k = Open-KomaRegistryKey -Hive $p[0] -View $p[1] -SubKey $p[2]
                if ($k) {
                    $pv = [string]$k.GetValue('pv')
                    if ($pv -and $pv -ne '0.0.0.0') { return $pv }
                }
            } catch { }
        }
        return $null
    }

    function Find-KomaInstallDir {
        $cands = @()
        $inst = Find-KomaInstalled
        if ($inst -and $inst.InstallLocation) { $cands += $inst.InstallLocation }
        try {
            $sw = Open-KomaRegistryKey -Hive CurrentUser -View Registry64 -SubKey 'Software'
            if ($sw) {
                foreach ($n in $sw.GetSubKeyNames()) {
                    $k = $sw.OpenSubKey($n + '\koma')
                    if ($k) {
                        $d = [string]$k.GetValue('InstallDir')
                        if ($d) { $cands += $d }
                    }
                }
            }
        } catch { }
        $pf = $env:ProgramW6432
        if (-not $pf) { $pf = $env:ProgramFiles }
        if ($pf) { $cands += (Join-Path $pf 'koma') }
        foreach ($c in $cands) {
            $d = $c.Trim().Trim('"').TrimEnd('\')
            if ($d -and (Test-Path -LiteralPath (Join-Path $d 'koma.exe'))) { return $d }
        }
        return $null
    }

    function Set-KomaUserPath {
        # Edits HKCU\Environment\Path with the registry API so REG_EXPAND_SZ and any %VARS% survive.
        # Returns 'added', 'removed' or 'unchanged'.
        param([string]$Dir, [switch]$Remove)
        $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
        if (-not $key) { $key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Environment') }
        try {
            $cur = $key.GetValue('Path', $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
            $kind = [Microsoft.Win32.RegistryValueKind]::ExpandString
            if ($null -ne $cur) { $kind = $key.GetValueKind('Path') }
            if ($Remove) {
                $new = Remove-KomaPathEntry -Current ([string]$cur) -Dir $Dir
            } else {
                $new = Merge-KomaPath -Current ([string]$cur) -Dir $Dir
            }
            if ($new -ceq [string]$cur) { return 'unchanged' }
            $key.SetValue('Path', $new, $kind)
        } finally {
            $key.Close()
        }
        # Tell running programs (Explorer, new terminals) that the environment changed.
        try {
            if (-not ([System.Management.Automation.PSTypeName]'KomaNative.Win').Type) {
                Add-Type -Namespace KomaNative -Name Win -MemberDefinition '[System.Runtime.InteropServices.DllImport("user32.dll", SetLastError = true, CharSet = System.Runtime.InteropServices.CharSet.Auto)] public static extern System.IntPtr SendMessageTimeout(System.IntPtr hWnd, uint Msg, System.UIntPtr wParam, string lParam, uint fuFlags, uint uTimeout, out System.UIntPtr lpdwResult);'
            }
            $res = [UIntPtr]::Zero
            [void][KomaNative.Win]::SendMessageTimeout([IntPtr]0xffff, 0x1A, [UIntPtr]::Zero, 'Environment', 2, 5000, [ref]$res)
        } catch { }
        if ($Remove) { return 'removed' }
        return 'added'
    }

    function Update-KomaSessionPath {
        param([string]$Dir)
        $m = [Environment]::GetEnvironmentVariable('Path', 'Machine')
        $u = [Environment]::GetEnvironmentVariable('Path', 'User')
        $joined = Join-KomaSessionPath -Machine $m -User $u
        if ($Dir) { $joined = Merge-KomaPath -Current $joined -Dir $Dir }
        if ($joined) { $env:Path = $joined }
    }

    function Invoke-KomaElevated {
        # Runs the elevated step text; returns the msiexec exit code. One UAC prompt for a
        # non-administrator window, none for an administrator window.
        param([string]$Text)
        if (Test-KomaAdmin) {
            $sb = [scriptblock]::Create($Text)
            $out = @(& $sb)
            return [int]$out[$out.Count - 1]
        }
        # -EncodedCommand keeps the elevated code off disk, so nothing can swap it before it runs.
        $wrapped = '$komaR = @(& {' + "`n" + $Text + "`n" + '})' + "`n" + 'exit ([int]$komaR[$komaR.Count - 1])'
        $enc = [Convert]::ToBase64String([System.Text.Encoding]::Unicode.GetBytes($wrapped))
        $exe = [Diagnostics.Process]::GetCurrentProcess().MainModule.FileName
        $p = $null
        try {
            $p = Start-Process -FilePath $exe -Verb RunAs -Wait -PassThru -WindowStyle Hidden -ArgumentList @('-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-EncodedCommand', $enc)
        } catch {
            $inner = $_.Exception
            $native = 0
            if ($inner -is [System.ComponentModel.Win32Exception]) { $native = $inner.NativeErrorCode }
            if (($native -eq 1223) -or ($inner.Message -match 'canceled by the user|cancelled by the user')) {
                Stop-KomaInstall (Get-KomaMsiMessage -Code 1223).Text
            }
            Stop-KomaInstall ("Windows would not start the installer with administrator rights ($($inner.Message)). Open PowerShell as administrator and run the command again.")
        }
        return [int]$p.ExitCode
    }

    function Resolve-KomaRunning {
        # Handles a running koma before files are replaced. Returns $true when the elevated
        # step must stop koma (the user agreed, or KOMA_FORCE_CLOSE=1).
        param($Options, [string]$Action)
        $self = $PID
        if ($Options.WaitPid -gt 0) {
            Write-KomaInfo "waiting for process $($Options.WaitPid) to exit"
            Wait-Process -Id $Options.WaitPid -Timeout 60 -ErrorAction SilentlyContinue
        }
        $procs = @(Get-Process -Name koma -ErrorAction SilentlyContinue | Where-Object { $_.Id -ne $self })
        if ($procs.Count -eq 0) { return $false }
        $word = 'processes'
        if ($procs.Count -eq 1) { $word = 'process' }
        if ($Options.ForceClose) {
            Write-KomaInfo "koma is running ($($procs.Count) $word); closing it (KOMA_FORCE_CLOSE=1)" 'Yellow'
        } elseif (Test-KomaInteractive) {
            if ($State.StepOpen) { Write-Host ''; $State.StepOpen = $false }
            $ans = Read-Host "      koma is running ($($procs.Count) $word). Close it now to $Action? [Y/n]"
            if ($ans -and $ans.Trim().ToLowerInvariant().StartsWith('n')) {
                Stop-KomaInstall 'koma is still running. Close it, then run the command again.'
            }
        } else {
            Stop-KomaInstall 'koma is running and this window cannot ask you. Close koma and run the command again, or set $env:KOMA_FORCE_CLOSE = ''1'' to close it automatically.'
        }
        foreach ($p in $procs) { try { [void]$p.CloseMainWindow() } catch { } }
        for ($i = 0; $i -lt 10; $i++) {
            if (-not (Get-Process -Name koma -ErrorAction SilentlyContinue | Where-Object { $_.Id -ne $self })) { break }
            Start-Sleep -Milliseconds 500
        }
        return $true
    }

    function Save-KomaFailedLog {
        # Copies the admin-side msiexec log somewhere the user owns; returns the path to print.
        param([string]$SafeId, [string]$Name)
        $src = Join-Path $env:windir ("Temp\koma-install-$SafeId\$Name")
        if (-not (Test-Path -LiteralPath $src)) { return $null }
        try {
            $dir = Join-Path $env:LOCALAPPDATA 'koma\logs'
            New-Item -ItemType Directory -Path $dir -Force | Out-Null
            $dest = Join-Path $dir ('install-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '.log')
            Copy-Item -LiteralPath $src -Destination $dest -Force
            Get-ChildItem -LiteralPath $dir -Filter 'install-*.log' -ErrorAction SilentlyContinue |
                Where-Object { $_.LastWriteTime -lt (Get-Date).AddDays(-14) } |
                Remove-Item -Force -ErrorAction SilentlyContinue
            return $dest
        } catch {
            return $src
        }
    }

    # ---------------------------------------------------------------------
    # Flows
    # ---------------------------------------------------------------------

    function Invoke-KomaInstall {
        param($Options)
        Write-KomaStep '[1/5] Checking this PC'
        $build = [Environment]::OSVersion.Version.Build
        $arch = Resolve-KomaArch -ProcArch $env:PROCESSOR_ARCHITECTURE -ProcArchW6432 $env:PROCESSOR_ARCHITEW6432 -OsBuild $build
        if (-not $arch.Ok) { Stop-KomaInstall $arch.Message }
        if ($Options.Version -and -not $Options.Base -and -not (Test-KomaVersionTag $Options.Version)) {
            Stop-KomaInstall "KOMA_VERSION '$($Options.Version)' is not a release tag. Use something like v0.2.27."
        }
        $base = Get-KomaBaseUrl -Base $Options.Base -Version $Options.Version
        if (-not (Test-KomaBaseUrl $base)) { Stop-KomaInstall "KOMA_RELEASE_BASE must be an https:// address (got '$base')." }
        $winName = 'Windows 10'
        if ($build -ge 22000) { $winName = 'Windows 11' }
        $prev = Find-KomaInstalled
        $line = "$winName x64"
        if ($arch.Emulated) { $line = "$winName ARM64 (koma runs as an x64 app)" }
        if ($prev -and $prev.Version) { $line = $line + ", koma $($prev.Version) installed" }
        Write-KomaResult $line

        $id = [guid]::NewGuid().ToString('N')
        $State.WorkDir = Join-Path ([IO.Path]::GetTempPath()) ('koma-install-' + $id)
        New-Item -ItemType Directory -Path $State.WorkDir -Force | Out-Null
        $msi = Join-Path $State.WorkDir 'koma-x64.msi'

        Write-KomaStep '[2/5] Downloading koma'
        Save-KomaFile -Url "$base/koma-x64.msi" -Path $msi
        $size = (Get-Item -LiteralPath $msi).Length
        Write-KomaResult ((Format-KomaBytes $size) + '  ok')

        Write-KomaStep '[3/5] Verifying download'
        $hash = ''
        if ($Options.SkipChecksum) {
            Write-KomaResult 'skipped (KOMA_SKIP_CHECKSUM=1)' 'Yellow'
        } else {
            $sumsText = $null
            try { $sumsText = Get-KomaText "$base/SHA256SUMS" } catch { $sumsText = $null }
            if (-not $sumsText) {
                Stop-KomaInstall 'This release has no SHA256SUMS, so the download cannot be verified. Nothing was installed. (Advanced: set $env:KOMA_SKIP_CHECKSUM = ''1'' to skip the check.)'
            }
            $sums = ConvertFrom-KomaSums $sumsText
            if (-not $sums.ContainsKey('koma-x64.msi')) {
                Stop-KomaInstall 'SHA256SUMS does not list koma-x64.msi, so the download cannot be verified. Nothing was installed.'
            }
            $hash = (Get-FileHash -LiteralPath $msi -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($hash -ne $sums['koma-x64.msi']) {
                Stop-KomaInstall ("The downloaded file does not match the checksum published with the release.`n  expected $($sums['koma-x64.msi'])`n  got      $hash`nRun the command again. If it keeps happening, your network or a proxy may be altering the download; report it at https://github.com/aula-id/koma/issues")
            }
            Write-KomaResult 'sha256 ok'
        }

        $action = 'install'
        if ($prev) { $action = 'upgrade' }
        Write-KomaStep '[4/5] Installing'
        if (Test-KomaAdmin) {
            Write-KomaResult 'already administrator, no prompt'
        } else {
            Write-KomaResult 'Windows will ask for permission once (click Yes)' 'Yellow'
        }

        $boot = ''
        if (-not (Get-KomaWebView2Version)) {
            Write-KomaInfo 'WebView2 runtime missing: installing it first'
            $b = Join-Path $State.WorkDir 'MicrosoftEdgeWebview2Setup.exe'
            try {
                Save-KomaFile -Url 'https://go.microsoft.com/fwlink/p/?LinkId=2124703' -Path $b -TimeoutSec 120
                $boot = $b
            } catch {
                Write-KomaInfo 'could not download WebView2. The terminal app works without it; the desktop app needs it from https://developer.microsoft.com/microsoft-edge/webview2' 'Yellow'
            }
        }

        $stop = Resolve-KomaRunning -Options $Options -Action $action
        $msiexec = Get-KomaMsiexecPath -WinDir $env:windir -Is64Process ([Environment]::Is64BitProcess) -Is64OS ([Environment]::Is64BitOperatingSystem)
        $text = New-KomaElevatedScript -Mode 'install' -MsiPath $msi -Sha256 $hash -StopKoma $stop -Bootstrapper $boot -Msiexec $msiexec -SafeId $id
        $code = Invoke-KomaElevated -Text $text
        $msg = Get-KomaMsiMessage -Code $code
        if ($msg.Kind -eq 'Fail') {
            $logPath = Save-KomaFailedLog -SafeId $id -Name 'koma-msi.log'
            $full = (Get-KomaMsiMessage -Code $code -LogPath $logPath).Text
            if (($code -ne 90001) -and ($code -ne 90002)) { $full = "(Windows Installer error $code) " + $full }
            Stop-KomaInstall $full
        }

        Write-KomaStep '[5/5] Finishing up'
        $now = Find-KomaInstalled
        $dir = Find-KomaInstallDir
        $pathNote = 'koma was installed but its folder was not found, so PATH was not changed'
        if ($dir) {
            $r = Set-KomaUserPath -Dir $dir
            if ($r -eq 'added') { $pathNote = 'added to your PATH, shortcuts created' } else { $pathNote = 'PATH already contains koma, shortcuts created' }
            Update-KomaSessionPath -Dir $dir
        } else {
            Update-KomaSessionPath
        }
        Write-KomaResult $pathNote
        $ver = ''
        if ($now -and $now.Version) { $ver = ' ' + $now.Version }
        $where = ''
        if ($dir) { $where = " (in $dir)" }
        Write-Host ''
        if ($prev) {
            Write-Host "koma$ver upgraded$where." -ForegroundColor Green
        } else {
            Write-Host "koma$ver installed$where." -ForegroundColor Green
        }
        if ($msg.Kind -eq 'Reboot') { Write-Host $msg.Text -ForegroundColor Yellow }
        Write-Host ''
        Write-Host 'Type  koma  to start in the terminal, or  koma gui  for the desktop app.'
        Write-Host 'A Start Menu entry "koma" also opens the desktop app.  https://koma.run'
        Write-Host ''
    }

    function Invoke-KomaUninstall {
        param($Options)
        Write-KomaStep '[1/2] Looking for koma'
        $prev = Find-KomaInstalled
        if (-not $prev) {
            Write-KomaResult 'not installed'
            Write-Host ''
            Write-Host 'koma is not installed, nothing to do.'
            Write-Host ''
            return
        }
        Write-KomaResult "koma $($prev.Version)"
        $dir = $prev.InstallLocation
        $id = [guid]::NewGuid().ToString('N')
        Write-KomaStep '[2/2] Removing'
        if (Test-KomaAdmin) {
            Write-KomaResult 'already administrator, no prompt'
        } else {
            Write-KomaResult 'Windows will ask for permission once (click Yes)' 'Yellow'
        }
        $stop = Resolve-KomaRunning -Options $Options -Action 'uninstall'
        $msiexec = Get-KomaMsiexecPath -WinDir $env:windir -Is64Process ([Environment]::Is64BitProcess) -Is64OS ([Environment]::Is64BitOperatingSystem)
        $text = New-KomaElevatedScript -Mode 'uninstall' -StopKoma $stop -Msiexec $msiexec -SafeId $id -ProductCode $prev.ProductCode
        $code = Invoke-KomaElevated -Text $text
        $msg = Get-KomaMsiMessage -Code $code
        if ($msg.Kind -eq 'Fail') {
            $logPath = Save-KomaFailedLog -SafeId $id -Name 'koma-msi.log'
            Stop-KomaInstall ("(Windows Installer error $code) " + (Get-KomaMsiMessage -Code $code -LogPath $logPath).Text)
        }
        if ($dir) { [void](Set-KomaUserPath -Dir $dir.TrimEnd('\') -Remove) }
        Update-KomaSessionPath
        Write-Host ''
        Write-Host 'koma was removed. Your sessions and settings were kept; delete the .koma folder in your user folder to remove them too.' -ForegroundColor Green
        if ($msg.Kind -eq 'Reboot') { Write-Host $msg.Text -ForegroundColor Yellow }
        Write-Host ''
    }

    # Tests dot-source the block with -DefineOnly to get the functions above and nothing else.
    if ($DefineOnly) { return }

    # ---------------------------------------------------------------------
    # Run. Preferences set here belong to this script block's scope only.
    # ---------------------------------------------------------------------
    Set-StrictMode -Version 1.0
    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'
    $State = @{ StepOpen = $false; WorkDir = $null }
    $failed = $false
    $verb = 'installed'

    Write-Host ''
    Write-Host 'koma installer  (https://koma.run)' -ForegroundColor Cyan
    Write-Host ''
    try {
        # Add TLS 1.2 to what the process already allows (older Windows PowerShell defaults to TLS 1.0).
        try { [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12 } catch { }
        $envTable = @{}
        foreach ($n in 'KOMA_VERSION', 'KOMA_RELEASE_BASE', 'KOMA_SKIP_CHECKSUM', 'KOMA_UNINSTALL', 'KOMA_FORCE_CLOSE', 'KOMA_WAIT_PID') {
            $envTable[$n] = [Environment]::GetEnvironmentVariable($n)
        }
        $opts = Get-KomaOptions -Env $envTable
        if ($opts.Uninstall) {
            $verb = 'removed'
            Invoke-KomaUninstall -Options $opts
        } else {
            Invoke-KomaInstall -Options $opts
        }
    } catch {
        $failed = $true
        Show-KomaFailure -ErrorRecord $_ -Verb $verb
    } finally {
        if ($State.WorkDir -and (Test-Path -LiteralPath $State.WorkDir)) {
            Remove-Item -LiteralPath $State.WorkDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
    # Callers such as `koma update` read the result from LASTEXITCODE.
    if ($failed) { $global:LASTEXITCODE = 1 } else { $global:LASTEXITCODE = 0 }
    # Never exit under iex (it would close the user's window); only a script file may.
    $cmdPath = Get-Variable -Name PSCommandPath -ErrorAction SilentlyContinue
    if ($failed -and $cmdPath -and $cmdPath.Value) { exit 1 }
}

if ($env:KOMA_INSTALL_NO_RUN -eq '1') { . $komaBody -DefineOnly } else { & $komaBody }
