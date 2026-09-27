# GUI release packaging only. Pinned upstream installer with SHA-256 verification.
# Installs into an isolated runner staging directory; WiX ships runtime/data only.
$ErrorActionPreference = 'Stop'
$version = '5.5.0.20241111'
$expected = 'F3FC4236425B690C8BE756F35793F77394EE004BE0A6460A440C754D892F68BC'
$scratchRoot = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [IO.Path]::GetTempPath() }
$directory = Join-Path $scratchRoot ('koma-ocr-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $directory | Out-Null
$installer = Join-Path $directory 'tesseract-installer.exe'
$installation = Join-Path $directory 'runtime'
Invoke-WebRequest -Uri "https://github.com/tesseract-ocr/tesseract/releases/download/5.5.0/tesseract-ocr-w64-setup-$version.exe" -OutFile $installer
if ((Get-FileHash -Algorithm SHA256 $installer).Hash -ne $expected) { throw 'Tesseract installer checksum mismatch' }
# NSIS requires /D to be the last argument; Start-Process must not add quotes to it.
$process = Start-Process -FilePath $installer -ArgumentList "/S /D=$installation" -Wait -PassThru
if ($process.ExitCode -ne 0) { throw "Tesseract staging installer failed: $($process.ExitCode)" }
python scripts/stage_windows_ocr.py $installation
if ($LASTEXITCODE -ne 0) { throw 'OCR staging failed' }
# Verify the staged executable and its sibling DLLs/data before WiX consumes them.
$env:TESSDATA_PREFIX = (Resolve-Path 'packaging/windows/ocr/tessdata').Path
& 'packaging/windows/ocr/tesseract.exe' --list-langs
if ($LASTEXITCODE -ne 0) { throw 'Staged Tesseract runtime cannot load' }
