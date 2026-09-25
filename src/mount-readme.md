# omnibin

Every executable nixpkgs ever shipped is in ./bin.

./bin/<name> the newest package that provides <name>
./bin/<name>@<version> that executable at that exact version

`ls ./bin` lists the bare names. The versioned forms are NOT listed, because
there are over eight hundred thousand of them, but they resolve:

./bin/python3@3.6.2
./bin/gcc@10.2.0

Do not walk this tree to find things. Ask instead, which costs no downloads:

omnibin which python3
omnibin which --all gcc

`which --all` prints every version with what it costs to fetch. For anything
those two do not answer, the index is a plain SQLite file:

sqlite3 /omnibin/index.db \
"SELECT attr, version FROM bins WHERE name = 'python3' ORDER BY version"

sqlite3 /omnibin/index.db \
"SELECT name FROM latest WHERE name LIKE 'gcc%'"

Tables: paths(digest, name, nar_url, nar_size), pkgs(attr, version, digest,
last_seen), bins(name, attr, version, digest), latest(name, attr, version,
digest).

Nothing is installed. A path is downloaded from cache.nixos.org the first
time something reads a file inside it, so the first run of a large package is
slow and every run after it is not.
