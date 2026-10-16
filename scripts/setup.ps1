# Scriba - ambiente de desenvolvimento local (idempotente).
# Execute em um PowerShell como administrador:  powershell -ExecutionPolicy Bypass -File scripts\setup.ps1
# Sem ErrorActionPreference=Stop: no PowerShell 5.1 ele aborta com qualquer stderr de programa nativo.

function Install-Winget($id, $extra = @()) {
    winget list --id $id --exact --accept-source-agreements | Out-Null
    if ($LASTEXITCODE -eq 0) { Write-Host "ok   $id"; return }
    Write-Host "inst $id"
    winget install --id $id --exact --silent --accept-package-agreements --accept-source-agreements @extra
    if ($LASTEXITCODE -ne 0) { Write-Warning "$id falhou (codigo $LASTEXITCODE)" }
}

# Build Tools antes do Rustup: o rustup-init avisa se o MSVC ainda nao existe.
Install-Winget 'Microsoft.VisualStudio.2022.BuildTools' @('--override', '--quiet --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended')
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

# Build Tools ja listado mas sem o workload C++: adiciona o workload.
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
if ((Test-Path $vswhere) -and -not (& $vswhere -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath)) {
    $bt = & $vswhere -products * -property installationPath | Select-Object -First 1
    if ($bt) {
        Write-Host "inst workload VCTools em $bt"
        Start-Process -Wait "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\setup.exe" `
            -ArgumentList "modify --installPath `"$bt`" --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended --quiet --norestart"
    }
}

Write-Host "`nPronto. Abra um terminal novo para carregar o PATH e as variaveis."
