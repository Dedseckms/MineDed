# Downloads the pinned mihomo release for bundling as a Tauri sidecar
# (`externalBin: ["binaries/mihomo"]`) and verifies its SHA-256.
# Optional: by default the launcher downloads the same pinned build on first
# use (crates/lc-vpn/src/binary.rs). Keep version and digests in sync with it.
#
# Usage: powershell -File fetch-mihomo.ps1 [-Target x86_64-pc-windows-msvc]
param(
    [string]$Target = "x86_64-pc-windows-msvc"
)
$ErrorActionPreference = "Stop"
$Version = "v1.19.33"
$Assets = @{
    "x86_64-pc-windows-msvc"     = @("mihomo-windows-amd64-compatible-$Version.zip", "7aaf636a4f2172dcd8ec34ac4f52797b059195e5afd6409508c963e545f8864d")
    "aarch64-pc-windows-msvc"    = @("mihomo-windows-arm64-$Version.zip", "a2a26869a14634c49f3fbcecc593b498ed8591fbdd7616b24475b3d60b148da3")
    "x86_64-unknown-linux-gnu"   = @("mihomo-linux-amd64-compatible-$Version.gz", "386508bfb1c72c43b6a0eda73708ecdce82546581b0ebcb16366c53757c622b6")
    "aarch64-unknown-linux-gnu"  = @("mihomo-linux-arm64-$Version.gz", "ba408b317080ee265c46cb923a50018c1b82cbd708551cdd46a1e3e307396187")
}
if (-not $Assets.ContainsKey($Target)) { throw "unsupported target $Target" }
$Name, $Sha = $Assets[$Target]
$Out = Join-Path $PSScriptRoot $Target
$Tmp = Join-Path ([IO.Path]::GetTempPath()) ("mihomo-" + [guid]::NewGuid())
New-Item -ItemType Directory $Tmp | Out-Null
try {
    $Archive = Join-Path $Tmp $Name
    Invoke-WebRequest -UseBasicParsing -Uri "https://github.com/MetaCubeX/mihomo/releases/download/$Version/$Name" -OutFile $Archive
    $Actual = (Get-FileHash -Algorithm SHA256 $Archive).Hash.ToLower()
    if ($Actual -ne $Sha) { throw "SHA-256 mismatch for ${Name}: $Actual" }
    if ($Name.EndsWith(".zip")) {
        Expand-Archive $Archive -DestinationPath (Join-Path $Tmp "x")
        $Exe = Get-ChildItem (Join-Path $Tmp "x") -Recurse -Filter *.exe | Select-Object -First 1
        Copy-Item $Exe.FullName (Join-Path $PSScriptRoot "mihomo-$Target.exe") -Force
    } else {
        $In = [IO.File]::OpenRead($Archive)
        $Gz = New-Object IO.Compression.GZipStream($In, [IO.Compression.CompressionMode]::Decompress)
        $Dst = [IO.File]::Create((Join-Path $PSScriptRoot "mihomo-$Target"))
        $Gz.CopyTo($Dst); $Dst.Close(); $Gz.Close(); $In.Close()
    }
    Write-Output "mihomo $Version for $Target ready in $PSScriptRoot"
} finally {
    Remove-Item -Recurse -Force $Tmp
}
