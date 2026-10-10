# GUI release packaging only. Pinned Git for Windows portable archive.
# Extract it; do not run the upstream installer. Stage usr/bin (bash, coreutils,
# grep, sed, find, MSYS DLLs), terminfo, and the upstream license, then prove
# bash -c can run grep, ls, and sed.
$ErrorActionPreference = 'Stop'
$version = '2.56.0'
$tag = 'v2.56.0.windows.1'
$expected = 'ECEB5E061AA90DF2F69DDD3E90F0030E1B8037A7829934BC40E4BE1CAA1ACCC1'
$scratchRoot = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [IO.Path]::GetTempPath() }
$directory = Join-Path $scratchRoot ('koma-shell-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $directory | Out-Null
$archive = Join-Path $directory "PortableGit-$version-64-bit.7z.exe"
$installation = Join-Path $directory 'runtime'
Invoke-WebRequest -Uri "https://github.com/git-for-windows/git/releases/download/$tag/PortableGit-$version-64-bit.7z.exe" -OutFile $archive
if ((Get-FileHash -Algorithm SHA256 $archive).Hash -ne $expected) { throw 'PortableGit checksum mismatch' }
$sevenZipCandidates = @()
if ($env:ProgramFiles) { $sevenZipCandidates += Join-Path $env:ProgramFiles '7-Zip\7z.exe' }
$programFiles86 = ${env:ProgramFiles(x86)}
if ($programFiles86) { $sevenZipCandidates += Join-Path $programFiles86 '7-Zip\7z.exe' }
$sevenZip = $sevenZipCandidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
if (-not $sevenZip) {
    $found = Get-Command 7z.exe -ErrorAction SilentlyContinue
    if ($found) { $sevenZip = $found.Source }
}
if (-not $sevenZip) { throw '7-Zip is required to extract PortableGit' }
New-Item -ItemType Directory -Path $installation | Out-Null
& $sevenZip x $archive "-o$installation" -y
if ($LASTEXITCODE -ne 0) { throw "PortableGit extraction failed: $LASTEXITCODE" }
python scripts/stage_windows_shell.py $installation
if ($LASTEXITCODE -ne 0) { throw 'Shell staging failed' }
$bash = Join-Path (Resolve-Path 'packaging/windows/shell/usr/bin').Path 'bash.exe'
$usrBin = Split-Path -Parent $bash
$env:PATH = "$usrBin;$env:PATH"
$env:MSYSTEM = 'MINGW64'
& $bash -c 'grep --version && ls --version && sed --version'
if ($LASTEXITCODE -ne 0) { throw 'Staged Git Bash cannot run grep, ls, and sed' }
