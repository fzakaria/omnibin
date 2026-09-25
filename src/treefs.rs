//! `/omnibin`: the part a person or an agent reads.
//!
//! One flat directory of executables. `ls` shows the bare names, one per
//! executable anybody ever shipped, each resolving to the newest package that
//! provides it. The versioned forms resolve too but are deliberately not
//! listed: there are over a million of them, and a directory nobody can read
//! is worse than one that answers every question you actually ask it.
//!
//! Nothing here touches the network. Every answer is a row in omnibin.db.

use fuser::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, Request,
};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use crate::index::Index;

const ROOT_INO: u64 = 1;
const BIN_INO: u64 = 2;
const README_INO: u64 = 3;
const FIRST_DYNAMIC_INO: u64 = 4;

/// The index is a file on disk that does not change under a running mount.
const TTL: Duration = Duration::from_secs(3600);

const DIR_MODE: u16 = 0o555;
const FILE_MODE: u16 = 0o444;
const LINK_MODE: u16 = 0o777;

/// The separator between an executable's name and the version to pin it to.
const VERSION_SEPARATOR: char = '@';

/// Dropped in the mount root, because the first thing anything exploring this
/// filesystem should be told is not to explore it.
const README: &str = "\
# omnibin

Every executable nixpkgs ever shipped is in ./bin.

  ./bin/<name>            the newest package that provides <name>
  ./bin/<name>@<version>  that executable at that exact version

`ls ./bin` lists the bare names. The versioned forms are NOT listed, because
there are over eight hundred thousand, but they resolve:

  ./bin/python3@3.6.2
  ./bin/gcc@4.9.4

Do not walk this tree to find things. Query the database instead; it answers
in milliseconds and costs no downloads:

  sqlite3 /omnibin/index.db \\
    \"SELECT attr, version FROM bins WHERE name = 'python3' ORDER BY version\"

  sqlite3 /omnibin/index.db \\
    \"SELECT name FROM latest WHERE name LIKE 'gcc%'\"

Tables: paths(digest, name, nar_url, nar_size), pkgs(attr, version, digest,
last_seen), bins(name, attr, version, digest), latest(name, attr, version,
digest).

Nothing is installed. A path is downloaded from cache.nixos.org the first
time something reads a file inside it, so the first run of a large package is
slow and every run after it is not.
";

pub struct TreeFs {
    index: Arc<Index>,
    uid: u32,
    gid: u32,

    /// Symlink targets, by inode, for names that have been looked up.
    targets: HashMap<u64, String>,
    by_name: HashMap<String, u64>,
    next_ino: u64,

    /// Every bare name, read once. The kernel asks for a directory in reply
    /// buffer sized chunks, hundreds of calls for a directory this size, and
    /// re-running the query for each of them is quadratic in the number of
    /// names, which at 35,940 is slow enough to look like a hang.
    names: Option<Vec<String>>,
}

impl TreeFs {
    pub fn new(index: Arc<Index>) -> Self {
        Self {
            index,
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
            targets: HashMap::new(),
            by_name: HashMap::new(),
            next_ino: FIRST_DYNAMIC_INO,
            names: None,
        }
    }

    /// The store path an executable name resolves to, bare or versioned.
    fn resolve(&self, name: &str) -> Option<String> {
        let (bin_name, bin) = match name.split_once(VERSION_SEPARATOR) {
            Some((bin_name, version)) => (
                bin_name,
                self.index.bin_at_version(bin_name, version).ok()??,
            ),
            None => (name, self.index.latest_bin(name).ok()??),
        };

        let path = self.index.path_by_digest(&bin.digest).ok()??;
        Some(format!("/nix/store/{}/bin/{}", path.base_name(), bin_name))
    }

