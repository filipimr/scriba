# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Current state

Planning stage. There is no source code, package manifest, build step, linter, or test suite yet, so no build, lint, or test commands exist. Do not invent them; document the real commands here as each phase adds them.

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
