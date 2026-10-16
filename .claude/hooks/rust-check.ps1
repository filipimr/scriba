# Stop: roda cargo fmt --check e cargo clippy -D warnings no workspace.
# Sai em silêncio enquanto não houver Cargo.toml ou cargo instalado.
$payload = [Console]::In.ReadToEnd() | ConvertFrom-Json
# Evita loop: se o Claude já está continuando por causa deste hook, não bloqueia de novo.
if ($payload.stop_hook_active) { exit 0 }

$root = $env:CLAUDE_PROJECT_DIR
if (-not $root) { $root = (Get-Location).Path }
if (-not (Test-Path (Join-Path $root 'Cargo.toml'))) { exit 0 }
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { exit 0 }

Set-Location $root
$problems = @()

$fmt = & cargo fmt --all -- --check 2>&1
if ($LASTEXITCODE -ne 0) { $problems += "cargo fmt --check:`n$($fmt -join "`n")" }

$clippy = & cargo clippy --all-targets --quiet -- -D warnings 2>&1
if ($LASTEXITCODE -ne 0) { $problems += "cargo clippy -D warnings:`n$($clippy -join "`n")" }

if ($problems.Count -gt 0) {
    [Console]::Error.WriteLine("Corrija antes de encerrar:`n`n" + ($problems -join "`n`n"))
    exit 2
}
exit 0
