# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Current state

Phases 0 (foundation and risk spikes), 1 (CI/CD), 2 (app skeleton and design system) and 3 (job queue and upload) are done. Phase 4 (file transcription) is in progress in six stages with their own PRs (4.1–4.6, listed in `ROADMAP.md`). The product flow is automatic: receive or record → extract audio → transcribe to `.txt` and `.md` → compress when worthwhile, with no options to pick; compression is now Phase 5 and recording Phase 6. The repo is `filipimr/scriba` (public) with `main` protected (checks `checks` and `deny` required); `README.md` (pt-BR) presents it on GitHub. The `scriba` binary serves the UI:

- `src/app.rs`: single instance and the server + worker lifecycle.
- `src/web/`: Axum router, `security.rs` (token/Host/Origin guard), embedded `assets.rs`, askama `pages.rs`, and `jobs.rs` (streaming multipart upload to `<data-dir>/uploads`, the `/api/jobs` routes answering HTML fragments; 286 stops the 1 s polling of a card).
- `src/jobs/`: the in-memory queue. `model.rs` holds the pure state machine (queued, running, done, failed, canceled); `id.rs` a hand-rolled ULID; `store.rs` the `Jobs` handle; `worker.rs` the dedicated thread (one job at a time, `catch_unwind`, cooperative cancel via `AtomicBool`); `pipeline.rs` the `Pipeline` trait.
- `fake.rs`: `FakePipeline` is the app's pipeline until Phase 4. File names steer it: `lento` (10× slower), `falha` (friendly failure) and `panico` (panic), handy for manual and E2E tests.
- `src/config.rs` (paths, `config.json`), `src/instance.rs` (`instance.json`) and `--self-test` in `src/self_test.rs`.

Templates live in `templates/`, CSS source in `ui/`, embedded files in `assets/`. Other crates: `crates/scriba-model` (model embedded via `include_bytes!` behind the `embedded-model` feature), `crates/spikes` (the Phase 0 spikes as tests), and `xtask`. No transcription or recording yet. Document new commands here as each phase adds them.

## Environment and commands

