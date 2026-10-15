# Process a whole game in sections (e.g. quarters) and optionally overlay the minimap on the broadcast video.
# Windows PowerShell counterpart of run_game.sh.
#
#   .\scripts\run_game.ps1 -Panorama pano.mp4 -Segments segmente.txt -Out out `
#       -Model C:\models\my-model -Court court.json -Teams teams.json [-Broadcast game.mp4] [-Ep cuda]
#
# segmente.txt: one line per section:  <name> <start-seconds> <duration-seconds>
#     q1 305 600
#     q2 1210 600
param(
  [Parameter(Mandatory)] [string]$Panorama,
  [Parameter(Mandatory)] [string]$Segments,
  [Parameter(Mandatory)] [string]$Out,
  [Parameter(Mandatory)] [string]$Model,
  [Parameter(Mandatory)] [string]$Court,
  [Parameter(Mandatory)] [string]$Teams,
  [string]$Broadcast = "",
  [double]$Sync = 0,
  [int]$OverlayW = 480,
  [string]$OverlayPos = "W-w-30:H-h-30",
  [string]$Bg = "",
  [int]$Every = 5,
  [string]$Tile = "auto",
  [string]$Scales = "",
  [string]$Ep = "auto",
  [string[]]$Extra = @()
)
$ErrorActionPreference = "Stop"
if ($Bg -eq "") { $Bg = if ($Broadcast) { "0.6" } else { "0" } }
New-Item -ItemType Directory -Force -Path $Out | Out-Null
foreach ($line in Get-Content $Segments) {
  $t = $line.Trim()
  if ($t -eq "" -or $t.StartsWith("#")) { continue }
  $name, $start, $dur = $t -split "\s+"
  Write-Host "=== ${name}: from ${start}s, ${dur}s ==="
  $a = @("run", $Panorama, "--model", $Model, "--court", $Court, "--teams", $Teams, "--tile", $Tile, "--ep", $Ep,
         "--start", $start, "--duration", $dur, "--every", $Every,
         "--alpha", "$Out\$name.mov", "--alpha-bg", $Bg, "--json", "$Out\$name.jsonl")
  if ($Scales) { $a += @("--scales", $Scales) }
  $a += $Extra
  & reco-minimap @a
  if ($LASTEXITCODE -ne 0) { throw "reco-minimap failed ($LASTEXITCODE)" }
  if ($Broadcast) {
    $bstart = [math]::Max(0, [double]$start + $Sync)
    & ffmpeg -nostdin -y -v error -ss $bstart -t $dur -i $Broadcast -i "$Out\$name.mov" `
      -filter_complex "[1:v]scale=${OverlayW}:-1[m];[0:v][m]overlay=${OverlayPos}:format=auto" `
      -c:v libx264 -crf 18 -preset medium -pix_fmt yuv420p -c:a copy "$Out\${name}_mit_minimap.mp4"
    Write-Host "  -> $Out\${name}_mit_minimap.mp4"
  }
}
