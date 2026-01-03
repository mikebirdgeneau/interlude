# Interlude

Interlude helps you take regular, short breaks while working. It runs quietly in the background on Linux Wayland systems and uses a simple dim screen overlay to remind you when it is time to pause.

It does not lock your session or manage passwords. Your existing screen locker continues to handle authentication.

## What It Does
- Reminds you to step away at regular intervals
- Shows a fullscreen dim overlay when a break is due
- Runs as a lightweight background service

## What It Does Not Do
- Lock your session
- Force breaks
- Replace tools like hyprlock

## Default Behaviour
- 30 minutes of work, followed by a short break
- Breaks last 180 seconds by default
- Snooze starts at 5 minutes and shortens if used repeatedly
- Keyboard controls:
  - `Enter`: start or dismiss a break
  - `z`: snooze when a break is due
- After a break, the next work period starts immediately

## Usage

```bash
cargo build            # debug build
cargo run -- <args>    # run with CLI options
cargo build --release  # optimized build
cargo fmt
cargo clippy -- -D warnings
```

### CLI Parameters

```
Usage: interlude [OPTIONS]

Options:
      --interval-minutes <INTERVAL_MINUTES>
          Minutes between breaks [default: 30]
      --break-seconds <BREAK_SECONDS>
          Break duration in seconds [default: 180]
      --snooze-base-seconds <SNOOZE_BASE_SECONDS>
          Initial snooze duration in seconds (shrinks each snooze) [default: 300]
      --snooze-decay <SNOOZE_DECAY>
          Snooze decay multiplier applied each time you snooze (0 < decay < 1) [default: 0.6]
      --snooze-min-seconds <SNOOZE_MIN_SECONDS>
          Minimum snooze duration in seconds [default: 30]
      --max-snoozes <MAX_SNOOZES>
          Optional: after N snoozes in a cycle, disable snooze (0 = unlimited) [default: 0]
      --immediate
          Immediately start a break sequence (for testing)
      --background <BACKGROUND>
          Background overlay color in hex (#RGB, #RRGGBB, or #RRGGBBAA) [default: #000000CC]
      --foreground <FOREGROUND>
          Foreground text/icon color in hex (#RGB, #RRGGBB, or #RRGGBBAA) [default: #FFFFFFDD]
  -h, --help
          Print help
```

## NixOS (Flake)

Interlude includes a NixOS module that runs it as a user service.

```nix
{
  inputs.interlude.url = "path:/home/mjbirdge/Documents/rust/interlude";

  outputs = { self, nixpkgs, interlude, ... }: {
    nixosConfigurations.myhost = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        interlude.nixosModules.default
        ({ ... }: {
          services.interlude = {
            enable = true;
            settings = {
              interval_minutes = 30;
              break_seconds = 180;
              snooze_base_seconds = 300;
              snooze_decay = 0.6;
              snooze_min_seconds = 30;
              max_snoozes = 0;
              immediate = false;
              background = "#000000CC";
              foreground = "#FFFFFDD";
            };
          };
        })
      ];
    };
  };
}
```

## Assets and Audio

Interlude embeds its visual and audio assets directly in the binary.

All third-party assets used by this project are listed below. If you change or add assets, please update this section and ensure the licenses remain compatible.

## Attribution

### SVG icon
- **Tabler Icons** (icon downloaded from tabler.io)  
  License: MIT  
  Source: https://tabler.io/icons  
  License text: https://tabler.io/license

### Audio cue
- **“CD_VIE_009FX_Gong_3”** by **kevp888** (Kevin Luce), from Freesound  
  License: Creative Commons Attribution 4.0 (CC BY 4.0)  
  Source: https://freesound.org/people/kevp888/sounds/710760/  
  Attribution: “kevp888” or “Kevin Luce” with a link to the source page

## Direction

Interlude is intentionally minimal. Future changes focus on visual clarity, reliability across compositors, and small quality-of-life improvements.

## Non-Goals
- Session locking or authentication
- Hard enforcement of breaks
- Graphical configuration tools
