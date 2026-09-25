# The NixOS module: mount omnibin at boot.
#
# This is the shape for a machine that exists to have every package: a VM, a
# container, a throwaway box an agent works in. It replaces /nix/store with the
# lazy store for the whole system, which is exactly what you want there and is
# not what you want on your laptop. On a machine you care about, use
# `omnibin-shell`, which does the same thing inside a mount namespace that dies
# with the shell.
#
# The passthrough bind is what keeps the system bootable. Every path the host
# already has, the kernel and systemd and this service's own binary, is served
# from the real store and never fetched, so the system does not depend on the
# network to run the software it already has.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.omnibin;

  # Where the real store is bound before the lazy one is mounted over it. In
  # /run because it must not survive a reboot and must exist before any of this
  # runs.
  realStore = "/run/omnibin/real-store";
in
{
  options.services.omnibin = {
    enable = lib.mkEnableOption "the omnibin lazy store";

    package = lib.mkOption {
      type = lib.types.package;
      description = "The omnibin package, wrapped with the database it reads.";
    };

    tree = lib.mkOption {
      type = lib.types.str;
      default = "/omnibin";
      description = "Where the browsable tree of executables is mounted.";
    };

    mountStore = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Serve /nix/store lazily for the whole system. The store paths in
        {file}`/omnibin/bin` are absolute, so with this off the tree resolves
        only for packages the machine already has.

        Leave it on for a VM or a container. Turning it on for a workstation
        puts a FUSE process in the path of every binary the system runs.
      '';
    };

    cacheDir = lib.mkOption {
      type = lib.types.str;
      default = "/var/cache/omnibin";
      description = "Where fetched store paths and listings are kept.";
    };

    addToPath = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Put {file}`/omnibin/bin` on the system PATH, last, so it answers for a
        command nothing else provides and never shadows one that something
        does.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ];

    # Last on PATH on purpose. First would mean every mistyped command in the
    # shell becomes a lookup in a filesystem with 32,000 answers.
    environment.extraInit = lib.mkIf cfg.addToPath ''
      export PATH="$PATH:${cfg.tree}/bin"
    '';

    systemd.services.omnibin = {
      description = "omnibin lazy Nix store";
      wantedBy = [ "multi-user.target" ];
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];

      # Nothing that runs before this can depend on it, and everything that
      # runs after it can. A unit ordered before local-fs would deadlock on
      # its own binary.
      unitConfig.DefaultDependencies = false;

      serviceConfig = {
        Type = "simple";
        ExecStartPre = [
          "${pkgs.coreutils}/bin/mkdir -p ${realStore} ${cfg.tree} ${cfg.cacheDir}"
        ]
        ++ lib.optional cfg.mountStore "${pkgs.util-linux}/bin/mount --bind /nix/store ${realStore}";

        ExecStart = lib.concatStringsSep " " (
          [
            "${cfg.package}/bin/omnibin"
            "mount"
            "--tree ${cfg.tree}"
            "--cache-dir ${cfg.cacheDir}"
            "--allow-other"
          ]
          ++ lib.optionals cfg.mountStore [
            "--store /nix/store"
            "--passthrough ${realStore}"
          ]
          ++ lib.optional (!cfg.mountStore) "--store ${realStore}"
        );

        Restart = "on-failure";
        RestartSec = 2;
      };
    };

    # allow_other is what lets every user on the machine read the mounts, and
    # fusermount refuses to set it up without this.
    environment.etc."fuse.conf".text = ''
      user_allow_other
    '';
  };
}
