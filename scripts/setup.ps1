# Scriba - ambiente de desenvolvimento local (idempotente).
# Execute em um PowerShell como administrador:  powershell -ExecutionPolicy Bypass -File scripts\setup.ps1
# Depois do build do FFmpeg, -LimparVcpkg apaga os temporarios do vcpkg (buildtrees, downloads, packages).
# -Sandbox so habilita o Windows Sandbox (cargo xtask sandbox-test) e sai; pede reiniciar o Windows.
# Sem ErrorActionPreference=Stop: no PowerShell 5.1 ele aborta com qualquer stderr de programa nativo.
param([switch]$LimparVcpkg, [switch]$Sandbox)

# Windows Sandbox (Windows 10/11 Pro, virtualizacao ligada no firmware). Idempotente.
if ($Sandbox) {
    # Sem elevacao o Get-WindowsOptionalFeature falha, e a falha parecia "sem Sandbox".
    $admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    if (-not $admin) { Write-Warning 'Rode num PowerShell como administrador (botao direito > Executar como administrador).'; exit 1 }
    $feature = Get-WindowsOptionalFeature -Online -FeatureName 'Containers-DisposableClientVM'
    if (-not $feature) { Write-Warning 'Este Windows nao tem o Windows Sandbox (precisa do Pro ou Enterprise).'; exit 1 }
    if ($feature.State -eq 'Enabled') { Write-Host 'ok   Windows Sandbox ja habilitado'; exit 0 }
    Write-Host 'inst Windows Sandbox'
    $result = Enable-WindowsOptionalFeature -Online -FeatureName 'Containers-DisposableClientVM' -All -NoRestart
    if ($result.RestartNeeded) { Write-Warning 'Reinicie o Windows para terminar de habilitar o Windows Sandbox.' }
    exit 0
}

# So o compilador x64 e o Windows 11 SDK (~3-4 GB), sem os componentes recomendados.
$vsComponents = '--add Microsoft.VisualStudio.Component.VC.Tools.x86.x64 --add Microsoft.VisualStudio.Component.Windows11SDK.26100'

function Install-Winget($id, $extra = @()) {
    winget list --id $id --exact --accept-source-agreements | Out-Null
    if ($LASTEXITCODE -eq 0) { Write-Host "ok   $id"; return }
    Write-Host "inst $id"
    winget install --id $id --exact --silent --accept-package-agreements --accept-source-agreements @extra
    if ($LASTEXITCODE -ne 0) { Write-Warning "$id falhou (codigo $LASTEXITCODE)" }
}

# Build Tools antes do Rustup: o rustup-init avisa se o MSVC ainda nao existe.
Install-Winget 'Microsoft.VisualStudio.2022.BuildTools' @('--override', "--quiet --wait --norestart $vsComponents")
Install-Winget 'Rustlang.Rustup'
Install-Winget 'LLVM.LLVM'
Install-Winget 'Kitware.CMake'
Install-Winget 'KhronosGroup.VulkanSDK'
# CLI do FFmpeg so para gerar fixtures (MP3/OGG/MP4). O app usa o FFmpeg estatico do vcpkg.
Install-Winget 'Gyan.FFmpeg.Essentials'

$rustup = "$env:USERPROFILE\.cargo\bin\rustup.exe"
if (Test-Path $rustup) { & $rustup default stable-x86_64-pc-windows-msvc }

# vcpkg fora do repo, em caminho curto (evita MAX_PATH no build do FFmpeg).
$vcpkg = 'C:\dev\vcpkg'
if (-not (Test-Path "$vcpkg\vcpkg.exe")) {
    New-Item -ItemType Directory -Force 'C:\dev' | Out-Null
    if (-not (Test-Path $vcpkg)) { git clone https://github.com/microsoft/vcpkg.git $vcpkg }
    & "$vcpkg\bootstrap-vcpkg.bat" -disableMetrics
}

[Environment]::SetEnvironmentVariable('VCPKG_ROOT', $vcpkg, 'User')
[Environment]::SetEnvironmentVariable('LIBCLANG_PATH', 'C:\Program Files\LLVM\bin', 'User')
# target/ em caminho curto: o build Vulkan do whisper.cpp estoura MAX_PATH (260) dentro do repo.
[Environment]::SetEnvironmentVariable('CARGO_TARGET_DIR', 'C:\dev\t\scriba', 'User')

# Build Tools ja listado mas sem o compilador C++: adiciona os componentes.
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
if ((Test-Path $vswhere) -and -not (& $vswhere -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath)) {
    $bt = & $vswhere -products * -property installationPath | Select-Object -First 1
    if ($bt) {
        Write-Host "inst compilador C++ em $bt"
        Start-Process -Wait "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\setup.exe" `
            -ArgumentList "modify --installPath `"$bt`" $vsComponents --quiet --norestart"
    }
}

# O cache de binarios do vcpkg (%LOCALAPPDATA%\vcpkg\archives) fica, entao nao ha recompilacao.
if ($LimparVcpkg) {
    foreach ($d in 'buildtrees', 'downloads', 'packages') {
        if (Test-Path "$vcpkg\$d") { Write-Host "limp $vcpkg\$d"; Remove-Item -Recurse -Force "$vcpkg\$d" }
    }
}

Write-Host "`nPronto. Abra um terminal novo para carregar o PATH e as variaveis."
