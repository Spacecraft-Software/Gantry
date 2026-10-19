# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
#
# services.gantry: run Gantry as greetd's greeter, on the bare VT (default)
# or inside a niri + Alacritty kiosk for truecolor.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib)
    getExe
    mkDefault
    mkEnableOption
    mkIf
    mkOption
    mkPackageOption
    types
    ;

  cfg = config.services.gantry;
  toml = pkgs.formats.toml { };
  configFile = toml.generate "gantry.toml" cfg.settings;
  gantryCmd = "${getExe cfg.package} --config ${configFile}";

  alacrittyConfig = toml.generate "gantry-alacritty.toml" {
    window = {
      decorations = "None";
      startup_mode = "Fullscreen";
    };
    font = {
      normal.family = "Inconsolata";
      size = cfg.kiosk.fontSize;
    };
    mouse.hide_when_typing = true;
    # Security: no URL hints (they would run xdg-open as the greeter user on
    # anything typed into a field) and no clipboard access from escapes.
    hints.enabled = [ ];
    terminal.osc52 = "Disabled";
  };

  # niri runs Alacritty fullscreen, then quits when Gantry exits so greetd
  # can start the chosen session.
  niriConfig = pkgs.writeText "gantry-niri.kdl" ''
    hotkey-overlay {
        skip-at-startup
    }
    // No key bindings at all: nothing at the login screen may spawn programs.
    binds {
    }
    prefer-no-csd
    cursor {
        hide-when-typing
    }
    layout {
        gaps 0
        focus-ring {
            off
        }
        border {
            off
        }
    }
    window-rule {
        open-fullscreen true
    }
    spawn-at-startup "${pkgs.runtimeShell}" "-c" "${getExe cfg.kiosk.terminal} --config-file ${alacrittyConfig} -e ${gantryCmd}; ${getExe cfg.kiosk.compositor} msg action quit --skip-confirmation"
  '';

  kioskCmd = "${getExe cfg.kiosk.compositor} --config ${niriConfig}";
in
{
  options.services.gantry = {
    enable = mkEnableOption "Gantry, a themed greetd greeter";

    package = mkPackageOption pkgs "gantry" { };

    mode = mkOption {
      type = types.enum [
        "vt"
        "kiosk"
      ];
      default = "vt";
      description = ''
        Where Gantry draws. `vt` runs it directly on the Linux console and
        reprograms the console palette to the theme. `kiosk` runs it inside
        a fullscreen terminal under a minimal Wayland compositor for 24-bit
        colour and full Unicode.
      '';
    };

    settings = mkOption {
      inherit (toml) type;
      default = { };
      example = {
        general.effect = "fire";
        screensaver.idle_timeout = "PT10M";
        theme.name = "steelbore-blue";
      };
      description = ''
        Contents of `/etc/gantry/config.toml`. Every key is optional; see
        the Gantry manual for the schema.
      '';
    };

    fingerprint = {
      enable = mkOption {
        type = types.bool;
        default = config.services.fprintd.enable;
        defaultText = lib.literalExpression "config.services.fprintd.enable";
        description = ''
          Offer fingerprint login through `pam_fprintd`. The greetd PAM
          stack substacks `login`, so this sets `fprintAuth` and the
          `pam_fprintd` arguments on the `login` service, which also
          affects console login.
        '';
      };

      timeout = mkOption {
        type = types.ints.positive;
        default = 10;
        description = "Seconds `pam_fprintd` waits for a finger before falling back to the password.";
      };

      maxTries = mkOption {
        type = types.ints.positive;
        default = 3;
        description = "Failed matches `pam_fprintd` allows before falling back to the password.";
      };
    };

    kiosk = {
      compositor = mkPackageOption pkgs "niri" { };
      terminal = mkPackageOption pkgs "alacritty" { };
      fontSize = mkOption {
        type = types.numbers.positive;
        default = 13;
        description = "Terminal font size in kiosk mode.";
      };
    };
  };

  config = mkIf cfg.enable {
    services.greetd = {
      enable = true;
      # Keeps systemd boot messages off the console under the text UI.
      useTextGreeter = cfg.mode == "vt";
      settings = {
        default_session = {
          command = if cfg.mode == "vt" then gantryCmd else kioskCmd;
          user = mkDefault "greeter";
        };
      };
    };

    environment.etc."gantry/config.toml".source = configFile;
    environment.systemPackages = [ cfg.package ];

    systemd.tmpfiles.rules =
      let
        user = config.services.greetd.settings.default_session.user;
      in
      [ "d /var/cache/gantry 0750 ${user} ${user} - -" ];

    fonts.packages = mkIf (cfg.mode == "kiosk") [ pkgs.inconsolata ];

    # nixpkgs' greetd PAM service has no rules of its own: it substacks
    # `login` (useDefaultRules = false), so fingerprint lives on `login`.
    security.pam.services.login = {
      # Overrides nixpkgs' default (fprintd.enable) in both directions.
      fprintAuth = mkDefault cfg.fingerprint.enable;
      rules.auth.fprintd.settings = mkIf cfg.fingerprint.enable {
        timeout = cfg.fingerprint.timeout;
        max-tries = cfg.fingerprint.maxTries;
      };
    };

    assertions = [
      {
        assertion = cfg.fingerprint.enable -> config.services.fprintd.enable;
        message = "services.gantry.fingerprint.enable needs services.fprintd.enable = true.";
      }
    ];
  };
}
