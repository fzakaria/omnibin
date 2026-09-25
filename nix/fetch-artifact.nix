# One pinned data artifact, as a fixed-output derivation.
#
# fetchurl, not fetchTree: fetchTree downloads during evaluation, and nothing
# reads these until a build does. recursiveHash, because data-pins.json records
# NAR hashes rather than flat ones.
{ pkgs }:
{ url, hash }:
pkgs.fetchurl {
  inherit url hash;
  recursiveHash = true;
}
