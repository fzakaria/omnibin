# omnibin

Every binary nixpkgs ever shipped, on your PATH. Nothing is installed, and a
package's bytes are fetched from cache.nixos.org the first time something
reads a file inside it.

```console
$ docker run --rm -it --device /dev/fuse --cap-add SYS_ADMIN fmzakari/omnibin
$ ls /omnibin/bin | wc -l
51468
$ python3@3.6.2 --version
Python 3.6.2
$ jq --version
jq-1.8.1
```

The image is 352 MB. The 51,468 commands it can run are not in it.

## What it needs

`--device /dev/fuse` and `--cap-add SYS_ADMIN`, and nothing else. `SYS_ADMIN`
is for `mount`, the device is for FUSE. Without them the container exits with
`omnibin: the mount never came up; is /dev/fuse present?` rather than starting
into a broken shell.

## How it starts

The entrypoint mounts before your command runs. It serves the lazy store, the
browsable tree at `/omnibin`, and the image's own store as a passthrough, then
binds the lazy store over `/nix/store` inside a mount namespace and execs what
you asked for. Everything that shipped in the image is read from the layers
and never fetched.

## As a base image

```dockerfile
# syntax=docker/dockerfile:1
FROM fmzakari/omnibin:latest

COPY <<'SH' /demo.sh
python3@3.6.2 -c 'import sys; print(sys.version.split()[0])'
jq --version
gcc@10.2.0 --version | head -1
SH

CMD ["bash", "/demo.sh"]
```

The packages are there when the container **runs**, not when it **builds**. A
`RUN` step in `docker build` has neither `/dev/fuse` nor the capability to
mount, so `RUN jq --version` fails exactly as it would on any base image
without jq. Put the work in `CMD` or `ENTRYPOINT`.

## Naming

```
/omnibin/bin/<name>            the newest package that provides <name>
/omnibin/bin/<name>@<version>  that command at that exact version
```

`ls /omnibin/bin` lists the bare names. The versioned forms resolve but are
not listed, because there are 881,933 of them. `omnibin which --all python3`
prints every version with what each costs to fetch, and `/omnibin/index.db` is
a plain SQLite file holding the whole index.

## Tags

`latest` tracks the newest data cut. Dated tags such as `data-20260925` pin
one, and the index inside a dated tag never changes.

## Caveats

Nothing is local: this is a window onto cache.nixos.org, not a copy of it.
Store paths reach back to 2012, but commands can only be found by name from
March 2017, which is when Hydra started publishing file listings. A cold run
of a large package is a download; every run after it is not.

Source, documentation and the browsable index:

- <https://github.com/fzakaria/omnibin>
- <https://omnibin.dev>
