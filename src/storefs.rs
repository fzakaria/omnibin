//! The lazy `/nix/store`.
//!
//! Every store path the index knows about is present here. Looking at one —
//! `stat`, `ls`, following a symlink — is answered from the file listing and
//! costs no transfer. Reading a file's contents is what fetches the path's
//! NAR, once, after which the path is served from the unpacked copy like any
//! other directory.
//!
//! A host that already has a real `/nix/store` keeps it: paths present in the
//! passthrough directory are served from there and never fetched. That is
//! what makes it safe to mount this over the real store inside a mount
//! namespace — locally built paths, and paths from a private cache, still
//! resolve.

use fuser::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, ReplyOpen,
    Request,
};
use serde_json::Value;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use crate::fetch::Fetcher;
use crate::index::Index;

/// The root of any FUSE filesystem.
const ROOT_INO: u64 = 1;

/// Store paths are immutable, so the kernel may hold an answer for as long as
/// it likes. An hour is long enough to matter and short enough that a newly
/// installed listings database is picked up by a running mount.
const TTL: Duration = Duration::from_secs(3600);

/// Every file here is read-only, for everyone.
const DIR_MODE: u16 = 0o555;
const FILE_MODE: u16 = 0o444;
const EXEC_MODE: u16 = 0o555;
const LINK_MODE: u16 = 0o777;

/// Where a node's bytes come from.
#[derive(Clone)]
enum Source {
    /// A path the host already has; served from the real store, never fetched.
    Passthrough(PathBuf),
    /// A path in the index, fetched on first read.
    Lazy(String),
}

#[derive(Clone)]
struct Node {
    source: Source,
    /// Position inside the store path. Empty for the store path itself.
    rel: PathBuf,
}

/// What the filesystem needs to know about one entry, from whichever source
/// answered: a real `stat`, or the published listing.
struct Entry {
    kind: FileType,
    size: u64,
    executable: bool,
}

pub struct StoreFs {
    index: Arc<Index>,
    fetcher: Arc<Fetcher>,
    passthrough: Option<PathBuf>,
    uid: u32,
    gid: u32,

    nodes: HashMap<u64, Node>,
    by_name: HashMap<(u64, PathBuf), u64>,
    next_ino: u64,

    open_files: HashMap<u64, fs::File>,
    next_fh: u64,
}

impl StoreFs {
    pub fn new(
        index: Arc<Index>,
        fetcher: Arc<Fetcher>,
        passthrough: Option<PathBuf>,
    ) -> Self {
        Self {
            index,
            fetcher,
            passthrough,
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
            nodes: HashMap::new(),
            by_name: HashMap::new(),
            next_ino: ROOT_INO + 1,
            open_files: HashMap::new(),
            next_fh: 1,
        }
    }

    fn intern(&mut self, parent: u64, name: &OsStr, node: Node) -> u64 {
        let key = (parent, PathBuf::from(name));
        if let Some(ino) = self.by_name.get(&key) {
            return *ino;
        }

        let ino = self.next_ino;
        self.next_ino += 1;
        self.nodes.insert(ino, node);
        self.by_name.insert(key, ino);
        ino
    }

    /// The real filesystem path for a node, fetching the store path if the
    /// node is lazy and has not been fetched yet.
    fn real_path(&self, node: &Node) -> Option<PathBuf> {
        match &node.source {
            Source::Passthrough(base) => Some(base.join(&node.rel)),
            Source::Lazy(digest) => {
                let path = self.index.path_by_digest(digest).ok()??;
                let dir = self.fetcher.materialize(&path).ok()?;
                Some(dir.join(&node.rel))
            }
        }
    }

    /// The real path only if it is already on disk — no fetch.
    fn materialized_path(&self, node: &Node) -> Option<PathBuf> {
        match &node.source {
            Source::Passthrough(base) => Some(base.join(&node.rel)),
            Source::Lazy(digest) if self.fetcher.is_materialized(digest) => {
                Some(self.fetcher.store_path_dir(digest).join(&node.rel))
            }
            Source::Lazy(_) => None,
        }
    }

    /// The listing subtree at a node's position, if the cache published one.
    fn listing_at(&self, node: &Node) -> Option<Value> {
        let Source::Lazy(digest) = &node.source else {
            return None;
        };

        // The crawl already established which paths the cache published a
        // listing for. Asking the network about one it did not is a round trip
        // whose answer is known.
        if !self.index.path_by_digest(digest).ok()??.has_listing {
            return None;
        }

        let mut current = self.fetcher.listing(digest).ok()??;
        for component in node.rel.components() {
            let name = component.as_os_str().to_str()?;
            current = current.get("entries")?.get(name)?.clone();
        }

        Some(current)
    }

