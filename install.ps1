# Install slashRepo on Windows: get repo-status.exe, then let it set itself up
# (%LOCALAPPDATA%\Programs\slashRepo, a Start Menu entry, /repo for every coding agent found:
# Claude Code, opencode, Codex, Antigravity, Gemini CLI). Safe to run again.
#
# With Rust (cargo) it builds from this checkout. Without cargo it downloads the prebuilt
# repo-status-x86_64-windows.exe of this version from the GitHub release and checks it against the
# release's SHA256SUMS before running it.
#
#   powershell -ExecutionPolicy Bypass -File install.ps1
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$manifest = Join-Path $here 'Cargo.toml'
$version = (Select-String -Path $manifest -Pattern '^version = "(.*)"' | Select-Object -First 1).Matches[0].Groups[1].Value

$tmp = $null
if (Get-Command cargo -ErrorAction SilentlyContinue) {
    cargo build --release --locked --manifest-path $manifest
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
    $target = (cargo metadata --format-version 1 --no-deps --manifest-path $manifest | ConvertFrom-Json).target_directory
    $exe = Join-Path $target 'release\repo-status.exe'
} else {
    if (-not [Environment]::Is64BitOperatingSystem) { throw 'no prebuilt exe for 32-bit Windows: install Rust (rustup.rs) and run again' }
    $url = "https://github.com/LudaLunizz/slashRepo/releases/download/v$version"
    $tmp = Join-Path ([IO.Path]::GetTempPath()) ('slashrepo-' + [guid]::NewGuid())
    New-Item -ItemType Directory $tmp | Out-Null
    $exe = Join-Path $tmp 'repo-status.exe'
    Invoke-WebRequest -UseBasicParsing "$url/repo-status-x86_64-windows.exe" -OutFile $exe
    Invoke-WebRequest -UseBasicParsing "$url/SHA256SUMS" -OutFile (Join-Path $tmp 'SHA256SUMS')
    $line = Get-Content (Join-Path $tmp 'SHA256SUMS') | Where-Object { $_ -match '\s\*?repo-status-x86_64-windows\.exe$' }
    $want = ("$line" -split '\s+')[0]
    $got = (Get-FileHash $exe -Algorithm SHA256).Hash
    if (-not $want -or $want -ne $got) { Remove-Item -Recurse -Force $tmp; throw 'checksum mismatch: not installing' }
}
try {
    # repo-status.exe is a windowed program: the pipe makes PowerShell wait for it and show its output
    & $exe install @args | Out-Host
    if ($LASTEXITCODE -ne 0) { throw 'repo-status install failed' }
} finally {
    if ($tmp) { Remove-Item -Recurse -Force $tmp }
}
