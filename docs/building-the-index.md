# Building the index

Three steps, each re-runnable, none of them resumable on purpose. The whole
pipeline is about half an hour on a machine with bandwidth, and re-running it
is cheaper than any mechanism that would let it resume.

It runs wherever both this repository and a checkout of
[nixpkgs-multiverse](https://github.com/fzakaria/nixpkgs-multiverse) are,
because step three joins against the multiverse artifacts.

## 1. Crawl the listings

```console
$ nix develop
$ tools/crawl-listings.py \
    --outpaths ~/nixpkgs-multiverse/index/.outpaths/data/outpaths-x86_64-linux.json \
    --out data/listings.jsonl.zst --jobs 64
crawling 233197 listings with 96 workers
...
data/listings.jsonl.zst: 225671 listings, 6801 not in cache, 725 malformed, 0 unreachable, in 11.7m
```

One HTTPS connection per worker, held open; about 330 listings a second, and
3.7 GB of output. The digest count is lower than the multiverse version count
because a store path can be named by several `(attribute, version)` pairs and
a listing is per path: 250,622 pairs over 233,197 distinct digests.

Three things it records rather than hides:

- **not in cache**, meaning the digest has no `.ls`, which tracks the narinfo being
  gone. Written as `ok: false`, because absence is an answer and re-deriving
  it costs another request.
- **malformed**, meaning the published object decompresses to broken JSON. 725 of
  233,197, or 0.31%. This is a defect in the bytes on the cache, not in the
  decoder: `curl --compressed` reproduces the same gap, so the JSON is
  genuinely truncated mid-token on the way out of the brotli stream. Recorded
  the same way as absent; the filesystem falls back to fetching the path.
- **unreachable**, meaning nothing answered after four attempts. A non-zero count
  fails the run; re-crawl.

The output is JSON Lines, one object per store path:

```json
{"d":"3n4qphl9s728sz8frmpqqrv9b1m87g68","ok":true,"root":{"entries":{...}}}
```

## 2. Shard it

```console
$ tools/shard-listings.py --in data/listings.jsonl.zst \
    --out-dir data/listings --generation "$(date -u +%Y%m%d)"
```

Writes only digests no published shard already holds, into parts capped below
GitHub's 2 GB asset limit. A listing is content-addressed, so a shard that has
been published stays correct forever and is never rewritten. See
[design.md](./design.md#sharding-and-why-it-never-needs-merging).

## 3. Build the database

```console
$ tools/build-index.py \
    --listings data/listings.jsonl.zst \
    --outpaths   ~/nixpkgs-multiverse/index/.outpaths/data/outpaths-x86_64-linux.json \
    --info-indexed ~/nixpkgs-multiverse/index/.outpaths/data/info-indexed.json.gz \
    --versions   ~/nixpkgs-multiverse/index/versions.json \
    --revisions  ~/nixpkgs-multiverse/revisions.json \
    --system x86_64-linux --out data/omnibin-x86_64-linux.db
```

`info-indexed` is where the store path names, NAR URLs and sizes come from, so
the filesystem never has to fetch a narinfo at runtime. It already knows what
to GET and how big it is. `outpaths` is the `(attribute, version) → digest`
map. `versions` and `revisions` supply the last-seen date that decides which
package a bare name resolves to.

## 4. Cut the release

```console
$ tools/cut-data-release.sh
cut-data-release: data-20260924, uploading 5 of 5 artifacts
$ git diff --stat data-pins.json
```

Uploads only artifacts whose bytes differ from what `data-pins.json` already
pins, creates the dated tag on first use, and repoints the pins. Assets on a
dated tag are immutable by convention; the narHash in each pin fails closed if
that convention is ever broken.

Commit `data-pins.json`. That is the only file in the flake tree that knows
where the data lives.

## Running it on a big machine

The crawl is bandwidth and file descriptors, not CPU. On leviathan:

```console
$ nix-shell -p 'python3.withPackages (ps: with ps; [ brotli zstandard ])' zstd \
    --run 'tools/crawl-listings.py --outpaths … --out data/listings.jsonl.zst --jobs 128'
```

`gh` is not authenticated there, so cut the release from a laptop after an
rsync of `data/`.
