# One pinned data artifact, as a fixed-output derivation.
#
# fetchurl, not fetchTree: fetchTree downloads during evaluation, and nothing
# reads these until a build does. recursiveHash, because data-pins.json records
# NAR hashes rather than flat ones.
#
# unsafeDiscardReferences is the part that is not optional. Nix scans a build's
# output for the bare hash part of every store path in its input closure, and
# these artifacts are lists of store path digests, so the scan finds hundreds
# of them and refuses the build: a fixed-output derivation may not have
# references at all. __structuredAttrs is set explicitly because fetchurl only
# sets it from 26.05 on, and the discard is ignored without it.
{ pkgs }:
{ url, hash }:
pkgs.fetchurl {
  inherit url hash;
  recursiveHash = true;
  derivationArgs = {
    __structuredAttrs = true;
    unsafeDiscardReferences.out = true;
  };
}
