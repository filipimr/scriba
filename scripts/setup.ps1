# Scriba - ambiente de desenvolvimento local (idempotente).
# Execute em um PowerShell como administrador:  powershell -ExecutionPolicy Bypass -File scripts\setup.ps1
# Depois do build do FFmpeg, -LimparVcpkg apaga os temporarios do vcpkg (buildtrees, downloads, packages).
# Sem ErrorActionPreference=Stop: no PowerShell 5.1 ele aborta com qualquer stderr de programa nativo.
param([switch]$LimparVcpkg)

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
