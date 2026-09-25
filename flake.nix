{
  description = "Every binary nixpkgs ever shipped, on your PATH";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs =
    { self, nixpkgs }:
    let
      # Linux only, and deliberately. The lazy store has to appear at
      # /nix/store, which means FUSE over a mount namespace; macOS has neither
      # in a form this can use, and the indexed paths are Linux builds anyway.
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f system);
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          data = import ./nix/data.nix { inherit pkgs system; };

          # Wrapped with the pinned database when one has been published for
          # this system, and taking --db when one has not.
          omnibin = import ./nix/omnibin.nix {
            inherit pkgs;
            inherit (data) database;
          };
        in
        {
          inherit omnibin;
          default = omnibin;

          # The safe way to use this on a machine you care about: the lazy
          # store lives in a mount namespace that dies with the shell.
          omnibin-shell = import ./nix/omnibin-shell.nix { inherit pkgs omnibin; };

          # A NixOS machine where every package is already installed.
          vm = import ./nix/vm.nix {
            inherit nixpkgs system omnibin;
            module = self.nixosModules.default;
          };

          # The same thing as a container image.
          docker = import ./nix/docker.nix { inherit pkgs omnibin; };

          # The raw crawl, republished: every file listing, as JSON Lines.
          listings = data.listings;
        }
      );

      apps = forAllSystems (system: {
        default = {
          type = "app";
          program = "${self.packages.${system}.omnibin-shell}/bin/omnibin-shell";
        };
        vm = {
          type = "app";
          program = "${self.packages.${system}.vm}/bin/run-nixos-vm";
        };
      });

      nixosModules.default =
        { pkgs, ... }:
        {
          imports = [ ./nix/module.nix ];
          services.omnibin.package = nixpkgs.lib.mkDefault self.packages.${pkgs.system}.omnibin;
        };

      devShells = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          # Everything the pipeline needs: the crawler's two Python modules,
          # the decompressors, and a Rust toolchain.
          default = pkgs.mkShell {
            packages = [
              (pkgs.python3.withPackages (ps: [
                ps.brotli
                ps.zstandard
              ]))
              pkgs.cargo
              pkgs.rustc
              pkgs.rustfmt
              pkgs.clippy
              pkgs.pkg-config
              pkgs.fuse3
              pkgs.zstd
              pkgs.xz
              pkgs.bzip2
              pkgs.brotli
              pkgs.sqlite
              pkgs.gh
            ];
          };
        }
      );

      formatter = forAllSystems (
        system: import ./nix/formatter.nix { pkgs = nixpkgs.legacyPackages.${system}; }
      );
    };
}
