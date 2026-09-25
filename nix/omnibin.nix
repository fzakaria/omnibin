# The omnibin binary, wrapped with the database it reads and the tools it
# shells out to.
#
# The wrapper is what makes the data version be the package version: OMNIBIN_DB
# points at a pinned artifact, so two people running the same `nix run` get the
# same answers, and a newer crawl arrives through `nix flake update`.
#
# The decompressors are on PATH rather than linked in. The cache has published
# NARs as xz, bzip2 and zstd across thirteen years and serves `.ls` as brotli
# or zstd; running the four tools costs one subprocess per store path, which is
# invisible next to the download it is decoding, and keeps four C libraries out
# of this build.
{
  pkgs,
  database ? null,
}:
let
  inherit (pkgs) lib;

  unwrapped = pkgs.rustPlatform.buildRustPackage {
    pname = "omnibin";
    version = "0.1.0";

    # An allowlist rather than a bare `../.`: a working checkout carries
    # target/ and data/, hundreds of megabytes that change the store path
    # without changing the build.
    src = lib.fileset.toSource {
      root = ../.;
      fileset = lib.fileset.unions [
        ../Cargo.toml
        ../Cargo.lock
        ../src
      ];
    };
    cargoLock.lockFile = ../Cargo.lock;

    nativeBuildInputs = [ pkgs.pkg-config ];
    buildInputs = [ pkgs.fuse3 ];

    meta = {
      description = "Every binary nixpkgs ever shipped, on your PATH";
      mainProgram = "omnibin";
      license = lib.licenses.mit;
      platforms = lib.platforms.linux;
    };
  };

  runtimeTools = [
    pkgs.zstd
    pkgs.xz
    pkgs.bzip2
    pkgs.brotli
    pkgs.fuse3
  ];
in
pkgs.runCommand "omnibin"
  {
    nativeBuildInputs = [ pkgs.makeWrapper ];
    inherit (unwrapped) meta;
  }
  ''
    mkdir -p $out/bin
    makeWrapper ${unwrapped}/bin/omnibin $out/bin/omnibin \
      --prefix PATH : ${lib.makeBinPath runtimeTools} \
      ${lib.optionalString (database != null) "--set-default OMNIBIN_DB ${database}"}
  ''
