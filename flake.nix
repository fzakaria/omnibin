{
  description = "Every binary nixpkgs ever shipped, on your PATH";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  # The site is the expensive build in this flake: it projects two 250 MB
  # databases into 147 MB of shards every time the data moves. The cache means
  # a contributor, and the pages deploy, fetch that rather than rebuild it.
  nixConfig = {
    extra-substituters = [ "https://omnibin.cachix.org" ];
    extra-trusted-public-keys = [
      "omnibin.cachix.org-1:HWeLv8+LfqLqLDOoQJmvmW7m0ug1Fne/DYaxgdECHgw="
    ];
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

          # The browsable index at omnibin.dev: the search, the leaderboards
          # and the charts, all static files built from the same databases the
          # filesystem reads.
          site = import ./nix/site.nix { inherit pkgs self; };
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

        # `nix run .#serve [port]` serves the built site locally, which is the
        # same tree the pages workflow deploys.
        serve = {
          type = "app";
          program = "${
            import ./nix/serve.nix {
              pkgs = nixpkgs.legacyPackages.${system};
              site = self.packages.${system}.site;
            }
          }/bin/serve-site";
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
