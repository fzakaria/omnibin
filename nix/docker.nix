# An OCI image: everything nixpkgs ever shipped, in a container.
#
# The image is built by Nix, so its layers are a real /nix/store holding the
# shell, omnibin, and the index. That store has to keep working, which makes
# this the same problem a NixOS machine has and not the blank slate it looks
# like: the lazy store is served with the image's own store as passthrough, so
# every path that shipped in the image is read from the layers and never
# fetched.
#
# The daemon serves a neutral mountpoint and an unshared mount namespace binds
# it over /nix/store, which is the same shape omnibin-shell uses and for the
# same reason. omnibin's libraries live in /nix/store, so a daemon serving
# /nix/store from inside the namespace can page fault on itself and hang. The
# alternative is mlockall, which needs a memlock ulimit that Docker does not
# give a container by default.
#
#   docker run --rm -it --device /dev/fuse --cap-add SYS_ADMIN fzakaria/omnibin
#
# SYS_ADMIN is for mount, and /dev/fuse is for FUSE. Nothing else is needed.
{ pkgs, omnibin }:
let
  # Where the lazy store is served before it is bound over /nix/store.
  lazyStore = "/run/omnibin/store";
  tree = "/omnibin";
  cache = "/var/cache/omnibin";

  entrypoint = pkgs.writeShellScript "omnibin-entrypoint" ''
    set -e

    mkdir -p ${lazyStore} ${tree} ${cache}

    omnibin mount \
      --store ${lazyStore} \
      --passthrough /nix/store \
      --tree ${tree} \
      --cache-dir ${cache} &

    # Wait for the tree to answer, so the first command the container runs is
    # not a race with the mount.
    for _ in $(seq 1 100); do
      if [ -e "${tree}/README.md" ]; then
        break
      fi
      sleep 0.1
    done

    if [ ! -e "${tree}/README.md" ]; then
      echo "omnibin: the mount never came up; is /dev/fuse present?" >&2
      exit 1
    fi

    # The bind lives in its own mount namespace, so the daemon outside it keeps
    # seeing the image's real store and can serve from it.
    exec ${pkgs.util-linux}/bin/unshare --mount -- \
      ${pkgs.writeShellScript "omnibin-enter" ''
        set -e
        mount --bind ${lazyStore} /nix/store
        export PATH="$PATH:${tree}/bin"
        exec "$@"
      ''} "$@"
  '';
in
pkgs.dockerTools.buildLayeredImage {
  name = "omnibin";
  tag = "latest";

  contents = [
    omnibin
    pkgs.bashInteractive
    pkgs.coreutils
    pkgs.util-linux
    pkgs.sqlite
    pkgs.dockerTools.caCertificates
  ];

  config = {
    Entrypoint = [ "${entrypoint}" ];
    Cmd = [ "${pkgs.bashInteractive}/bin/bash" ];
    Env = [
      "PATH=/bin:/usr/bin:${tree}/bin"
      "SSL_CERT_FILE=/etc/ssl/certs/ca-bundle.crt"
    ];
  };
}
