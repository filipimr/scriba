# Scriba - ambiente de desenvolvimento local (idempotente).
# Execute em um PowerShell como administrador:  powershell -ExecutionPolicy Bypass -File scripts\setup.ps1
$ErrorActionPreference = 'Stop'

function Install-Winget($id, $extra = @()) {
    $listed = winget list --id $id --exact --accept-source-agreements 2>$null | Select-String $id
    if ($listed) { Write-Host "ok   $id"; return }
    Write-Host "inst $id"
    winget install --id $id --exact --silent --accept-package-agreements --accept-source-agreements @extra
}

Install-Winget 'Rustlang.Rustup'
Install-Winget 'Microsoft.VisualStudio.2022.BuildTools' @('--override', '--quiet --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended')
Install-Winget 'LLVM.LLVM'
Install-Winget 'Kitware.CMake'
Install-Winget 'KhronosGroup.VulkanSDK'

# vcpkg fora do repo, em caminho curto (evita MAX_PATH no build do FFmpeg).
$vcpkg = 'C:\dev\vcpkg'
if (-not (Test-Path "$vcpkg\vcpkg.exe")) {
    New-Item -ItemType Directory -Force 'C:\dev' | Out-Null
    if (-not (Test-Path $vcpkg)) { git clone https://github.com/microsoft/vcpkg.git $vcpkg }
    & "$vcpkg\bootstrap-vcpkg.bat" -disableMetrics
}

[Environment]::SetEnvironmentVariable('VCPKG_ROOT', $vcpkg, 'User')
[Environment]::SetEnvironmentVariable('LIBCLANG_PATH', 'C:\Program Files\LLVM\bin', 'User')

Write-Host "`nPronto. Abra um terminal novo para carregar o PATH e as variaveis."
