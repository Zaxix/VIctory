# Defeat's launcher: VIctory-NVIDIA's (play-foxhound.ps1, whose options are described at its top) with the
# owner's PC in mind (Ryzen 7 5800X, RTX 4070 12 GB, a 1080p screen).
#
#   .\scripts\play-defeat.ps1 -Exe 'C:\path\to\game_scarlett_master.exe'
#
# Ray tracing stays off (VICTORY_BVH_BUILDER is not set). With NVIDIA's DLSS files (Get-NVIDIA-DLSS.cmd, once)
# the launcher's own default, DLSS Quality at 2560x1440, is used. Without them the scene is drawn at 1920x1080
# and the title upscales it (-Res 1080to1440), not drawn at 2560x1440. Any option given here wins, e.g.
# -Res 1440 or -Dlss off. In the game F1 opens the settings menu; what it saves also wins over these.
$given = @{}
for ($i = 0; $i -lt $args.Count; $i++) {
    if ("$($args[$i])" -match '^-(\w+)') { $given[$Matches[1].ToLower()] = $true }
}
$repo = Split-Path $PSScriptRoot -Parent
$sl = if ($env:VICTORY_STREAMLINE_DIR) { $env:VICTORY_STREAMLINE_DIR } else { Join-Path $repo 'nvidia-sdk\Streamline\bin\x64' }
$dlss = Test-Path (Join-Path $sl 'sl.interposer.dll')
$extra = @{}
if (-not $dlss -and -not $given.ContainsKey('res')) {
    Write-Host 'No DLSS files: the scene is drawn at 1920x1080 and upscaled by the title (-Res 1080to1440).'
    $extra.Res = '1080to1440'
}
& "$PSScriptRoot\play-foxhound.ps1" @args @extra
