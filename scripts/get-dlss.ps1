# Fetches the NVIDIA files Defeat's DLSS needs, from NVIDIA's own GitHub release of the Streamline SDK, into the
# folder the runner looks in: streamline\ in Defeat's data folder (%LOCALAPPDATA%\victory\data unless
# VICTORY_DATA_DIR or victory.toml says otherwise; VICTORY_DLSS_DIR names another folder for the game).
#
#   powershell -ExecutionPolicy Bypass -File scripts\get-dlss.ps1
#
# The files are NVIDIA's, under NVIDIA's licences (saved beside them: read them). This repository does not
# carry them, so each person gets them from NVIDIA. The SDK's zip is about 280 MB; only the DLLs DLSS loads
# and the licences are kept. Nothing of NVIDIA's is needed to build Defeat: the files load when the game runs.
param(
    [string]$Tag = 'v2.14.16',
    [string]$To = ''
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'   # Invoke-WebRequest's progress bar makes it many times slower
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
if (-not $To) {
    $data = if ($env:VICTORY_DATA_DIR) { $env:VICTORY_DATA_DIR } else { Join-Path $env:LOCALAPPDATA 'victory\data' }
    $To = Join-Path $data 'streamline'
}
# What DLSS Super Resolution loads: Streamline's interposer, its common plugin, the DLSS plugin and NVIDIA's model.
$want = 'sl.interposer.dll', 'sl.common.dll', 'sl.dlss.dll', 'nvngx_dlss.dll'
$url = "https://github.com/NVIDIA-RTX/Streamline/releases/download/$Tag/streamline-sdk-$Tag.zip"
$zip = Join-Path ([IO.Path]::GetTempPath()) "streamline-sdk-$Tag.zip"
New-Item -ItemType Directory -Force $To | Out-Null
Write-Host "Getting NVIDIA's Streamline SDK $Tag (about 280 MB) from $url ..."
if (Get-Command curl.exe -ErrorAction SilentlyContinue) {
    & curl.exe --location --fail --silent --show-error --progress-bar --output $zip $url
    if ($LASTEXITCODE -ne 0) { throw "could not get $url" }
} else {
    Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing
}
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [IO.Compression.ZipFile]::OpenRead($zip)
try {
    $got = @()
    foreach ($entry in $archive.Entries) {
        $name = $entry.Name
        # The release build's files are in bin/x64 (bin/x64/development holds the logging ones).
        $release = $entry.FullName -match '^bin/x64/[^/]+$'
        $licence = $entry.FullName -match '^bin/x64/[^/]*license[^/]*$' -or $entry.FullName -eq 'license.txt'
        if (($release -and $want -contains $name) -or $licence) {
            $out = Join-Path $To $name
            [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $out, $true)
            $got += $name
        }
    }
} finally { $archive.Dispose() }
Remove-Item $zip -ErrorAction SilentlyContinue
$missing = $want | Where-Object { -not (Test-Path (Join-Path $To $_)) }
if ($missing) { throw "not in the SDK's zip: $($missing -join ', ')" }
Write-Host "Done: $($got -join ', ') in $To"
Write-Host 'Turn DLSS on in the game with F1 (DLSS line), or start with $env:VICTORY_DLSS = ''quality''.'
