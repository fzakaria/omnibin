# An OCI image: everything nixpkgs ever shipped, in a container.
#
# The container has no real Nix store of its own, so there is nothing to
# preserve and the lazy store is mounted straight at /nix/store with no
# passthrough. FUSE inside a container needs the device and the capability:
#
#   docker run --rm -it --device /dev/fuse --cap-add SYS_ADMIN omnibin
#
# The entrypoint mounts, waits for the tree, and execs a shell.
{ pkgs, omnibin }:
pkgs.dockerTools.buildLayeredImage {
  name = "omnibin";
  tag = "latest";

  contents = [
    omnibin
    pkgs.bashInteractive
    pkgs.coreutils
    pkgs.sqlite
    pkgs.dockerTools.caCertificates
  ];

  config = {
    Entrypoint = [
      "${pkgs.writeShellScript "omnibin-entrypoint" ''
        set -e
        mkdir -p /omnibin /var/cache/omnibin

        omnibin mount --store /nix/store --tree /omnibin \
          --cache-dir /var/cache/omnibin &

        for _ in $(seq 1 50); do
          [ -e /omnibin/README.md ] && break
          sleep 0.1
        done

        export PATH="$PATH:/omnibin/bin"
        exec "$@"
      ''}"
    ];
    Cmd = [ "${pkgs.bashInteractive}/bin/bash" ];
    Env = [ "PATH=/bin:/usr/bin:/omnibin/bin" ];
  };
}
