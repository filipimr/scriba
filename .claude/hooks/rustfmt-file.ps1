# PostToolUse (Edit|Write): formata o arquivo .rs editado com rustfmt.
# Sai em silêncio se o arquivo não for .rs ou se o rustfmt não estiver instalado.
$payload = [Console]::In.ReadToEnd() | ConvertFrom-Json
$file = $payload.tool_input.file_path
if (-not $file -or $file -notmatch '\.rs$') { exit 0 }
if (-not (Get-Command rustfmt -ErrorAction SilentlyContinue)) { exit 0 }
if (-not (Test-Path $file)) { exit 0 }

$out = & rustfmt --edition 2021 $file 2>&1
if ($LASTEXITCODE -ne 0) {
    # Erro de sintaxe: devolve ao Claude para corrigir.
    [Console]::Error.WriteLine("rustfmt falhou em ${file}:`n$out")
    exit 2
}
exit 0
