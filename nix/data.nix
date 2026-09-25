# The published data artifacts, fetched by pin.
#
# data-pins.json is the only thing in this repository that knows where the
# crawl output lives. Each entry names the release tag that froze a file and
# the NAR hash it must have, so a re-uploaded or tampered asset fails the
# build rather than being served.
#
# The database is the artifact the filesystem needs. The listings shards are
# the raw crawl, republished so anybody can rebuild the database, or read the
# file listings directly, without repeating a 600,000-request crawl.
{ pkgs, system }:
let
  pins = builtins.fromJSON (builtins.readFile ../data-pins.json);
  fetchArtifact = import ./fetch-artifact.nix { inherit pkgs; };

  fetchPinned =
    name:
    let
      pin =
        pins.files.${name}
          or (throw "omnibin: ${name} is not pinned in data-pins.json; run tools/cut-data-release.sh");
    in
    fetchArtifact {
      url = "${pins.baseUrl}/${pin.tag}/${name}";
      hash = pin.narHash;
    };

  databaseName = "omnibin-${system}.db";
in
{
  inherit fetchPinned;

  # The index for this system, or null before the first data release names
  # one. Null rather than a throw so the flake evaluates on a system nothing
  # has been published for yet; the package is then built unwrapped and takes
  # --db, and it starts carrying its own database the moment a cut lands.
  database = if pins.files ? ${databaseName} then fetchPinned databaseName else null;

  # Every listings shard the pins name, as one directory.
  listings = pkgs.runCommand "omnibin-listings" { } (
    ''
      mkdir -p $out
    ''
    + pkgs.lib.concatMapStrings (name: ''
      ln -s ${fetchPinned name} $out/${name}
    '') (builtins.filter (pkgs.lib.hasPrefix "listings-") (builtins.attrNames pins.files))
  );
}
