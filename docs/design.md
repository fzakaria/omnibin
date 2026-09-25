# Design

## The store is two halves

A Nix store path is metadata and bytes, and they have wildly different costs.
The metadata, being what files exist, how big they are, which are executable
and where the symlinks point, is a few kilobytes. The bytes are anything from 20 KB to
a gigabyte.

Almost everything a filesystem is asked is metadata. `stat`, `ls`, resolving a
symlink, checking whether a path exists: none of it reads content. So omnibin
splits the two and pays for them separately.

## The metadata is already published

Hydra writes a `.ls` next to every narinfo on cache.nixos.org. It is the whole
NAR listing as JSON:

```json
{"root":{"entries":{"bin":{"entries":{
  "python3":{"target":"python3.14","type":"symlink"},
  "python3.14":{"executable":true,"size":14264,"type":"regular"}, ...
```

Measured over a random sample of 400 indexed paths: about 9 KB compressed, 78
KB of JSON, 1,635 entries per path. It is served with
`Content-Encoding: br` for older objects and `zstd` for newer ones, and it
exists for exactly the paths the narinfo exists for. That is 233,694 of the
236,236 store paths the index names, with the rest gone from the cache.

That is the entire metadata layer of thirteen years of nixpkgs, already
hosted. Crawling all of it took 11.7 minutes and produced 3.7 GB.

## Does a NAR index help?

Partly, and less than it sounds, and the reason is worth writing down.

92.6% of published listings already carry a `narOffset` for every regular
file, being the byte position of that file's contents inside the uncompressed
NAR, measured over the same 400-path sample.
The index nixbuild.net builds for its own storage is, for most paths, already
public.

What is missing is not the index, it is a seekable NAR. cache.nixos.org serves
`nar/<hash>.nar.zst` and `nar/<hash>.nar.xz`, each a single compression
stream. A byte offset into the uncompressed NAR does not translate into a byte
range of the compressed one, so a `Range` request cannot fetch one file out of
a package. Indexing every NAR ourselves would not change that; it would
reproduce information the cache mostly publishes and still leave the bytes
unreachable.

Random access to individual files needs storage we control, whether re-hosted,
chunked, or stored uncompressed, and that is tens of terabytes of NARs. It is
a real project and it is not this one. omnibin fetches at path granularity,
which is the unit a closure is assembled from anyway.

## Would the file index fit in one SQLite database?

Three different questions, three different answers, all measured rather than
guessed.

| index                                 | rows             | fits in one file                                        |
| ------------------------------------- | ---------------- | ------------------------------------------------------- |
| `bin/` entries of indexed packages    | 350,084 measured | yes, tens of MB                                         |
| every file of every indexed path      | ~369 M estimated | technically; tens of GB, over GitHub's 2 GB asset limit |
| every file of the whole closure graph | ~3 B estimated   | no                                                      |

The first row is a count from the real crawl at its current coverage; the
other two are the sampled average of 1,635 entries per path multiplied out,
and are only meant to settle the order of magnitude.

So `omnibin.db` holds the first one. It is what `/omnibin/bin` answers from,
it is tens of megabytes, and it ships inside the package.

The full listings are kept as JSON Lines, one object per store path, and read
by digest. That is the shape they were crawled in, it shards cleanly, and a
consumer that wants one path's listing decompresses one record rather than
querying half a billion rows.

## Sharding, and why it never needs merging

A file listing describes a store path, and a store path is content-addressed.
The listing for a digest cannot change. Ever.

So shards are cut by crawl generation, not by content. Each run writes only
the digests nobody has published before, into files named for the day they
were crawled. Every earlier shard hashes identically to its pin and is never
uploaded again. There is no merge step, no cross-file invariant, and nothing
that can go stale. The union of every shard is the artifact, and later shards
never contradict earlier ones because there is nothing to contradict.

## Passthrough, and why the store mount is safe

Store paths are absolute. `/nix/store/...-python3-3.6.2/bin/python3` has that
path baked into its ELF interpreter and its RPATH, and so does every library
it loads. A lazy store is therefore only useful mounted at `/nix/store`, which
on a machine that already has one means hiding it.

omnibin does not hide it. The real store is bind-mounted aside first, and
every lookup checks it before the index. A path the host already has, whether
built locally, substituted from a private cache, or the omnibin binary itself,
is served from the real files and never fetched. The system keeps
working while its store is replaced underneath it.

On a workstation that happens inside a user and mount namespace that dies with
the shell, so nothing outside it ever sees the mount. On a VM or a container
it happens for the whole machine, because there it is the point.

## What the filesystem answers, and from where

| operation                                  | source                        | network                 |
| ------------------------------------------ | ----------------------------- | ----------------------- |
| `lookup` of a store path                   | `omnibin.db`                  | none                    |
| `getattr`, `readdir`, `readlink` inside it | the published listing, cached | one 9 KB fetch per path |
| the same, once fetched                     | the unpacked copy             | none                    |
| `read` of a file                           | the unpacked copy             | the path's NAR, once    |
| anything under `/omnibin/bin`              | `omnibin.db`                  | none                    |

`readdir` of `/nix/store` itself lists only what has been fetched. Enumerating
three hundred thousand packages would be an honest answer and a useless one.
The index is how you enumerate.