- One-time setup, in an admin PowerShell: `powershell -ExecutionPolicy Bypass -File scripts\setup.ps1`. It is idempotent; `-LimparVcpkg` deletes vcpkg temp files. It installs the toolchain and sets the user variables `VCPKG_ROOT=C:\dev\vcpkg`, `LIBCLANG_PATH`, and `CARGO_TARGET_DIR=C:\dev\t\scriba`. The short target dir is required: inside the repo, the whisper.cpp Vulkan build exceeds MAX_PATH.
- Static FFmpeg (once, then cached): `C:\dev\vcpkg\vcpkg.exe install --triplet x64-windows-static-release --x-install-root=vcpkg_installed`. The triplet is the release-only overlay in `triplets/` (declared in `vcpkg.json`); debug libs were never linked. In manifest mode, installing another triplet replaces `vcpkg_installed/`, so an environment from before Phase 3.5 reinstalls FFmpeg once with this command.
- `.cargo/config.toml` holds the build env: triplet `x64-windows-static-release`, `+crt-static`, `FFMPEG_DIR`, the whisper.cpp `GGML_*` SIMD baseline, `CMAKE_MSVC_RUNTIME_LIBRARY`, and `CMAKE_{C,CXX}_FLAGS_RELEASE`. Without those last two, the release build of whisper.cpp comes out unoptimized (`/Od`): cmake-rs strips `/O*` and overwrites the Release flags under the Visual Studio generator. Changing `GGML_*`/`CMAKE_*` needs `cargo clean -p whisper-rs-sys` (plus `--release` for the release profile), because the build script does not track them; `release.yml` does this clean before building.
- `cargo xtask model` downloads the models to `models/` and checks SHA-256; `--only` takes comma-separated name fragments (`--only base` is what CI uses; `--only small` fetches the CPU-model candidate). `cargo xtask fixtures` regenerates `tests/fixtures/`; it needs the "Microsoft Maria" SAPI voice and the `ffmpeg` CLI.
- Lint: `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --features spikes/vulkan -- -D warnings`.
- Tests: `cargo test --workspace --features spikes/vulkan`. Hardware tests (GPU, NVENC, real audio devices, large model) are `#[ignore = "hardware"]`; run them locally with `cargo test -p spikes --features vulkan -- --ignored`. Measure whisper speed in `--release`, on AC power and in best-performance mode (on battery the CPU numbers drop sharply).
- Transcription benchmark (hardware): `cargo test -p spikes --release --features vulkan --test bench -- --ignored --nocapture`, steered by `SCRIBA_BENCH_MODELS` (`base,small,large`), `SCRIBA_BENCH_DEVICES` (`cpu` and Vulkan indices), `SCRIBA_BENCH_FLASH` (`0,1`), `SCRIBA_BENCH_THREADS`, `SCRIBA_BENCH_AUDIO` (16 kHz mono WAV) and `SCRIBA_BENCH_REPEAT`; it prints a Markdown table.
- `--self-test` also prints `whisper_print_system_info()`, and fails if transcription exceeds `SCRIBA_SELF_TEST_MAX_SECS` (set only by `release.yml`, as a guard against an unoptimized whisper.cpp).
- Upload memory: `tests/upload.rs` streams `SCRIBA_UPLOAD_MB` megabytes (default 256, what CI runs) through the real exe and checks its peak working set. The Phase 3 criterion is `SCRIBA_UPLOAD_MB=2048 cargo test --test upload -- --nocapture`.
- HTML snapshots (`insta`) of the job card live in `src/web/snapshots/`. After an intended markup change, accept them with `INSTA_UPDATE=always cargo test --lib web::jobs` and review the diff.
- The FFmpeg cache key is the triplet plus the hash of `vcpkg.json` and `triplets/`. Changing either costs one cold FFmpeg build on the branch and another on `main` (`main` does not read caches saved by PRs). CI also runs weekly on `main` (`schedule`, or `gh workflow run ci.yml`) so the caches never sit 7 days unused, when GitHub evicts them.
- CI runs `cargo llvm-cov nextest --workspace --profile ci` (nextest config in `.config/nextest.toml` serializes whisper tests) and `cargo deny check` (`deny.toml`). CI has no Vulkan, so clippy there runs without `spikes/vulkan`.
- `scriba --self-test` transcribes a sample on CPU and encodes a short x264 clip. Release builds use `cargo build --release --features embedded-test-model` (embeds `ggml-base-q5_1.bin` + `tests/fixtures/fala.wav`); without the feature it reads from `models/`.
- Releases: push a tag `vX.Y.Z` matching the `scriba` version in `Cargo.toml`; `.github/workflows/release.yml` builds, smoke-tests, and publishes exe + SHA-256 + `THIRD_PARTY_LICENSES.html` (`cargo about`).
- Run the app: `cargo run` opens the browser on `http://127.0.0.1:47800/?t=<token>` (the token becomes an `HttpOnly; SameSite=Strict` cookie). Flags: `--no-open`, `--port` (falls back to a free port if busy; `0` = any), `--data-dir` (default `%LOCALAPPDATA%\Scriba`), `--output-dir` (default `Documentos\Scriba`). A second run finds the live instance via `<data-dir>/instance.json` + `/health`, prints its URL and exits. Ctrl+C shuts down cleanly.
- CSS: `cargo xtask css` regenerates `ui/tokens.css` from the `DESIGN-scriba.md` YAML (fails if any component text/background pair is below WCAG AA) and compiles `assets/app.css` with the pinned Tailwind CLI (downloaded to `$CARGO_TARGET_DIR/tools`). Both outputs are committed; run it after changing tokens, `ui/app.css` or template classes. CI runs `cargo xtask css --check`.
- E2E: `cargo build && cd e2e && npm ci && npx playwright test`. The `scriba` fixture in `e2e/fixtures.ts` starts a fresh debug `scriba.exe` per test (from `CARGO_TARGET_DIR`, or `SCRIBA_EXE`; port 0, temp data dir), driven in the installed Edge (`channel: "msedge"`). Visual comparisons run only in CI; references live in `e2e/tests/*-snapshots/` and are regenerated by the manual `e2e-snapshots.yml` workflow (`gh workflow run e2e-snapshots.yml --ref <branch>`, then download the artifact and commit it).
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
