# What `nix flake check` verifies beyond the packages building.
#
# These run in the sandbox rather than in a shell somebody remembered to
# invoke, which is the point: a lint that only runs when CI calls it is a lint
# that stops running the moment the workflow changes shape.
{ pkgs, self }:
{
  # House style over the whole tree, code comments included: no em dashes,
  # none of the vocabulary that marks machine written prose.
  #
  # The source is copied in rather than read from the store directly, so the
  # script sees a checkout with itself at tools/ and derives the same root it
  # would locally. check-prose.py fails when it finds nothing to read, which
  # is what catches a skip rule that has swallowed the tree.
  prose = pkgs.runCommand "omnibin-prose" { nativeBuildInputs = [ pkgs.python3 ]; } ''
    cp -r ${self} ./source
    chmod -R u+w ./source
    cd ./source
    python3 tools/check-prose.py
    touch $out
  '';

  # Tests that the module mounts over /nix/store without wedging the machine,
  # by booting it at its defaults and running programs from the passthrough.
  # No network in the sandbox, so fetching is left to the mount test.
  module = pkgs.testers.runNixOSTest {
    name = "omnibin-module";

    # A wedged store hangs the driver's shell past any per-command timeout.
    globalTimeout = 600;

    nodes.machine = {
      imports = [ self.nixosModules.default ];
      services.omnibin.enable = true;
    };

    testScript = ''
      machine.wait_for_unit("omnibin.service", timeout=120)
      machine.wait_until_succeeds("test -e /omnibin/README.md", timeout=120)

      # The store the machine runs from is the lazy one, not the real one.
      machine.succeed("findmnt -n -o SOURCE /nix/store | grep -qx omnibin")

      # A program the machine shipped with still runs, from the passthrough.
      machine.succeed("${pkgs.hello}/bin/hello")

      # A login shell reads its profile out of the store.
      machine.succeed("su -l root -c true")

      # The tree answers from the index.
      machine.succeed("test -L /omnibin/bin/python3")

      # Stopping hands /nix/store back to the real store. --no-block because
      # this systemctl runs from the lazy store and would get SIGBUS.
      machine.succeed("systemctl stop --no-block omnibin.service")
      machine.wait_until_fails("findmnt -n -o SOURCE /nix/store | grep -qx omnibin", timeout=60)
      machine.succeed("${pkgs.hello}/bin/hello")
    '';
  };
}
