# GUI release packaging only. Pinned upstream installer with SHA-256 verification.
# Extract it; do not run it. MultiUser.nsh overwrites $INSTDIR in .onInit, so
# /S /D= still installs to Program Files and leaves this staging directory empty.
# English data is not in the installer. Pin the tessdata_fast file the installer downloads.
$ErrorActionPreference = 'Stop'
$version = '5.5.0.20241111'
$expected = 'F3FC4236425B690C8BE756F35793F77394EE004BE0A6460A440C754D892F68BC'
$engRevision = '87416418657359cb625c412a48b6e1d6d41c29bd'
$engExpected = '7D4322BD2A7749724879683FC3912CB542F19906C83BCC1A52132556427170B2'
$scratchRoot = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [IO.Path]::GetTempPath() }
$directory = Join-Path $scratchRoot ('koma-ocr-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $directory | Out-Null
$installer = Join-Path $directory 'tesseract-installer.exe'
$installation = Join-Path $directory 'runtime'
Invoke-WebRequest -Uri "https://github.com/tesseract-ocr/tesseract/releases/download/5.5.0/tesseract-ocr-w64-setup-$version.exe" -OutFile $installer
if ((Get-FileHash -Algorithm SHA256 $installer).Hash -ne $expected) { throw 'Tesseract installer checksum mismatch' }
$sevenZipCandidates = @()
if ($env:ProgramFiles) { $sevenZipCandidates += Join-Path $env:ProgramFiles '7-Zip\7z.exe' }
$programFiles86 = ${env:ProgramFiles(x86)}
if ($programFiles86) { $sevenZipCandidates += Join-Path $programFiles86 '7-Zip\7z.exe' }
$sevenZip = $sevenZipCandidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
if (-not $sevenZip) {
    $found = Get-Command 7z.exe -ErrorAction SilentlyContinue
    if ($found) { $sevenZip = $found.Source }
}
if (-not $sevenZip) { throw '7-Zip is required to extract the Tesseract installer' }
New-Item -ItemType Directory -Path $installation | Out-Null
& $sevenZip x $installer "-o$installation" -y
if ($LASTEXITCODE -ne 0) { throw "Tesseract installer extraction failed: $LASTEXITCODE" }
$plugins = Join-Path $installation '$PLUGINSDIR'
if (Test-Path -LiteralPath $plugins) { Remove-Item -LiteralPath $plugins -Recurse -Force }
$eng = Join-Path $installation 'tessdata\eng.traineddata'
New-Item -ItemType Directory -Path (Split-Path -Parent $eng) -Force | Out-Null
Invoke-WebRequest -Uri "https://raw.githubusercontent.com/tesseract-ocr/tessdata_fast/$engRevision/eng.traineddata" -OutFile $eng
if ((Get-FileHash -Algorithm SHA256 $eng).Hash -ne $engExpected) { throw 'English traineddata checksum mismatch' }
python scripts/stage_windows_ocr.py $installation
if ($LASTEXITCODE -ne 0) { throw 'OCR staging failed' }
# Verify the staged executable and its sibling DLLs/data before WiX consumes them.
$env:TESSDATA_PREFIX = (Resolve-Path 'packaging/windows/ocr/tessdata').Path
& 'packaging/windows/ocr/tesseract.exe' --list-langs
if ($LASTEXITCODE -ne 0) { throw 'Staged Tesseract runtime cannot load' }
