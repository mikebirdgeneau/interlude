# Interlude

Interlude encourages sustainable focus through recurring, short wellness breaks. It targets Linux Wayland compositors (Hyprland and other wlroots-based setups) and presents a fullscreen dim overlay that gently nudges the user to pause without taking over session locking.

## Purpose and Philosophy
- Soft enforcement: focus and visibility cues, not hard locks.
- Authentication stays with existing lockers (e.g., hyprlock).
- The overlay is always-on-top but never owns the session lock.

## Default Behavior
- Work/break cadence: 30 minutes of work, 60-second break.
- Snooze: 5-minute base that shrinks with each use until a floor is reached.
- Interaction: `Enter` starts a break or dismisses after completion; `z` snoozes when due.
- After dismissing a break, the work timer restarts immediately.

## Architecture (MVP)
- Scheduler: a small Rust state machine handling timing and snooze decay.
- Wayland overlay: fullscreen dim overlay on all outputs.
- Backend: wlr-layer-shell for wlroots and Hyprland.
- Optional Hyprland enhancements: temporary submap or special workspace while active.

## Build and Run
- `cargo build` builds debug binaries.
- `cargo run -- <args>` runs the CLI with arguments.
- `cargo build --release` builds optimized binaries.
- `cargo fmt` and `cargo clippy -- -D warnings` keep style and linting clean.

## Assets and Audio
- Fonts, SVGs, and audio cues are embedded in the binary via `include_bytes!`.
- Audio playback is in-process (rodio) using Opus assets; build environments need ALSA and Opus development headers plus `pkg-config` (see `devenv.nix`).
- If system Opus libs are missing, the build can fall back to bundled Opus via `.cargo/config.toml`.

## Current Direction
The Wayland overlay now uses wlr-layer-shell, and the scheduler has initial tests. Next steps include tightening compositor hints, refining overlay visuals, and expanding coverage. Future expansions may include gentle animations, breathing prompts, stats/streaks, and multiple break types.

## Non-Goals (MVP)
- No authentication or password handling.
- No session-lock ownership.
- No GUI configuration; CLI-only with wellness-oriented defaults.
