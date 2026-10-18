//! Reading omnibin.db.
//!
//! Every answer the filesystem gives about what exists comes from here, and
//! none of them touch the network. Looking at the store is free; only reading
//! a file's bytes costs a fetch.

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::Mutex;

/// A store path omnibin knows how to fetch.
#[derive(Clone, Debug)]
pub struct StorePath {
    pub digest: String,
    pub name: String,
    pub nar_url: String,
    pub nar_size: Option<u64>,
    pub has_listing: bool,
}

impl StorePath {
    /// The basename Nix would give this path: `<digest>-<name>`.
    pub fn base_name(&self) -> String {
        format!("{}-{}", self.digest, self.name)
    }
}

/// One executable that some package's bin/ holds.
#[derive(Clone, Debug)]
pub struct Bin {
    pub attr: String,
    pub version: String,
    pub digest: String,
}

pub struct Index {
    /// One connection behind a lock. Both filesystems run on their own FUSE
    /// threads and a rusqlite Connection is not Sync; every query here is a
    /// point lookup on an indexed column, so contention never shows up.
    db: Mutex<Connection>,
}

/// A Nix store path digest is this many base-32 characters.
pub const DIGEST_LEN: usize = 32;

/// Bytes an SQLite URI filename cannot carry literally: the escape itself,
/// and the two that would end the path and start a query or a fragment.
const URI_RESERVED: &[u8] = b"%?#";

/// The database path as a percent-encoded SQLite URI with `immutable=1`.
///
/// Without immutable, SQLite stats `<db>-journal` before every read. The
/// database is in /nix/store, so with the lazy store mounted there omnibin
/// would be answering its own lookup while holding the connection lock.
fn immutable_uri(path: &Path) -> String {
    let mut uri = String::from("file:");
    for &byte in path.as_os_str().as_bytes() {
        // Non-ASCII bytes are escaped too, so a path that is not UTF-8 still
        // makes a valid URI and SQLite decodes it back to the same bytes.
        if URI_RESERVED.contains(&byte) || !byte.is_ascii() {
            uri.push_str(&format!("%{byte:02X}"));
            continue;
        }
        uri.push(byte as char);
    }

    uri.push_str("?immutable=1");
    uri
}

impl Index {
    pub fn open(path: &Path) -> Result<Self> {
        let flags =
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI;
        let db = Connection::open_with_flags(immutable_uri(path), flags)
            .with_context(|| format!("opening {}", path.display()))?;
        Ok(Self { db: Mutex::new(db) })
    }

    /// The store path a digest names.
    ///
    /// Only the digest is ever matched. The name after it is what the cache
    /// says the path is called, and a caller that got it wrong, from a stale
    /// symlink or a typo, still reaches the right bytes: in a content
    /// addressed store the digest is the whole identity.
    pub fn path_by_digest(&self, digest: &str) -> Result<Option<StorePath>> {
        let db = self.db.lock().unwrap();
        let row = db
            .query_row(
                "SELECT digest, name, nar_url, nar_size, has_listing
                   FROM paths WHERE digest = ?1",
                [digest],
                |r| {
                    Ok(StorePath {
                        digest: r.get(0)?,
                        name: r.get(1)?,
                        nar_url: r.get(2)?,
                        nar_size: r.get(3)?,
                        has_listing: r.get::<_, i64>(4)? != 0,
                    })
                },
            )
            .optional()?;

        Ok(row)
    }

    /// What a bare executable name on PATH resolves to.
    pub fn latest_bin(&self, name: &str) -> Result<Option<Bin>> {
        let db = self.db.lock().unwrap();
        let row = db
            .query_row(
                "SELECT attr, version, digest FROM latest WHERE name = ?1",
                [name],
                |r| {
                    Ok(Bin {
                        attr: r.get(0)?,
                        version: r.get(1)?,
                        digest: r.get(2)?,
                    })
                },
            )
            .optional()?;

        Ok(row)
    }

    /// What `<name>@<version>` resolves to.
    ///
    /// Two packages can ship the same executable at the same version, and
    /// `curl` and `curlWithGnuTls` both have a `curl` 8.10.1, so the same
    /// precedence the bare name uses settles it: the attribute named after the
    /// executable first, then the shorter attribute, then the alphabet.
    pub fn bin_at_version(&self, name: &str, version: &str) -> Result<Option<Bin>> {
        let db = self.db.lock().unwrap();
        let row = db
            .query_row(
                "SELECT attr, version, digest FROM bins
                  WHERE name = ?1 AND version = ?2
                  ORDER BY (attr = ?1) DESC, length(attr), attr
                  LIMIT 1",
                [name, version],
                |r| {
                    Ok(Bin {
                        attr: r.get(0)?,
                        version: r.get(1)?,
                        digest: r.get(2)?,
                    })
                },
            )
            .optional()?;

        Ok(row)
    }

    /// Every bare executable name, for `readdir` of the bin directory.
    pub fn bare_names(&self) -> Result<Vec<String>> {
        let db = self.db.lock().unwrap();
        let mut stmt = db.prepare("SELECT name FROM latest ORDER BY name")?;
        let names = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(names)
    }

    /// Every package that ever shipped an executable of this name.
    pub fn bins_named(&self, name: &str) -> Result<Vec<Bin>> {
        let db = self.db.lock().unwrap();
        let mut stmt = db.prepare(
            "SELECT attr, version, digest FROM bins WHERE name = ?1 ORDER BY attr, version",
        )?;
        let bins = stmt
            .query_map([name], |r| {
                Ok(Bin {
                    attr: r.get(0)?,
                    version: r.get(1)?,
                    digest: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(bins)
    }

    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        let db = self.db.lock().unwrap();
        Ok(db
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }
}

/// Tests that `Index::open` escapes the path in its URI, by writing a
/// database under a name SQLite would misread unescaped and reading it back.
#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::path::PathBuf;

    /// A directory of its own under the system temp dir, removed on drop.
    struct TempDir(PathBuf);

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A name with `%`, `?`, `#`, a space and a non-UTF-8 byte round-trips.
    #[test]
    fn opens_a_path_that_needs_escaping() {
        let dir = TempDir(
            std::env::temp_dir().join(format!("omnibin-index-test-{}", std::process::id())),
        );
        std::fs::create_dir_all(&dir.0).unwrap();

        let name = OsStr::from_bytes(b"in dex%41?x=1#frag\xff.db");
        let path = dir.0.join(name);
        let writer = Connection::open(&path).unwrap();
        writer
            .execute_batch(
                "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);
                 INSERT INTO meta VALUES ('names', '42');",
            )
            .unwrap();
        drop(writer);

        let index = Index::open(&path).unwrap();
        assert_eq!(index.meta("names").unwrap().as_deref(), Some("42"));
    }
}
