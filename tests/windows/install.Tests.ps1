# Pester 5 tests for the pure functions in install.ps1.
# Run on Windows (Windows PowerShell 5.1 and PowerShell 7):  Invoke-Pester tests/windows
# These tests only define the functions (KOMA_INSTALL_NO_RUN=1). They never run the installer.

BeforeAll {
    $script:InstallScript = Join-Path $PSScriptRoot '..\..\install.ps1'
    $env:KOMA_INSTALL_NO_RUN = '1'
    . $script:InstallScript
}

AfterAll {
    Remove-Item Env:\KOMA_INSTALL_NO_RUN -ErrorAction SilentlyContinue
}

Describe 'Resolve-KomaArch' {
    It 'accepts native AMD64' {
        (Resolve-KomaArch -ProcArch 'AMD64' -ProcArchW6432 '' -OsBuild 19045).Ok | Should -BeTrue
    }
    It 'uses PROCESSOR_ARCHITEW6432 for a 32-bit host on 64-bit Windows' {
        $r = Resolve-KomaArch -ProcArch 'x86' -ProcArchW6432 'AMD64' -OsBuild 22631
        $r.Ok | Should -BeTrue
        $r.Native | Should -Be 'AMD64'
    }
    It 'allows ARM64 on Windows 11 (x64 emulation)' {
        $r = Resolve-KomaArch -ProcArch 'ARM64' -ProcArchW6432 '' -OsBuild 22631
        $r.Ok | Should -BeTrue
        $r.Emulated | Should -BeTrue
    }
    It 'rejects ARM64 on Windows 10' {
        $r = Resolve-KomaArch -ProcArch 'ARM64' -ProcArchW6432 '' -OsBuild 19045
        $r.Ok | Should -BeFalse
        $r.Message | Should -Match 'Windows 11'
    }
    It 'rejects 32-bit Windows' {
        (Resolve-KomaArch -ProcArch 'x86' -ProcArchW6432 '' -OsBuild 19045).Ok | Should -BeFalse
    }
    It 'rejects builds older than Windows 10 1809' {
        (Resolve-KomaArch -ProcArch 'AMD64' -ProcArchW6432 '' -OsBuild 17134).Ok | Should -BeFalse
    }
}

Describe 'URL building' {
    It 'defaults to the latest release' {
        Get-KomaAssetUrl -Base '' -Version '' | Should -Be 'https://github.com/aula-id/koma/releases/latest/download/koma-x64.msi'
    }
    It 'adds the v prefix to a version' {
        Get-KomaAssetUrl -Base '' -Version '0.2.27' | Should -Be 'https://github.com/aula-id/koma/releases/download/v0.2.27/koma-x64.msi'
    }
    It 'keeps an existing v' {
        Get-KomaAssetUrl -Base '' -Version 'v0.2.27' | Should -Match '/download/v0\.2\.27/'
    }
    It 'lets the base override win and trims the trailing slash' {
        Get-KomaAssetUrl -Base 'https://example.test/r/' -Version 'v1.2.3' | Should -Be 'https://example.test/r/koma-x64.msi'
    }
    It 'validates version tags' {
        Test-KomaVersionTag 'v1.2.3' | Should -BeTrue
        Test-KomaVersionTag '1.2.3-rc.1' | Should -BeTrue
        Test-KomaVersionTag '1.2;calc' | Should -BeFalse
        Test-KomaVersionTag '../x' | Should -BeFalse
        Test-KomaVersionTag '' | Should -BeFalse
    }
    It 'requires https except for localhost' {
        Test-KomaBaseUrl 'https://example.test/r' | Should -BeTrue
        Test-KomaBaseUrl 'http://127.0.0.1:8080/r' | Should -BeTrue
        Test-KomaBaseUrl 'http://example.test/r' | Should -BeFalse
    }
}

Describe 'ConvertFrom-KomaSums' {
    BeforeAll { $script:h = 'a' * 64 }
    It 'parses LF and CRLF and lowercases the hash' {
        $m = ConvertFrom-KomaSums ("$($script:h.ToUpper())  koma-x64.msi`r`n$($script:h)  other.bin`n")
        $m['koma-x64.msi'] | Should -Be $script:h
        $m['other.bin'] | Should -Be $script:h
    }
    It 'handles the * binary marker and a ./ prefix' {
        (ConvertFrom-KomaSums "$($script:h) *koma-x64.msi")['koma-x64.msi'] | Should -Be $script:h
        (ConvertFrom-KomaSums "$($script:h)  ./koma-x64.msi")['koma-x64.msi'] | Should -Be $script:h
    }
    It 'ignores malformed lines' {
        (ConvertFrom-KomaSums 'zzz  koma-x64.msi').ContainsKey('koma-x64.msi') | Should -BeFalse
    }
    It 'returns an empty table for blank input' {
        (ConvertFrom-KomaSums '').Count | Should -Be 0
    }
}

