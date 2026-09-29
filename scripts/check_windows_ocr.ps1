# Run against the actual MSI's administrative extraction, never host Tesseract.
param([string]$Directory = 'target/release')
$ErrorActionPreference = 'Stop'
$artifact = Get-ChildItem $Directory -Filter 'koma*.msi' | Select-Object -First 1
if (!$artifact) { throw 'No Koma MSI was built' }
$scratchRoot = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [IO.Path]::GetTempPath() }
$extract = Join-Path $scratchRoot ('koma-msi-check-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $extract | Out-Null
$process = Start-Process msiexec.exe -ArgumentList "/a `"$($artifact.FullName)`" /qn TARGETDIR=`"$extract`"" -Wait -PassThru
if ($process.ExitCode -ne 0) { throw 'MSI administrative extraction failed' }
$binary = Get-ChildItem $extract -Filter tesseract.exe -Recurse | Select-Object -First 1
if (!$binary) { throw 'Packaged Tesseract executable missing' }
$env:TESSDATA_PREFIX = Join-Path $binary.Directory.FullName 'tessdata'
if (!(Test-Path (Join-Path $env:TESSDATA_PREFIX 'eng.traineddata'))) { throw 'Packaged English data missing' }
$fixture = Join-Path $extract 'blank.pgm'
[byte[]]$header = [Text.Encoding]::ASCII.GetBytes("P5`n100 50`n255`n")
[byte[]]$pixels = @(255) * 5000
[IO.File]::WriteAllBytes($fixture, $header + $pixels)
& $binary.FullName $fixture stdout -l eng --psm 6
if ($LASTEXITCODE -ne 0) { throw 'Packaged OCR could not load English and process the fixture' }
