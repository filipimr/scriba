<#
.SYNOPSIS
  Smoke do exe de release, de ponta a ponta, pelas rotas: o CI (release.yml) e o Windows
  Sandbox (cargo xtask sandbox-test) o usam.

.DESCRIPTION
  Sobe o scriba.exe com uma pasta de dados temporária, envia o fala.mp4 pela rota, espera o card
  concluir, confere as palavras-chave no transcricao.txt e a linha de abertura no log, testa a
  segunda instância e sai pelo POST /api/app/sair (código 0, sem instance.json).

  Com -Sandbox, também: informações da máquina (build do Windows, RAM, vulkan-1.dll), o
  --self-test, o tempo da abertura até a UI responder e uma gravação de 8 s com o fala.wav
  tocando na saída (sem dispositivo de áudio, ela fica "pulada").

  O resultado vai para <Out>\resultado.json; o código de saída é 0 só se tudo passou.
  Roda no Windows PowerShell 5.1 (o do Sandbox) e no pwsh 7.
#>
param(
    [Parameter(Mandatory)] [string] $Exe,
    [Parameter(Mandatory)] [string] $Fixtures,
    [string] $Out = (Join-Path $env:TEMP "scriba-smoke"),
    [switch] $Sandbox
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
New-Item -ItemType Directory -Force $Out | Out-Null
$data = Join-Path $env:TEMP ("scriba-smoke-" + [guid]::NewGuid().ToString("N").Substring(0, 8))
$outputs = Join-Path $data "saidas"
$steps = New-Object System.Collections.ArrayList
$info = [ordered]@{}

function Step([string] $name, [scriptblock] $body) {
    $t = [Diagnostics.Stopwatch]::StartNew()
    $entry = [ordered]@{ nome = $name; ok = $false; segundos = 0; detalhe = "" }
    try {
        $detail = & $body
        $entry.ok = $true
        if ($detail -eq "pulada") { $entry.ok = $null }
        $entry.detalhe = [string]$detail
    } catch {
        $entry.detalhe = $_.Exception.Message
    }
    $entry.segundos = [math]::Round($t.Elapsed.TotalSeconds, 1)
    $mark = if ($entry.ok -eq $true) { "ok  " } elseif ($null -eq $entry.ok) { "pula" } else { "FALHA" }
    Write-Host ("{0} {1} ({2} s) {3}" -f $mark, $name, $entry.segundos, $entry.detalhe)
    [void]$steps.Add($entry)
    return $entry.ok
}

$script:base = $null
$script:headers = $null
$script:session = $null

function Api([string] $method, [string] $path, $body = $null, [string] $contentType = $null) {
    $params = @{ Uri = "$script:base$path"; Method = $method; Headers = $script:headers; WebSession = $script:session; UseBasicParsing = $true }
    if ($null -ne $body) { $params.Body = $body }
    if ($contentType) { $params.ContentType = $contentType }
    return Invoke-WebRequest @params
}

function Utf8([string] $s) { return [Text.Encoding]::UTF8.GetBytes($s) }

# Multipart à mão: o Windows PowerShell 5.1 não tem o -Form.
function Upload([string] $file) {
    $boundary = "scriba" + [guid]::NewGuid().ToString("N")
    $name = [IO.Path]::GetFileName($file)
    $head = "--$boundary`r`nContent-Disposition: form-data; name=`"arquivo`"; filename=`"$name`"`r`nContent-Type: application/octet-stream`r`n`r`n"
    $tail = "`r`n--$boundary--`r`n"
    $ms = New-Object IO.MemoryStream
    foreach ($part in @((Utf8 $head), [IO.File]::ReadAllBytes($file), (Utf8 $tail))) { $ms.Write($part, 0, $part.Length) }
    return Api "POST" "/api/jobs" $ms.ToArray() "multipart/form-data; boundary=$boundary"
}

function JobId([string] $html) {
    $m = [regex]::Match($html, 'id="job-([0-9A-Z]{26})"')
    if (-not $m.Success) { throw "sem card na resposta: $html" }
    return $m.Groups[1].Value
}

# Espera o polling do card terminar (286) e devolve o HTML final.
function WaitJob([string] $id, [int] $seconds) {
    $deadline = (Get-Date).AddSeconds($seconds)
    while ((Get-Date) -lt $deadline) {
        $r = Api "GET" "/api/jobs/$id"
        if ([int]$r.StatusCode -eq 286) { return $r.Content }
        Start-Sleep -Milliseconds 500
    }
    throw "o job $id não terminou em $seconds s"
}

# As palavras do fala.wav (com "." no lugar das letras acentuadas).
$keywords = @("bom dia", "scriba", "reuni.o", "or.amento", "amanh.", "nove horas|9 horas")
function Hits([string] $text) {
    $t = $text.ToLower()
    return @($keywords | Where-Object { $t -match $_ }).Count
}

function Transcript([string] $html) {
    if ($html -notmatch "Conclu") { throw "o job não concluiu: $html" }
    $folder = Get-ChildItem $outputs -Directory | Sort-Object LastWriteTime | Select-Object -Last 1
    $txt = Get-Content (Join-Path $folder.FullName "transcricao.txt") -Raw -Encoding UTF8
    return $txt
}

$proc = $null
try {
    if ($Sandbox) {
        Step "máquina" {
            $os = Get-CimInstance Win32_OperatingSystem
            $info.windows = "$($os.Caption) $($os.Version)"
            $info.ram_gb = [math]::Round($os.TotalVisibleMemorySize / 1MB, 1)
            $info.vulkan_dll = Test-Path (Join-Path $env:SystemRoot "System32\vulkan-1.dll")
            $info.gpu = (Get-CimInstance Win32_VideoController | ForEach-Object Name) -join "; "
            "$($info.windows), $($info.ram_gb) GB, vulkan-1.dll: $($info.vulkan_dll), vídeo: $($info.gpu)"
        } | Out-Null
        Step "--self-test" {
            $log = Join-Path $Out "self-test.txt"
            $p = Start-Process $Exe -ArgumentList "--self-test" -Wait -PassThru -NoNewWindow -RedirectStandardOutput $log -RedirectStandardError "$log.err"
            $text = (Get-Content $log -Raw -Encoding UTF8) + (Get-Content "$log.err" -Raw -Encoding UTF8)
            $info.self_test = $text
            if ($p.ExitCode -ne 0) { throw "código $($p.ExitCode): $text" }
            ($text -split "`n" | Select-String "backend:" | Select-Object -First 1).ToString().Trim()
        } | Out-Null
    }

    $opened = Step "abre e a UI responde" {
        $t = [Diagnostics.Stopwatch]::StartNew()
        $script:proc = Start-Process $Exe -ArgumentList "--no-open", "--port", "0", "--data-dir", "`"$data`"", "--output-dir", "`"$outputs`"" -PassThru
        $file = Join-Path $data "instance.json"
        $deadline = (Get-Date).AddSeconds(60)
        while (-not (Test-Path $file)) {
            if ($script:proc.HasExited) { throw "o exe saiu com $($script:proc.ExitCode)" }
            if ((Get-Date) -gt $deadline) { throw "sem instance.json em 60 s" }
            Start-Sleep -Milliseconds 200
        }
        $inst = Get-Content $file -Raw | ConvertFrom-Json
        $script:base = "http://127.0.0.1:$($inst.port)"
        # O Windows PowerShell 5.1 descarta um cabeçalho Cookie: o token vai na sessão.
        $script:session = New-Object Microsoft.PowerShell.Commands.WebRequestSession
        $script:session.Cookies.Add((New-Object Net.Cookie("scriba_token", $inst.token, "/", "127.0.0.1")))
        $script:headers = @{ Origin = $script:base }
        $page = Api "GET" "/"
        if ($page.Content -notmatch "Solte um v") { throw "página inicial inesperada" }
        $info.abertura_s = [math]::Round($t.Elapsed.TotalSeconds, 2)
        "porta $($inst.port), UI em $($info.abertura_s) s"
    }
    if (-not $opened) { throw "o app não abriu" }

    Step "envio transcrito" {
        $id = JobId (Upload (Join-Path $Fixtures "fala.mp4")).Content
        $txt = Transcript (WaitJob $id 600)
        $hits = Hits $txt
        if ($hits -lt 3) { throw "só $hits palavras-chave: $txt" }
        "$hits palavras-chave"
    } | Out-Null

    Step "log com a abertura" {
        $log = Get-Content (Join-Path $data "logs\scriba.log") -Raw -Encoding UTF8
        if ($log -notmatch "Scriba \d+\.\d+\.\d+ ") { throw "sem a linha de abertura" }
        if ($log -notmatch "Scriba em http") { throw "sem a URL" }
        ($log -split "`n" | Select-String "Scriba \d" | Select-Object -First 1).ToString().Trim()
    } | Out-Null

    if ($Sandbox) {
        Step "gravação de 8 s" {
            try {
                $start = Api "POST" "/api/record/start?painel=1"
            } catch {
                $code = [int]$_.Exception.Response.StatusCode
                if ($code -eq 422) { $info.gravacao = "sem dispositivo de áudio"; return "pulada" }
                throw
            }
            $player = New-Object Media.SoundPlayer (Join-Path $Fixtures "fala.wav")
            $player.Play()
            Start-Sleep -Seconds 8
            $player.Stop()
            $id = JobId (Api "POST" "/api/record/stop?painel=1").Content
            $txt = Transcript (WaitJob $id 600)
            $info.gravacao_texto = $txt
            "transcrita, $(Hits $txt) palavras-chave (o loopback do Sandbox pode não ter som)"
        } | Out-Null
    }

    Step "segunda instância" {
        $p = Start-Process $Exe -ArgumentList "--no-open", "--data-dir", "`"$data`"", "--output-dir", "`"$outputs`"" -Wait -PassThru
        if ($p.ExitCode -ne 0) { throw "saiu com $($p.ExitCode)" }
        if ($script:proc.HasExited) { throw "a primeira instância caiu" }
        "saiu com 0 e a primeira segue"
    } | Out-Null

    Step "saída pela rota" {
        $r = Api "POST" "/api/app/sair"
        if ([int]$r.StatusCode -ne 204) { throw "status $($r.StatusCode)" }
        if (-not $script:proc.WaitForExit(30000)) { throw "não saiu em 30 s" }
        if ($script:proc.ExitCode -ne 0) { throw "código $($script:proc.ExitCode)" }
        if (Test-Path (Join-Path $data "instance.json")) { throw "instance.json ficou" }
        "código 0"
    } | Out-Null
} catch {
    [void]$steps.Add([ordered]@{ nome = "smoke"; ok = $false; segundos = 0; detalhe = $_.Exception.Message })
    Write-Host "FALHA $($_.Exception.Message)"
} finally {
    if ($script:proc -and -not $script:proc.HasExited) { Stop-Process -Id $script:proc.Id -Force }
    $logs = Join-Path $data "logs"
    if (Test-Path $logs) { Copy-Item $logs (Join-Path $Out "logs") -Recurse -Force }
}

$failed = @($steps | Where-Object { $_.ok -eq $false }).Count
$result = [ordered]@{ ok = ($failed -eq 0); passos = $steps; info = $info }
$result | ConvertTo-Json -Depth 5 | Out-File -Encoding utf8 (Join-Path $Out "resultado.json")
Write-Host ("resultado: {0} ({1} falha(s)) em {2}" -f $(if ($failed) { "FALHOU" } else { "ok" }), $failed, (Join-Path $Out "resultado.json"))
exit [int]($failed -ne 0)
