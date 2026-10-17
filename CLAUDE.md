# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Current state

Phase 0 (foundation and risk spikes) is done; Phase 1 (CI/CD) is in progress on GitHub at `filipimr/scriba` (public). The workspace has a placeholder `scriba` binary (`src/main.rs`, only `--self-test` in `src/self_test.rs`), `crates/scriba-model` (model embedded via `include_bytes!` behind the `embedded-model` feature), `crates/spikes` (the Phase 0 spikes as tests), and `xtask`. No app features exist yet. Document new commands here as each phase adds them.

## Environment and commands

- One-time setup, in an admin PowerShell: `powershell -ExecutionPolicy Bypass -File scripts\setup.ps1`. It is idempotent; `-LimparVcpkg` deletes vcpkg temp files. It installs the toolchain and sets the user variables `VCPKG_ROOT=C:\dev\vcpkg`, `LIBCLANG_PATH`, and `CARGO_TARGET_DIR=C:\dev\t\scriba`. The short target dir is required: inside the repo, the whisper.cpp Vulkan build exceeds MAX_PATH.
- Static FFmpeg (once, ~12–20 min, then cached): `C:\dev\vcpkg\vcpkg.exe install --triplet x64-windows-static --x-install-root=vcpkg_installed`. In manifest mode, installing another triplet replaces `vcpkg_installed/`.
- `.cargo/config.toml` holds the build env: triplet `x64-windows-static`, `+crt-static`, `FFMPEG_DIR`, the whisper.cpp `GGML_*` SIMD baseline, and `CMAKE_MSVC_RUNTIME_LIBRARY`. Changing `GGML_*`/`CMAKE_*` needs `cargo clean -p whisper-rs-sys`, because the build script does not track them.
- `cargo xtask model` downloads the models to `models/` and checks SHA-256; `--only base` downloads just the small test model (what CI uses). `cargo xtask fixtures` regenerates `tests/fixtures/`; it needs the "Microsoft Maria" SAPI voice and the `ffmpeg` CLI.
- Lint: `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --features spikes/vulkan -- -D warnings`.
- Tests: `cargo test --workspace --features spikes/vulkan`. Hardware tests (GPU, NVENC, real audio devices, large model) are `#[ignore = "hardware"]`; run them locally with `cargo test -p spikes --features vulkan -- --ignored`. Measure whisper speed only in `--release`.
- CI runs `cargo llvm-cov nextest --workspace --profile ci` (nextest config in `.config/nextest.toml` serializes whisper tests) and `cargo deny check` (`deny.toml`). CI has no Vulkan, so clippy there runs without `spikes/vulkan`.
- `scriba --self-test` transcribes a sample on CPU and encodes a short x264 clip. Release builds use `cargo build --release --features embedded-test-model` (embeds `ggml-base-q5_1.bin` + `tests/fixtures/fala.wav`); without the feature it reads from `models/`.
- Releases: push a tag `vX.Y.Z` matching the `scriba` version in `Cargo.toml`; `.github/workflows/release.yml` builds, smoke-tests, and publishes exe + SHA-256 + `THIRD_PARTY_LICENSES.html` (`cargo about`).
- E2E: `cd e2e && npm ci && npx playwright test` (placeholder until Phase 2).
- Spike D binary: `cargo build --release -p spikes --features "vulkan embedded-model" --bin exe_unico`.

- `ROADMAP.md` (pt-BR) is the project plan. It covers:
  - product decisions;
  - the target architecture: a Rust monolith shipped as a single Windows `.exe`, with an Axum + HTMX UI embedded in the binary, whisper-rs for transcription, ffmpeg-next for media, and cpal for microphone + system-audio loopback capture;
  - phases 0–7 with checklists;
  - the testing policy, risks, and a decision log.
- `DESIGN-scriba.md` is the visual design system the UI must follow (described below).

## How work proceeds

- One phase at a time. Start a phase only when the user asks, with its own detailed plan approved before execution, and do not roll into the next phase when it finishes.
- When a phase starts, set its row in the `ROADMAP.md` status table to "Em andamento". When it finishes, tick its checklists, set it to "Concluída", and record decisions and measured numbers in "Registro de decisões".
- A feature is done only when it has automated tests at the right level (see "Qualidade e testes" in `ROADMAP.md`) and CI is green. Tests that need real audio devices or a GPU belong in the local hardware suite, not in CI.
- If a spike or test invalidates a decision, update `ROADMAP.md` before continuing.

## Design system (`DESIGN-scriba.md`)

`DESIGN-scriba.md` is a DESIGN.md-style document in two parts:

- **YAML front matter** holds the design tokens, with English keys: `colors`, `typography`, `rounded`, `spacing`, `motion`, `elevation`, and `components`. Components reference tokens with `{group.token}` syntax, e.g. `backgroundColor: "{colors.primary}"`, and variants are separate keys (`button-primary-active`, `record-button-recording`). Phase 2 turns these tokens into CSS via `cargo xtask css`.
- **Markdown body** (pt-BR) explains the system: brand and logo, iconography, colors, typography, layout, elevation, shapes, motion, components, accessibility, microcopy, do's and don'ts, iteration guide, and known gaps.

It is the Scriba app's own design system, inspired by Apple's visual language: one blue accent for actions, pills, Inter with negative tracking, and a monochrome brand. The source logo is `docs/brand/logo-original.jpg`.

## Editing `DESIGN-scriba.md`

- Keep the front matter and the prose in sync. A token or component changed in YAML must be reflected in the body sections that describe it, and vice versa.
- Reference tokens with `{...}` rather than raw hex values in components. The body repeats hex values for readability; keep those matching the YAML.
- Follow the rules the spec sets:
  - `colors.primary` is the only action color; status colors mark state, never actions (sole exception: `button-danger` inside `confirm-dialog`).
  - Weight ladder 400/600.
  - Body text at 17px.
  - No gradients; shadows only on `toast` and `confirm-dialog`.
  - All text meets WCAG AA contrast in both themes.
  - Light and dark themes: `colors:` is the light theme, `themes.dark` overrides only differing colors. Every new color needs a dark value. Blue text uses `colors.link`; `colors.primary` is only for fills.
- Document default, pressed, focus, disabled, and error states, not hover.
- Preserve the "Lacunas conhecidas" section and update it when a gap is filled rather than deleting it silently.
