# Documentation

Every binary nixpkgs ever shipped, on your PATH, fetched only when something
reads it. These pages cover what it is, how to run it, and what it costs.

1. [Using it](./using.md) covers the three ways in: a shell on any Linux, a
   NixOS module, and a container image.
2. [Design](./design.md) explains how a filesystem can show all of nixpkgs
   without holding any of it, why the metadata half is already published, and
   what a NAR index would and would not buy.
3. [Building the index](./building-the-index.md) covers the crawl, the
   sharding, and how a data release is cut.
4. [Caveats](./caveats.md) is the honest list of what does not work.

The index itself is browsable at <https://omnibin.dev/>, and every store path
it names is in [nixpkgs-multiverse](https://nixmultiverse.com/), which is
where they all come from.