Describe 'Get-KomaMsiMessage' {
    It 'treats 0 as ok and 3010 as reboot' {
        (Get-KomaMsiMessage -Code 0).Kind | Should -Be 'Ok'
        (Get-KomaMsiMessage -Code 3010).Kind | Should -Be 'Reboot'
    }
    It 'explains 1602' { (Get-KomaMsiMessage -Code 1602).Text | Should -Match 'cancel' }
    It 'explains 1618' { (Get-KomaMsiMessage -Code 1618).Text | Should -Match 'Another Windows installation' }
    It 'explains 1603 and cites the log' {
        $m = Get-KomaMsiMessage -Code 1603 -LogPath 'C:\x.log'
        $m.Kind | Should -Be 'Fail'
        $m.Text | Should -Match 'C:\\x\.log'
    }
    It 'explains a declined UAC prompt' { (Get-KomaMsiMessage -Code 1223).Text | Should -Match 'declined' }
    It 'names unknown codes' { (Get-KomaMsiMessage -Code 9999).Text | Should -Match '9999' }
}

Describe 'PATH helpers' {
    It 'appends when missing' {
        Merge-KomaPath -Current 'C:\a;C:\b' -Dir 'C:\koma' | Should -Be 'C:\a;C:\b;C:\koma'
    }
    It 'is idempotent and ignores case and trailing slash' {
        Merge-KomaPath -Current 'C:\a;c:\KOMA\' -Dir 'C:\koma' | Should -Be 'C:\a;c:\KOMA\'
    }
    It 'handles an empty current value' {
        Merge-KomaPath -Current '' -Dir 'C:\koma' | Should -Be 'C:\koma'
    }
    It 'drops stray empty segments when it adds' {
        Merge-KomaPath -Current 'C:\a;;' -Dir 'C:\koma' | Should -Be 'C:\a;C:\koma'
    }
    It 'keeps %VAR% entries as written' {
        Merge-KomaPath -Current '%USERPROFILE%\bin' -Dir 'C:\koma' | Should -Be '%USERPROFILE%\bin;C:\koma'
    }
    It 'counts %ProgramFiles%\koma as present' {
        Merge-KomaPath -Current '%ProgramFiles%\koma' -Dir "$env:ProgramFiles\koma" | Should -Be '%ProgramFiles%\koma'
    }
    It 'removes an entry' {
        Remove-KomaPathEntry -Current 'C:\a;C:\koma;C:\b' -Dir 'C:\koma\' | Should -Be 'C:\a;C:\b'
    }
    It 'puts Machine before User in the session path' {
        Join-KomaSessionPath -Machine 'M1;M2' -User 'U1' | Should -Be 'M1;M2;U1'
        Join-KomaSessionPath -Machine '' -User 'U1' | Should -Be 'U1'
    }
}

Describe 'Get-KomaMsiexecPath' {
    It 'uses Sysnative for 32-bit PowerShell on 64-bit Windows' {
        Get-KomaMsiexecPath -WinDir 'C:\Windows' -Is64Process $false -Is64OS $true | Should -Be 'C:\Windows\Sysnative\msiexec.exe'
    }
    It 'uses System32 otherwise' {
        Get-KomaMsiexecPath -WinDir 'C:\Windows' -Is64Process $true -Is64OS $true | Should -Be 'C:\Windows\System32\msiexec.exe'
    }
}

Describe 'New-KomaElevatedScript' {
    It 'is valid PowerShell, escapes quotes and runs msiexec quietly' {
        $t = New-KomaElevatedScript -MsiPath "C:\o'brien\koma.msi" -Sha256 ('b' * 64) -StopKoma $true -Msiexec 'C:\Windows\System32\msiexec.exe' -SafeId 'x'
        { [scriptblock]::Create($t) } | Should -Not -Throw
        $t | Should -Match "o''brien"
        $t | Should -Match '/qn /norestart'
        $t | Should -Not -Match '/passive'
        $t | Should -Not -Match '(?m)^\s*exit\b'
    }
    It 'builds a valid uninstall step' {
        $t = New-KomaElevatedScript -Mode 'uninstall' -Msiexec 'C:\Windows\System32\msiexec.exe' -SafeId 'x' -ProductCode '{00000000-0000-0000-0000-000000000000}'
        { [scriptblock]::Create($t) } | Should -Not -Throw
        $t | Should -Match '/x '
    }
}

Describe 'Get-KomaOptions' {
    It 'only accepts exactly 1 for switches' {
        $o = Get-KomaOptions -Env @{ KOMA_SKIP_CHECKSUM = 'true'; KOMA_FORCE_CLOSE = 'yes' }
        $o.SkipChecksum | Should -BeFalse
        $o.ForceClose | Should -BeFalse
    }
    It 'reads version, uninstall and wait pid' {
        $o = Get-KomaOptions -Env @{ KOMA_VERSION = 'v1.2.3'; KOMA_UNINSTALL = '1'; KOMA_WAIT_PID = '42' }
        $o.Version | Should -Be 'v1.2.3'
        $o.Uninstall | Should -BeTrue
        $o.WaitPid | Should -Be 42
    }
}

Describe 'Session hygiene' {
    It 'defines the script block only inside the guarded call' {
        (Get-Content -LiteralPath $script:InstallScript -Raw) | Should -Match 'KOMA_INSTALL_NO_RUN'
    }
    It 'has no top-level exit that would close an iex caller' {
        $lines = Get-Content -LiteralPath $script:InstallScript
        ($lines | Where-Object { $_ -match '^exit\b' }).Count | Should -Be 0
    }
}
