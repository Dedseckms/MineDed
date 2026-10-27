# Renders docs/changelog/<version>.png for every CHANGELOG.md section (or one: -Version 1.2.1).
param([string]$Version)
$ErrorActionPreference = 'Stop'
$root = Resolve-Path "$PSScriptRoot\..\.."
$chrome = @("$env:ProgramFiles\Google\Chrome\Application\chrome.exe", "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe") |
  Where-Object { Test-Path $_ } | Select-Object -First 1
$scenes = 'forest', 'plains', 'cherry', 'snowy', 'ocean', 'desert', 'cave', 'nether'
$sections = Select-String -Path "$root\CHANGELOG.md" -Encoding UTF8 -Pattern '^##\s+(\d+\.\d+\.\d+)\s*[\u2014-]\s*(.+)$' |
  ForEach-Object { [pscustomobject]@{ v = $_.Matches[0].Groups[1].Value; t = $_.Matches[0].Groups[2].Value.Trim() } }
$card = (Join-Path $PSScriptRoot 'card.html') -replace '\\', '/'
foreach ($s in $sections) {
  if ($Version -and $s.v -ne $Version) { continue }
  $i = [Math]::Abs($s.v.GetHashCode()) % $scenes.Count
  $hash = "v=$([uri]::EscapeDataString($s.v))&t=$([uri]::EscapeDataString($s.t))&s=$($scenes[$i])"
  $out = Join-Path $PSScriptRoot "$($s.v).png"
  Start-Process -Wait -FilePath $chrome -ArgumentList @('--headless=new', '--disable-gpu', '--hide-scrollbars',
    '--allow-file-access-from-files', '--force-color-profile=srgb', '--window-size=1280,640',
    '--virtual-time-budget=2000', "--screenshot=$out", "file:///$card#$hash")
  Write-Output "rendered $($s.v)"
}