    /// What one node is, from the listing if there is one and from the
    /// unpacked copy otherwise.
    fn describe(&self, node: &Node) -> Option<Entry> {
        // A path already on disk answers from real files: it is faster than
        // parsing a listing, and it is the only correct source for a
        // passthrough path, which has no listing at all.
        if let Some(real) = self.materialized_path(node) {
            return Self::stat(&real);
        }

        if let Some(value) = self.listing_at(node) {
            return Some(Entry {
                kind: match value.get("type")?.as_str()? {
                    "directory" => FileType::Directory,
                    "symlink" => FileType::Symlink,
                    _ => FileType::RegularFile,
                },
                size: value.get("size").and_then(Value::as_u64).unwrap_or(0),
                executable: value
                    .get("executable")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            });
        }

        // No listing: this costs a fetch, which is the price of a path the
        // cache never published a listing for.
        Self::stat(&self.real_path(node)?)
    }

    /// Describe one real file on disk.
    fn stat(real: &Path) -> Option<Entry> {
        let meta = fs::symlink_metadata(real).ok()?;
        let kind = if meta.is_dir() {
            FileType::Directory
        } else if meta.file_type().is_symlink() {
            FileType::Symlink
        } else {
            FileType::RegularFile
        };

        Some(Entry {
            kind,
            size: meta.size(),
            executable: meta.mode() & 0o111 != 0,
        })
    }

    fn attr(&self, ino: u64, entry: &Entry) -> FileAttr {
        let (perm, nlink) = match entry.kind {
            FileType::Directory => (DIR_MODE, 2),
            FileType::Symlink => (LINK_MODE, 1),
            _ if entry.executable => (EXEC_MODE, 1),
            _ => (FILE_MODE, 1),
        };

        FileAttr {
            ino,
            size: entry.size,
            // Nix normalises every store path's mtime to one second past the
            // epoch, and so does this.
            blocks: entry.size.div_ceil(512),
            atime: UNIX_EPOCH + Duration::from_secs(1),
            mtime: UNIX_EPOCH + Duration::from_secs(1),
            ctime: UNIX_EPOCH + Duration::from_secs(1),
            crtime: UNIX_EPOCH + Duration::from_secs(1),
            kind: entry.kind,
            perm,
            nlink,
            uid: self.uid,
            gid: self.gid,
            rdev: 0,
            blksize: 4096,
            flags: 0,
        }
    }

    /// The entries of a directory node, listing-first.
    fn children(&self, node: &Node) -> Option<Vec<(String, FileType)>> {
        if let Some(real) = self.materialized_path(node) {
            return Self::read_dir(&real);
        }

        if let Some(value) = self.listing_at(node) {
            let entries = value.get("entries")?.as_object()?;
            return Some(
                entries
                    .iter()
                    .map(|(name, child)| {
                        let kind = match child.get("type").and_then(Value::as_str) {
                            Some("directory") => FileType::Directory,
                            Some("symlink") => FileType::Symlink,
                            _ => FileType::RegularFile,
                        };
                        (name.clone(), kind)
                    })
                    .collect(),
            );
        }

        Self::read_dir(&self.real_path(node)?)
    }

    /// List one real directory on disk.
    fn read_dir(real: &Path) -> Option<Vec<(String, FileType)>> {
        let mut out = Vec::new();
        for entry in fs::read_dir(real).ok()? {
            let entry = entry.ok()?;
            let kind = match entry.file_type().ok()? {
                t if t.is_dir() => FileType::Directory,
                t if t.is_symlink() => FileType::Symlink,
                _ => FileType::RegularFile,
            };
            out.push((entry.file_name().to_string_lossy().into_owned(), kind));
        }

        Some(out)
    }

    /// Resolve a name directly under the store root.
    fn store_root_child(&self, name: &OsStr) -> Option<Node> {
        let name_str = name.to_str()?;

        // The host's own store wins. A path built locally, or substituted from
        // a private cache, is already correct and must not be shadowed.
        if let Some(base) = &self.passthrough {
            let candidate = base.join(name_str);
            if candidate.exists() {
                return Some(Node {
                    source: Source::Passthrough(candidate),
                    rel: PathBuf::new(),
                });
            }
        }

        let path = self.index.path_by_base_name(name_str).ok()??;
        Some(Node {
            source: Source::Lazy(path.digest),
            rel: PathBuf::new(),
        })
    }
}

impl Filesystem for StoreFs {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let node = if parent == ROOT_INO {
            self.store_root_child(name)
        } else {
            self.nodes.get(&parent).map(|parent_node| Node {
                source: parent_node.source.clone(),
                rel: parent_node.rel.join(name),
            })
        };

