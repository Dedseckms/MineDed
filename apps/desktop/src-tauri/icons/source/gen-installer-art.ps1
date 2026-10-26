# SPDX-License-Identifier: GPL-3.0-or-later
# Renders installer-art.html in headless Chrome/Edge and converts the screenshots to the
# 24-bit BMPs used by tauri.conf.json (bundle.windows.nsis / bundle.windows.wix).
# Windows-only dev tool; the generated BMPs are committed.
#
#   powershell -File apps/desktop/src-tauri/icons/source/gen-installer-art.ps1

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$outDir = Join-Path (Split-Path -Parent $here) "installer"
New-Item -ItemType Directory -Force $outDir | Out-Null

$browser = @(
  "$env:ProgramFiles\Google\Chrome\Application\chrome.exe",
  "${env:ProgramFiles(x86)}\Google\Chrome\Application\chrome.exe",
  "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $browser) { throw "Chrome or Edge not found" }

$page = ([Uri](Join-Path $here "installer-art.html")).AbsoluteUri
$tmp = Join-Path ([IO.Path]::GetTempPath()) ("mineded-art-" + [Guid]::NewGuid())
New-Item -ItemType Directory $tmp | Out-Null

$kinds = @{
  "nsis-sidebar" = @("sidebar", 164, 314)
  "nsis-header"  = @("header", 150, 57)
  "wix-banner"   = @("banner", 493, 58)
  "wix-dialog"   = @("dialog", 493, 312)
}
try {
  foreach ($name in $kinds.Keys) {
    $k, $w, $h = $kinds[$name]
    $png = Join-Path $tmp "$name.png"
    $p = Start-Process -FilePath $browser -Wait -PassThru -WindowStyle Hidden -ArgumentList @(
      "--headless=new", "--disable-gpu", "--hide-scrollbars", "--allow-file-access-from-files",
      "--force-device-scale-factor=1", "--user-data-dir=`"$tmp\profile`"",
      "--window-size=$w,$h", "--virtual-time-budget=2000",
      "--screenshot=`"$png`"", "`"$page`?kind=$k`""
    )
    if (-not (Test-Path $png)) { throw "render failed for $name (exit $($p.ExitCode))" }
    $src = [Drawing.Bitmap]::FromFile($png)
    try {
      $bmp = New-Object Drawing.Bitmap $w, $h, ([Drawing.Imaging.PixelFormat]::Format24bppRgb)
      $g = [Drawing.Graphics]::FromImage($bmp)
      $g.DrawImage($src, (New-Object Drawing.Rectangle 0, 0, $w, $h),
        (New-Object Drawing.Rectangle 0, 0, $w, $h), [Drawing.GraphicsUnit]::Pixel)
      $g.Dispose()
      $bmp.Save((Join-Path $outDir "$name.bmp"), [Drawing.Imaging.ImageFormat]::Bmp)
      $bmp.Dispose()
    } finally { $src.Dispose() }
    Write-Host "$name.bmp ${w}x${h}"
  }
} finally {
  Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
