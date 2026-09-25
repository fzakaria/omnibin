# Using it

There are four ways in and they are the same binary. Which one you want
depends on how much of the machine you are willing to let it have.

|                 | mounts `/nix/store` for     | needs root            | survives the shell |
| --------------- | --------------------------- | --------------------- | ------------------ |
| `omnibin-shell` | this shell and its children | no                    | no                 |
| NixOS module    | the whole machine           | yes                   | yes                |
| VM              | the whole machine           | it is the machine     | yes                |
| container       | the whole container         | `--cap-add SYS_ADMIN` | yes                |

## omnibin-shell

```console
$ nix run github:fzakaria/omnibin
omnibin: tree at /run/user/1000/omnibin, cache at /home/you/.cache/omnibin
$ python3@3.6.2 --version
Python 3.6.2
$ exit
```

This is the one for a machine you care about. It starts the daemon, then
creates a user namespace and a mount namespace and binds the lazy store over
`/nix/store` inside them. Your real store is served through it, so everything
the machine already has keeps working. Exit and the mount is gone; the host
never saw it.

Two details worth knowing.

The daemon runs **outside** the namespace on purpose. omnibin's own libraries
live in `/nix/store`, and a process serving `/nix/store` from inside the
namespace can page-fault on itself, which deadlocks with no way out. Outside,
`/nix/store` is still the real one and nothing can recurse. The module does
mount from inside, as root, and calls `mlockall` to make that safe.

The tree is **not** at `/omnibin` here. Being root in a user namespace is not
being root on the host, and `/` belongs to real root, so the directory cannot
be created. It goes under `XDG_RUNTIME_DIR` and `$OMNIBIN_TREE` names it.
Override with `OMNIBIN_TREE=/somewhere/you/own`.

Run a single command instead of a shell by passing it:

```console
$ nix run github:fzakaria/omnibin -- python3@3.6.2 -c 'print(1)'
```

## The NixOS module

```nix
{
  imports = [ inputs.omnibin.nixosModules.default ];
  services.omnibin.enable = true;
}
```

This mounts the lazy store for the whole system and puts `/omnibin/bin` last
on `PATH`. It is what a VM or a container wants and it is not what a laptop
wants: it puts a FUSE process in the path of every binary the machine runs,
and if that process dies the machine has no `/nix/store` until systemd
restarts it.

The real store is bind-mounted to `/run/omnibin/real-store` before the lazy
one goes over it, and every lookup checks it first. That is what keeps the
machine bootable. The kernel, systemd and omnibin itself are served from the
files that were already there, and are never fetched.

Options:

| option                        | default              |                                         |
| ----------------------------- | -------------------- | --------------------------------------- |
| `services.omnibin.tree`       | `/omnibin`           | where the browsable tree goes           |
| `services.omnibin.mountStore` | `true`               | serve `/nix/store` lazily               |
| `services.omnibin.cacheDir`   | `/var/cache/omnibin` | fetched paths and listings              |
| `services.omnibin.addToPath`  | `true`               | append `${tree}/bin` to the system PATH |

`mountStore = false` serves the tree without touching `/nix/store`. The
symlinks in it are absolute, so they resolve only for packages the machine
already has. It is useful for reading the index, not for running anything new.

## The VM

```console
$ nix run github:fzakaria/omnibin#vm
```

The module, a root login, and nothing else. 4 GB of RAM and a 16 GB disk for
the fetch cache. This is the agent sandbox: a machine with every package on it
that took no time to build.

## The container

FUSE in a container needs the device and the capability. Nothing else.

```console
$ nix build github:fzakaria/omnibin#docker
$ docker load < result
$ docker run --rm -it --device /dev/fuse --cap-add SYS_ADMIN omnibin
```

There is no real store inside, so there is nothing to preserve and the lazy
store is mounted straight at `/nix/store` with no passthrough.

## Naming

```
/omnibin/bin/<name>            the newest package that provides <name>
/omnibin/bin/<name>@<version>  that executable at that exact version
```

`ls /omnibin/bin` lists the 35,940 bare names only. The versioned forms
resolve on lookup and are not listed, because there are 881,933 of them.

A bare name resolves by four rules, the first that separates two candidates
winning: an attribute named after the executable beats one that is not, a
newer last-seen date beats an older, a shorter attribute name beats a longer,
and the alphabet settles the rest. So `python3` comes from the `python3`
attribute rather than from `python3Full`, and a bare name resolves to the same
package for everybody.

The version is the package's, not the executable's. `bibtex@2023` is the
`bibtex` that shipped in TeX Live 2023.

## Asking the index

Do not walk the tree to find things. The database answers in milliseconds and
costs no downloads.

```console
$ omnibin which python3
/nix/store/gxzhl7aaiid7zp3y47jqqiq7zg5mqpwp-python3-3.14.6/bin/python3

$ omnibin which --all python3 | wc -l
416

$ omnibin which --all python3 | head -3
python3@3.11.7-env  jupyter  0.8 MB  /nix/store/zp0zk4…-python3-3.11.7-env/bin/python3
python3@3.11.9-env  jupyter  0.8 MB  /nix/store/hn4bkl…-python3-3.11.9-env/bin/python3
python3@3.12.4-env  jupyter  0.8 MB  /nix/store/xcvryj…-python3-3.12.4-env/bin/python3
```

416 packages have shipped something called `python3`, and most of them are not
CPython. A `jupyter` environment ships one too, which is why `@version` is a
version of the package rather than of the interpreter. The size is what
running that one costs the first time.

The database is at `/omnibin/index.db` inside a mount, `$OMNIBIN_TREE/index.db`
under `omnibin-shell`, and `$OMNIBIN_DB` everywhere else:

```sql
-- which packages ever shipped an `rg`
SELECT attr, version FROM bins WHERE name = 'rg' ORDER BY version;

-- every executable whose name starts with gcc
SELECT name FROM latest WHERE name LIKE 'gcc%';

-- what a package costs to run
SELECT p.name, p.nar_size FROM latest l JOIN paths p USING (digest)
 WHERE l.name = 'ffmpeg';
```

Tables: `paths(digest, name, nar_url, nar_size, has_listing)`,
`pkgs(attr, version, digest, last_seen)`,
`bins(name, attr, version, digest)`, `latest(name, attr, version, digest)`.

## For an agent

Put this in the agent's instructions, or let it read the `README.md` in the
mount, which says the same thing:

> Every executable nixpkgs ever shipped is in `/omnibin/bin`. Do not install
> anything and do not walk the tree. To find a command, query
> `/omnibin/index.db`. To pin a version, use `<name>@<version>`.
