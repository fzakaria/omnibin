# Caveats

**Nothing is local.** This is a window onto cache.nixos.org, not a copy of it.
With no network you can read the index and anything already fetched, and
nothing else.

**Cold starts are downloads.** A first `python3@3.6.2` is 92 MB over three
store paths and 2.7 seconds. The filesystem fetches at store-path granularity
because the cache's NARs are single compression streams and cannot be
range-read; see [design.md](./design.md#does-a-nar-index-help).

**Coverage is whatever the cache still has.** The multiverse census re-checks
liveness weekly, and omnibin only advertises paths it found alive. The tail
thins out going back in time, and a 2013 binary that does substitute may still
fail to run on a modern kernel.

**One system per database.** A store path belongs to one system. The database
is built for one, and `x86_64-linux` is the one that has real coverage;
`aarch64-linux` is thinner, and darwin has no lazy store here at all because
there is no FUSE-over-`/nix/store` story on it worth having.

**`ls /nix/store` lies by omission.** It lists what has been fetched. The
index is how you enumerate.

**Bare names are a policy, not a fact.** `python3` resolving to the `python3`
attribute's newest build is a rule this project chose. Two packages can ship
the same executable at the same version — `curl` and `curlWithGnuTls` both
have a `curl` 8.10.1 — and `<name>@<version>` picks between them by the same
rule. When it matters, go through `/nix/store` directly.

**Nix inside will not work.** The lazy store is a filesystem, not a registered
Nix store: there is no database behind it, so `nix build` and `nix-store
--query` have nothing to read. omnibin is for consuming packages. If you need
Nix itself, use it on the real store through the passthrough.

**The FUSE process is a dependency of everything.** On a machine where the
module mounted `/nix/store`, killing omnibin takes the store with it until
systemd restarts it. This is the reason `omnibin-shell` exists and the reason
the module is documented for machines you can throw away.
