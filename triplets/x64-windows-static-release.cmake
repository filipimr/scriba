# Cópia do triplet comunitário do vcpkg, versionada para não depender do master
# que o CI clona. É o x64-windows-static só com release: o Rust linka sempre o
# CRT de release e o ffmpeg-sys-next lê só <FFMPEG_DIR>/lib, então a versão
# debug (~40% do build do FFmpeg) nunca era usada.
set(VCPKG_TARGET_ARCHITECTURE x64)
set(VCPKG_CRT_LINKAGE static)
set(VCPKG_LIBRARY_LINKAGE static)

set(VCPKG_BUILD_TYPE release)
