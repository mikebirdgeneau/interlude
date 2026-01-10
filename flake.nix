{
  description = "Interlude: short, recurring wellness breaks";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f system);
      cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
      packageVersion = cargoToml.package.version;

      in
      {
        packages = forAllSystems (system:
            let
            pkgs = import nixpkgs { inherit system; };
            in
            {
            default = pkgs.rustPlatform.buildRustPackage {
            pname = "interlude";
            version = packageVersion;
            src = ./.;
            cargoLock = {
            lockFile = ./Cargo.lock;
            };
            nativeBuildInputs = [
            pkgs.pkg-config
            ];
            buildInputs = [
            pkgs.alsa-lib
            pkgs.libopus
            pkgs.libxkbcommon
            pkgs.wayland
            ];
            };
            });

      apps = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        {
          default = {
            type = "app";
            program = "${self.packages.${system}.default}/bin/interlude";
          };
        });

      nixosModules.default = { config, lib, pkgs, ... }:
        let
          cfg = config.programs.interlude;
          svc = config.services.interlude;
        in
        {
          options = {
            programs.interlude = {
              enable = lib.mkEnableOption "Interlude wellness break overlay";
              package = lib.mkOption {
                type = lib.types.package;
                default = self.packages.${pkgs.system}.default;
                description = "Interlude package to install.";
              };
            };

            services.interlude = {
              enable = lib.mkEnableOption "Interlude wellness break service";
              package = lib.mkOption {
                type = lib.types.package;
                default = self.packages.${pkgs.system}.default;
                description = "Interlude package to run in the user session.";
              };
              sessionTarget = lib.mkOption {
                type = lib.types.nullOr lib.types.str;
                default = null;
                description = ''
                  Optional systemd user target that should be active
                  before Interlude starts.
                '';
              };
              settings = lib.mkOption {
                type = lib.types.submodule {
                  options = {
                    interval_minutes = lib.mkOption {
                      type = lib.types.ints.positive;
                      default = 30;
                      description = "Minutes between breaks.";
                    };
                    break_seconds = lib.mkOption {
                      type = lib.types.ints.positive;
                      default = 180;
                      description = "Break duration in seconds.";
                    };
                    snooze_base_seconds = lib.mkOption {
                      type = lib.types.ints.positive;
                      default = 300;
                      description = "Initial snooze duration in seconds.";
                    };
                    snooze_decay = lib.mkOption {
                      type = lib.types.float;
                      default = 0.6;
                      description = "Snooze decay multiplier (0 < decay < 1).";
                    };
                    snooze_min_seconds = lib.mkOption {
                      type = lib.types.ints.positive;
                      default = 25;
                      description = "Minimum snooze duration in seconds.";
                    };
                    max_snoozes = lib.mkOption {
                      type = lib.types.ints.unsigned;
                      default = 0;
                      description = "Max snoozes before disabling (0 = unlimited).";
                    };
                    immediate = lib.mkOption {
                      type = lib.types.bool;
                      default = false;
                      description = "Immediately start a break sequence.";
                    };
                    background = lib.mkOption {
                      type = lib.types.str;
                      default = "#000000CC";
                      description = "Background overlay color in hex.";
                    };
                    foreground = lib.mkOption {
                      type = lib.types.str;
                      default = "#FFFFFFDD";
                      description = "Foreground text/icon color in hex.";
                    };
                    fade_fps = lib.mkOption {
                      type = lib.types.ints.positive;
                      default = 60;
                      description = "Target FPS during fade animations.";
                    };
                  };
                };
                default = { };
                description = "Interlude CLI settings mapped to flags.";
              };
            };
          };

          config = lib.mkMerge [
            (lib.mkIf cfg.enable {
              environment.systemPackages = [ cfg.package ];
            })
            (lib.mkIf svc.enable {
              systemd.user.targets.interlude-session = {
                description = "Interlude wellness break session target";
              } // lib.optionalAttrs (svc.sessionTarget != null) {
                wants = [ svc.sessionTarget ];
                after = [ svc.sessionTarget ];
              };
              systemd.user.services.interlude = {
              description = "Interlude wellness break overlay";
              wantedBy = [ "interlude-session.target" ];
              partOf = [ "interlude-session.target" ];
              after = [ "interlude-session.target" ];
              serviceConfig = {
                ExecStartPre = 
                  let
                    waitWayland = pkgs.writeShellScript "interlude-wait-wayland" ''
                      set -euo pipefail
                      # Prefer the actual socket for this session
                      if [ -n "''${XDG_RUNTIME_DIR:-}" ] && [ -n "''${WAYLAND_DISPLAY:-}" ]; then
                        sock="''${XDG_RUNTIME_DIR}/''${WAYLAND_DISPLAY}"
                        until [ -S "$sock" ]; do sleep 0.1; done
                        exit 0
                      fi

                      # Fallback: any wayland socket for this user
                      dir="''${XDG_RUNTIME_DIR:-/run/user/$UID}"
                      until compgen -G "$dir/wayland-*" >/dev/null; do sleep 0.1; done
                    '';
                  in
                  "${waitWayland}";
                ExecStart =
                  let
                    settings = svc.settings;
                    args = lib.flatten [
                      [ "--interval-minutes" (toString settings.interval_minutes) ]
                      [ "--break-seconds" (toString settings.break_seconds) ]
                      [ "--snooze-base-seconds" (toString settings.snooze_base_seconds) ]
                      [ "--snooze-decay" (toString settings.snooze_decay) ]
                      [ "--snooze-min-seconds" (toString settings.snooze_min_seconds) ]
                      [ "--max-snoozes" (toString settings.max_snoozes) ]
                      (lib.optional settings.immediate "--immediate")
                      [ "--background" settings.background ]
                      [ "--foreground" settings.foreground ]
                      [ "--fade-fps" (toString settings.fade_fps) ]
                    ];
                    escapedArgs = lib.escapeShellArgs args;
                    resetScript = pkgs.writeShellScript "interlude-start" ''
                      runtime_dir="''${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
                      marker="$runtime_dir/interlude-reset-on-boot"
                      extra=""
                      if [ ! -f "$marker" ]; then
                        extra="--reset-state"
                        touch "$marker"
                      fi
                      exec ${svc.package}/bin/interlude ${escapedArgs} $extra
                    '';
                  in
                  "${resetScript}";
                Restart = "on-failure";
                RestartSec = "2s";
              };
              };
            })
          ];
        };
    };
}
