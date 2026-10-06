# Run against the actual MSI's administrative extraction, never a host Git install.
param([string]$Directory = 'target/release')
$ErrorActionPreference = 'Stop'
$artifact = Get-ChildItem $Directory -Filter 'koma*.msi' | Select-Object -First 1
if (!$artifact) { throw 'No Koma MSI was built' }
$scratchRoot = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [IO.Path]::GetTempPath() }
$extract = Join-Path $scratchRoot ('koma-msi-shell-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $extract | Out-Null
$process = Start-Process msiexec.exe -ArgumentList "/a `"$($artifact.FullName)`" /qn TARGETDIR=`"$extract`"" -Wait -PassThru
if ($process.ExitCode -ne 0) { throw 'MSI administrative extraction failed' }
$bash = Get-ChildItem $extract -Filter bash.exe -Recurse | Where-Object {
    $_.Directory.Name -eq 'bin' -and $_.Directory.Parent.Name -eq 'usr' -and $_.Directory.Parent.Parent.Name -eq 'shell'
} | Select-Object -First 1
if (!$bash) { throw 'Packaged shell\usr\bin\bash.exe missing' }
foreach ($name in @('grep.exe', 'ls.exe', 'sed.exe', 'find.exe', 'msys-2.0.dll')) {
    if (!(Test-Path (Join-Path $bash.Directory.FullName $name))) { throw "Packaged shell is missing $name" }
}
$license = Get-ChildItem $extract -Filter LICENSE.txt -Recurse | Where-Object { $_.Directory.Name -eq 'shell' } | Select-Object -First 1
if (!$license) { throw 'Packaged shell LICENSE.txt missing' }
$env:PATH = "$($bash.Directory.FullName);$env:PATH"
$env:MSYSTEM = 'MINGW64'
& $bash.FullName -c 'grep --version && ls --version && sed --version'
if ($LASTEXITCODE -ne 0) { throw 'Packaged Git Bash cannot run grep, ls, and sed' }