    fn attr(&self, ino: u64, kind: FileType, size: u64) -> FileAttr {
        let (perm, nlink) = match kind {
            FileType::Directory => (DIR_MODE, 2),
            FileType::Symlink => (LINK_MODE, 1),
            _ => (FILE_MODE, 1),
        };

        FileAttr {
            ino,
            size,
            blocks: size.div_ceil(512),
            atime: UNIX_EPOCH + Duration::from_secs(1),
            mtime: UNIX_EPOCH + Duration::from_secs(1),
            ctime: UNIX_EPOCH + Duration::from_secs(1),
            crtime: UNIX_EPOCH + Duration::from_secs(1),
            kind,
            perm,
            nlink,
            uid: self.uid,
            gid: self.gid,
            rdev: 0,
            blksize: 4096,
            flags: 0,
        }
    }
}

impl Filesystem for TreeFs {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let Some(name) = name.to_str() else {
            reply.error(libc::ENOENT);
            return;
        };

        if parent == ROOT_INO {
            match name {
                "bin" => reply.entry(&TTL, &self.attr(BIN_INO, FileType::Directory, 0), 0),
                "README.md" => reply.entry(
                    &TTL,
                    &self.attr(README_INO, FileType::RegularFile, README.len() as u64),
                    0,
                ),
                _ => reply.error(libc::ENOENT),
            }
            return;
        }

        if parent != BIN_INO {
            reply.error(libc::ENOENT);
            return;
        }

        let Some(target) = self.resolve(name) else {
            reply.error(libc::ENOENT);
            return;
        };

        let ino = *self.by_name.entry(name.to_string()).or_insert_with(|| {
            let ino = self.next_ino;
            self.next_ino += 1;
            ino
        });
        let size = target.len() as u64;
        self.targets.insert(ino, target);

        reply.entry(&TTL, &self.attr(ino, FileType::Symlink, size), 0);
    }

    fn getattr(&mut self, _req: &Request, ino: u64, _fh: Option<u64>, reply: ReplyAttr) {
        match ino {
            ROOT_INO => reply.attr(&TTL, &self.attr(ROOT_INO, FileType::Directory, 0)),
            BIN_INO => reply.attr(&TTL, &self.attr(BIN_INO, FileType::Directory, 0)),
            README_INO => reply.attr(
                &TTL,
                &self.attr(README_INO, FileType::RegularFile, README.len() as u64),
            ),
            _ => match self.targets.get(&ino) {
                Some(target) => {
                    let size = target.len() as u64;
                    reply.attr(&TTL, &self.attr(ino, FileType::Symlink, size))
                }
                None => reply.error(libc::ENOENT),
            },
        }
    }

    fn readlink(&mut self, _req: &Request, ino: u64, reply: ReplyData) {
        match self.targets.get(&ino) {
            Some(target) => reply.data(target.as_bytes()),
            None => reply.error(libc::ENOENT),
        }
    }

    fn read(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock: Option<u64>,
        reply: ReplyData,
    ) {
        if ino != README_INO {
            reply.error(libc::ENOENT);
            return;
        }

        let bytes = README.as_bytes();
        let start = (offset as usize).min(bytes.len());
        let end = (start + size as usize).min(bytes.len());
        reply.data(&bytes[start..end]);
    }

    fn readdir(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        let mut entries: Vec<(String, FileType)> = vec![
            (".".into(), FileType::Directory),
            ("..".into(), FileType::Directory),
        ];

        match ino {
            ROOT_INO => {
                entries.push(("bin".into(), FileType::Directory));
                entries.push(("README.md".into(), FileType::RegularFile));
            }
            BIN_INO => {
                // Bare names only. The versioned forms resolve on lookup and
                // are left out on purpose.
                if self.names.is_none() {
                    match self.index.bare_names() {
                        Ok(names) => self.names = Some(names),
                        Err(_) => {
                            reply.error(libc::EIO);
                            return;
                        }
                    }
                }

                let names = self.names.as_ref().expect("just filled");
                entries.extend(names.iter().map(|n| (n.clone(), FileType::Symlink)));
            }
            _ => {
                reply.error(libc::ENOTDIR);
                return;
            }
        }

        for (i, (name, kind)) in entries.into_iter().enumerate().skip(offset as usize) {
            if reply.add(ino, (i + 1) as i64, kind, &name) {
                break;
            }
        }

        reply.ok();
    }
}
