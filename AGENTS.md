# Repository Guidelines

## Project Scope & Goals
Interlude encourages sustainable focus through recurring, short wellness breaks. The MVP targets Linux Wayland (Hyprland and other wlroots-based compositors). The app provides a fullscreen dim overlay that nudges the user into a break; it does not own session locks or authentication.

## Project Structure & Module Organization
- `src/main.rs` is the binary entry point.
- `src/cli.rs` holds CLI parsing and user-facing options (MVP is CLI-only).
- `src/scheduler.rs` implements the timing state machine (cadence, snooze decay, phase transitions).
- `src/wayland_lock.rs` contains the wlr-layer-shell overlay backend.
- `src/shm.rs` and `src/tiny_font.rs` provide Wayland buffers and text rendering helpers.
- Build artifacts live in `target/` and should not be edited manually.

## Behavior & Defaults
- Default cadence: 30-minute work blocks, 60-second breaks, 5-minute base snooze.
- Overlay interaction: `Enter` begins a break or dismisses after completion; `z` snoozes when due.
- Snooze shrinks with each use, has a minimum floor, and resets after a completed break.
- After a break, the user dismisses the overlay and the work timer restarts immediately.

## Architecture & Platform Notes
- Locking philosophy: Interlude never handles passwords or unlocks; existing lockers remain authoritative.
- Enforcement model: soft enforcement via a fullscreen dim overlay, focus, visibility, and compositor hints.
- Wayland backend: wlr-layer-shell for always-on-top, focused overlays on all outputs.
- Optional Hyprland additions: temporary submap or special workspace while the overlay is active.
- Future-proofing: scheduler is platform-agnostic; overlay backends may swap per platform.

## Build, Test, and Development Commands
- `cargo build` builds the project in debug mode.
- `cargo run -- <args>` runs the binary with CLI arguments.
- `cargo build --release` produces an optimized binary in `target/release`.
- `cargo check` should be run after code changes to validate compilation quickly.
- `cargo fmt` formats Rust sources using rustfmt.
- `cargo clippy -- -D warnings` runs lint checks and treats warnings as errors.

## Coding Style & Naming Conventions
- Use Rust 2024 edition conventions (see `Cargo.toml`).
- Prefer 4-space indentation (rustfmt default).
- Use `snake_case` for modules/functions and `CamelCase` for types.
- Keep module responsibilities narrow; prefer small, focused helpers.

## Testing Guidelines
- No test framework is configured yet; use standard Rust tests (`#[test]`) when adding coverage.
- Place unit tests alongside modules (e.g., `src/scheduler.rs`) or in `tests/` for integration tests.
- Name tests with clear intent, e.g., `test_scheduler_ticks_once`.

## Commit & Pull Request Guidelines
- Commit message conventions are not established (only an initial commit exists). Use concise, present-tense summaries, e.g., `Add layer-shell overlay stub`.
- PRs should describe the change, list key files touched, and include reproduction steps for behavior changes.

## Environment & Tooling
- `devenv.yaml` exists for Nix-based setups; update it if you add external toolchain dependencies.
- Audio playback is in-process (rodio) with embedded Opus assets; build environments need ALSA and Opus development headers (and `pkg-config`).
- `.cargo/config.toml` sets a CMake policy env var to allow bundled Opus builds when system libs are missing.
