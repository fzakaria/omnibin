# omnibin

Every binary nixpkgs ever shipped, on your PATH.

```console
$ ls /omnibin/bin | wc -l
35940

$ python3 --version
Python 3.14.6

$ python3@3.6.2 --version
Python 3.6.2
```

Nothing was installed. Nothing was built. The machine had none of those
packages a second ago and still does not have most of them: `/omnibin/bin` is
a filesystem, `/nix/store` underneath it is a filesystem, and a package's
bytes are fetched from [cache.nixos.org](https://cache.nixos.org) the first
time something reads a file inside it.

**Documentation:** [Design](./docs/design.md) ·
[Using it](./docs/using.md) ·
[Building the index](./docs/building-the-index.md) ·
[Caveats](./docs/caveats.md)

## Why

Every environment an agent works in begins with somebody guessing which
packages it will need. Guess short and the agent is stuck; guess long and you
are shipping a ten-gigabyte image to run `jq` twice. Either way the agent's
first move on a real task is to install something, which is the one thing a
sealed environment is supposed to prevent.

omnibin removes the guess. The machine starts with every version of every
package already on its PATH, weighing nothing, and pays only for what is
actually run.

The other half is history. [nixpkgs-multiverse] resolves any
`(attribute, version)` in thirteen years of nixpkgs to the store path Hydra
built for it, so an old version is not a build, it is a download. 416 packages
have shipped something called `python3`; `python3@3.6.2` is one of them, and
reaching it costs a download rather than an afternoon.

## Four ways in

**A shell, on any Linux.** The lazy store is mounted over `/nix/store` inside
a user and mount namespace that belongs to this shell and dies with it. Your
real store is served through it untouched, so the software you already have
keeps working — including the shell you are typing into.

```console
$ nix run github:fzakaria/omnibin
```

**A NixOS machine.** One module, and every package is installed on it.

```nix
{
  imports = [ inputs.omnibin.nixosModules.default ];
  services.omnibin.enable = true;
}
```

**A VM**, which is that module with a login and nothing else:

```console
$ nix run github:fzakaria/omnibin#vm
```

**A container.** FUSE needs the device and the capability; the image needs
nothing else.

```console
$ nix build github:fzakaria/omnibin#docker && docker load < result
$ docker run --rm -it --device /dev/fuse --cap-add SYS_ADMIN omnibin
```

## How it works

Hydra publishes a `.ls` file beside every narinfo on cache.nixos.org: the
complete listing of a store path's contents as JSON, with each entry's type,
size, executable bit and symlink target. That is the metadata half of a Nix
store, already served, at about nine kilobytes per path. omnibin crawls those
listings once — the only data this project adds to what [nixpkgs-multiverse]
already publishes — and keeps them in a database.

The filesystem then answers every question about what exists out of that
database, with no network at all:

```console
$ ls /nix/store/bm64j3i36fzaxb7yg2da7yvv29ndn4ar-python3-3.6.2/bin
2to3     idle      pydoc     python    python3.6         python3-config  pyvenv
2to3-3.6 idle3     pydoc3    python3   python3.6-config  python-config   pyvenv-3.6
         idle3.6   pydoc3.6            python3.6m        python3.6m-config
```

That path is not on the machine. Listing it cost nothing. Reading one byte out
of `python3.6` is what fetches the NAR, once, after which the path is served
from the unpacked copy like any other directory.

`/omnibin/bin` is the second filesystem, and it is the one people use.
`ls` shows the 35,940 bare names, one per executable anybody ever shipped,
each resolving to the newest package that provides it. The versioned forms resolve
too but are deliberately not listed — there are over a million of them, and a
directory nothing can read is worse than one that answers every question you
actually ask it. Ask the index instead:

```console
$ sqlite3 /omnibin/index.db \
    "SELECT attr, version FROM bins WHERE name = 'python3' ORDER BY version"
```

## What it costs

Measured, on a machine that had never seen CPython 3.6.2:

```console
$ time python3@3.6.2 -c 'import sys; print(sys.version.split()[0])'
3.6.2
real    0m2.690s

$ time python3@3.6.2 -c 'print(6*7)'
42
real    0m0.035s
```

The cold run fetched three store paths and 92 MB: the interpreter, its glibc,
and one more. The warm run touched the network zero times. That is the whole
trade — disk and bandwidth for exactly the packages that were used, instead of
an image sized for the packages somebody guessed would be.

## Status

Early, and honest about it.

The filesystem works and is tested against the real cache. The first full
crawl fetched **225,671 listings out of 233,197 store paths in 11.7 minutes**
— 6,801 the cache no longer holds, 725 whose published listing is corrupt
upstream, none that failed. That yields **35,940 bare executable names** over
350,084 `(name, attribute, version)` rows.

Those numbers are a floor. The index was joined against a multiverse artifact
cut two days older than its own outpaths file, so only 102,553 of 250,622
`(attribute, version)` pairs matched and the older end of history is thinner
than it will be. A matching cut is the next thing to run.

The data releases are not cut yet, so `data-pins.json` is empty and the
package takes `--db` until it is.

## License

MIT, please see [LICENSE](LICENSE).

The Nix expressions and tooling are original work. The published listings are
generated metadata about store paths — names, sizes and modes — crawled from
cache.nixos.org, not nixpkgs source.

[nixpkgs-multiverse]: https://github.com/fzakaria/nixpkgs-multiverse