        let Some(node) = node else {
            reply.error(libc::ENOENT);
            return;
        };

        let Some(entry) = self.describe(&node) else {
            reply.error(libc::ENOENT);
            return;
        };

        let ino = self.intern(parent, name, node);
        reply.entry(&TTL, &self.attr(ino, &entry), 0);
    }

    fn getattr(&mut self, _req: &Request, ino: u64, _fh: Option<u64>, reply: ReplyAttr) {
        if ino == ROOT_INO {
            let entry = Entry {
                kind: FileType::Directory,
                size: 0,
                executable: false,
            };
            reply.attr(&TTL, &self.attr(ROOT_INO, &entry));
            return;
        }

        let Some(node) = self.nodes.get(&ino).cloned() else {
            reply.error(libc::ENOENT);
            return;
        };

        match self.describe(&node) {
            Some(entry) => reply.attr(&TTL, &self.attr(ino, &entry)),
            None => reply.error(libc::ENOENT),
        }
    }

    fn readlink(&mut self, _req: &Request, ino: u64, reply: ReplyData) {
        let Some(node) = self.nodes.get(&ino).cloned() else {
            reply.error(libc::ENOENT);
            return;
        };

        // A symlink target is in the listing, so following one inside an
        // un-fetched package costs nothing.
        if let Some(value) = self.listing_at(&node) {
            if let Some(target) = value.get("target").and_then(Value::as_str) {
                reply.data(target.as_bytes());
                return;
            }
        }

        let Some(real) = self.real_path(&node) else {
            reply.error(libc::ENOENT);
            return;
        };

        match fs::read_link(&real) {
            Ok(target) => reply.data(target.as_os_str().as_bytes()),
            Err(_) => reply.error(libc::ENOENT),
        }
    }

    fn readdir(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        // The store root lists only what has been fetched. Enumerating three
        // hundred thousand packages would be an honest answer to `ls
        // /nix/store` and a useless one; the index is how you enumerate, and
        // /omnibin/bin is how you browse.
        let mut entries: Vec<(String, FileType)> = vec![
            (".".into(), FileType::Directory),
            ("..".into(), FileType::Directory),
        ];

        if ino == ROOT_INO {
            if let Some(base) = &self.passthrough {
                if let Ok(dir) = fs::read_dir(base) {
                    for entry in dir.flatten() {
                        entries.push((
                            entry.file_name().to_string_lossy().into_owned(),
                            FileType::Directory,
                        ));
                    }
                }
            }
        } else {
            let Some(node) = self.nodes.get(&ino).cloned() else {
                reply.error(libc::ENOENT);
                return;
            };

            let Some(children) = self.children(&node) else {
                reply.error(libc::ENOENT);
                return;
            };
            entries.extend(children);
        }

        for (i, (name, kind)) in entries.into_iter().enumerate().skip(offset as usize) {
            // The kernel stops asking when a reply buffer fills.
            if reply.add(ino, (i + 1) as i64, kind, &name) {
                break;
            }
        }

        reply.ok();
    }

    fn open(&mut self, _req: &Request, ino: u64, _flags: i32, reply: ReplyOpen) {
        let Some(node) = self.nodes.get(&ino).cloned() else {
            reply.error(libc::ENOENT);
            return;
        };

        // This is the moment a package is actually downloaded.
        let Some(real) = self.real_path(&node) else {
            reply.error(libc::EIO);
            return;
        };

        match fs::File::open(&real) {
            Ok(file) => {
                let fh = self.next_fh;
                self.next_fh += 1;
                self.open_files.insert(fh, file);
                reply.opened(fh, 0);
            }
            Err(_) => reply.error(libc::EIO),
        }
    }

    fn read(
        &mut self,
        _req: &Request,
        _ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock: Option<u64>,
        reply: ReplyData,
    ) {
        let Some(file) = self.open_files.get_mut(&fh) else {
            reply.error(libc::EBADF);
            return;
        };

        if file.seek(SeekFrom::Start(offset as u64)).is_err() {
            reply.error(libc::EIO);
            return;
        }

        let mut buf = vec![0u8; size as usize];
        let mut filled = 0;
        while filled < buf.len() {
            match file.read(&mut buf[filled..]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(_) => {
                    reply.error(libc::EIO);
                    return;
                }
            }
        }

        buf.truncate(filled);
        reply.data(&buf);
    }

    fn release(
        &mut self,
        _req: &Request,
        _ino: u64,
        fh: u64,
        _flags: i32,
        _lock: Option<u64>,
        _flush: bool,
        reply: fuser::ReplyEmpty,
    ) {
        self.open_files.remove(&fh);
        reply.ok();
    }
}
