# Deploy mineup-relay (hub + iroh-relay) to /opt/relay. Run from the repo root on the
# dev machine (Windows, rustup + OpenSSH). Prompts for the SSH and sudo passwords
# interactively; nothing secret is stored by this script.
#   powershell -File server/relay/deploy.ps1 [-SshHost dedseckms.ru] [-Port 19999] [-User dedseckms] [-SkipBuild] [-NoStart]
param(
    [string]$SshHost = "dedseckms.ru",
    [int]$Port = 19999,
    [string]$User = "dedseckms",
    [switch]$SkipBuild,
    [switch]$NoStart
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$src = Join-Path $root "server\relay"
$target = "x86_64-unknown-linux-musl"
$bin = Join-Path $root "target\$target\release\hub"
$dest = "$User@$SshHost"

if (-not $SkipBuild) {
    rustup target add $target
    # Static musl binary linked by the bundled rust-lld: no C toolchain needed.
    $env:CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER = "rust-lld"
    cargo build --release -p hub --bin hub --target $target --manifest-path (Join-Path $root "Cargo.toml")
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
}
if (-not (Test-Path $bin)) { throw "missing $bin" }

# Stage files (LF endings for shell scripts).
$stage = Join-Path ([IO.Path]::GetTempPath()) ("mineup-relay-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force "$stage\hub", "$stage\iroh-relay", "$stage\caddy" | Out-Null
foreach ($f in "compose.yaml", "compose.qad.yaml", "start.sh", "check.sh", "install.sh", "DEPLOY.md", ".env.example",
    "hub\Dockerfile", "iroh-relay\relay.toml", "iroh-relay\relay-qad.toml", "caddy\relay.caddy") {
    $text = [IO.File]::ReadAllText((Join-Path $src $f)) -replace "`r`n", "`n"
    [IO.File]::WriteAllText((Join-Path $stage $f), $text, (New-Object Text.UTF8Encoding $false))
}
Copy-Item $bin "$stage\hub\hub"

$ts = (Get-Date).ToUniversalTime().ToString("yyyyMMdd-HHmmss")
$tarName = "upload-$ts.tar"
$tarPath = Join-Path ([IO.Path]::GetTempPath()) $tarName
tar -cf $tarPath -C $stage .
if ($LASTEXITCODE -ne 0) { throw "tar failed" }
# Upload archive + installer, then unpack (installer backs up the previous files).
scp -P $Port $tarPath (Join-Path $stage "install.sh") "${dest}:/opt/relay/"
if ($LASTEXITCODE -ne 0) { throw "upload failed" }
ssh -p $Port $dest "bash /opt/relay/install.sh /opt/relay/$tarName $ts"
if ($LASTEXITCODE -ne 0) { throw "install failed" }
Remove-Item -Recurse -Force $stage, $tarPath

if (-not $NoStart) {
    ssh -t -p $Port $dest "cd /opt/relay && ./start.sh && sleep 15 && ./check.sh"
}
