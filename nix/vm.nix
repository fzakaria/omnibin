# A NixOS VM with every package in it: `nix run .#vm`.
#
# The demo, and the shape the agent sandbox takes. Nothing is installed beyond
# a shell and the tools needed to look around; every other command on this
# machine's PATH is fetched the first time somebody runs it.
{
  nixpkgs,
  system,
  module,
  omnibin,
}:
(nixpkgs.lib.nixosSystem {
  inherit system;
  modules = [
    module
    (
      { pkgs, lib, ... }:
      {
        services.omnibin = {
          enable = true;
          package = omnibin;
        };

        # A disposable machine: log in as root, no password, no state.
        users.users.root.password = "";
        services.getty.autologinUser = "root";

        virtualisation.vmVariant.virtualisation = {
          memorySize = 4096;
          diskSize = 16 * 1024;
          graphics = false;
        };

        environment.systemPackages = with pkgs; [
          sqlite
          jq
        ];

        users.motd = ''
          Every executable nixpkgs ever shipped is on your PATH.

            ls /omnibin/bin | wc -l
            python3 --version
            /omnibin/bin/python3@3.6.2 --version

          Read /omnibin/README.md before exploring. Query the index, do not
          walk the tree.
        '';

        system.stateVersion = lib.versions.majorMinor lib.version;
      }
    )
  ];
}).config.system.build.vm
