# `omnibin-shell`: the whole thing, in a mount namespace, on any Linux.
#
# This is the version for a machine you care about. The lazy store appears at
# /nix/store inside a namespace only this shell and its children can see, and
# it disappears when the shell exits. The host's own store is served through
# it untouched, so software the machine already has keeps working.
#
# The daemon deliberately runs OUTSIDE the namespace. omnibin is a process
# whose own libraries live in /nix/store; serving /nix/store from inside the
# namespace means a page fault on one of them is answered by a thread blocked
# on that same fault, and the whole namespace hangs. Mounting over a live
# store from within is possible, and the NixOS module does it as root with
# mlockall, but it is not worth needing privileges for here.
#
# So: the daemon serves a neutral mountpoint in the host's namespace, where
# /nix/store is still the real one and nothing can recurse, and the namespace
# binds that mount over /nix/store. The passthrough is then simply the real
# store, with no bind mount needed at all.
{ pkgs, omnibin }:
let
  # The inner half, as its own script. Nesting three levels of shell quoting
  # into an unshare invocation is how you get a script nobody can edit.
  inner = pkgs.writeShellScript "omnibin-shell-inner" ''
    set -eu

    # Everything below is invisible to the host. This is the only mount the
    # namespace makes, and it is what puts thirteen years of nixpkgs at the
    # absolute paths every binary in them expects.
    mount --bind "$OMNIBIN_STORE" /nix/store

    # Last on PATH. First would mean every mistyped command becomes a lookup
    # in a filesystem with tens of thousands of answers, and would shadow the
    # tools the surrounding project pinned.
    PATH="$PATH:$OMNIBIN_TREE/bin"
    export PATH

    exec "$@"
  '';
in
pkgs.writeShellApplication {
  name = "omnibin-shell";
  runtimeInputs = [
    omnibin
    pkgs.util-linux
    pkgs.coreutils
    pkgs.fuse3
  ];
  text = ''
    runtime=''${XDG_RUNTIME_DIR:-/tmp/omnibin-$(id -u)}

    # Being root in a user namespace is not being root on the host, which
    # decides where things go: / belongs to real root, so /omnibin cannot be
    # created. The tree goes under XDG_RUNTIME_DIR, which this user owns, and
    # $OMNIBIN_TREE names it. On a machine where the NixOS module did the
    # mounting there is a real /omnibin.
    OMNIBIN_TREE=''${OMNIBIN_TREE:-$runtime/omnibin}
    OMNIBIN_STORE=''${OMNIBIN_STORE:-$runtime/omnibin-store}
    OMNIBIN_CACHE=''${OMNIBIN_CACHE:-''${XDG_CACHE_HOME:-$HOME/.cache}/omnibin}
    export OMNIBIN_TREE OMNIBIN_STORE OMNIBIN_CACHE

    mkdir -p "$OMNIBIN_CACHE"

    # A previous run that was killed rather than exited leaves its mounts
    # behind, and a stale FUSE mountpoint refuses even stat(), so mounting
    # over it fails with a permission error that has nothing to do with
    # permissions. Clear them before making the directories.
    for stale in "$OMNIBIN_TREE" "$OMNIBIN_STORE"; do
      fusermount3 -u "$stale" 2>/dev/null || true
    done
    mkdir -p "$OMNIBIN_TREE" "$OMNIBIN_STORE"

    omnibin mount \
      --store "$OMNIBIN_STORE" \
      --passthrough /nix/store \
      --tree "$OMNIBIN_TREE" \
      --cache-dir "$OMNIBIN_CACHE" &
    mount_pid=$!
    trap 'kill "$mount_pid" 2>/dev/null || true' EXIT

    # Wait for the tree to answer before handing over the shell, so the first
    # command typed is not a race with the mount.
    ready=""
    for _ in $(seq 1 100); do
      if [ -e "$OMNIBIN_TREE/README.md" ]; then
        ready=yes
        break
      fi
      sleep 0.1
    done

    if [ -z "$ready" ]; then
      echo "omnibin-shell: the mount never came up" >&2
      exit 1
    fi

    if [ "$#" -eq 0 ]; then
      set -- "''${SHELL:-/bin/sh}"
    fi

    unshare --user --map-root-user --mount -- ${inner} "$@"
  '';
}
