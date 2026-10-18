//! omnibin: every binary nixpkgs ever shipped, on your PATH.

mod fetch;
mod index;
mod nar;
mod storefs;
mod treefs;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use fuser::{Filesystem, MountOption, Session};
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{mpsc, Arc};

use fetch::{Fetcher, DEFAULT_CACHE_URL};
use index::Index;
use storefs::StoreFs;
use treefs::TreeFs;

/// Where the Nix store has to appear. Store paths are absolute and baked into
/// every binary's interpreter and RPATH, so the lazy store is only useful at
/// this exact path.
const NIX_STORE: &str = "/nix/store";

/// Where the lazy store is mounted before being bound over /nix/store.
///
/// libfuse execs `mount` and `fusermount3` out of /nix/store before it
/// returns, and nothing serves the new mount until it does. Mounted directly
/// at /nix/store, those execs hang forever.
const STAGED_STORE: &str = "/run/omnibin/store";

/// Where the browsable tree is mounted by default.
const DEFAULT_TREE: &str = "/omnibin";

/// Set by the Nix wrapper so the database version is the package version.
const DB_ENV: &str = "OMNIBIN_DB";

#[derive(Parser)]
#[command(name = "omnibin", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Mount the lazy store and the browsable tree.
    Mount {
        /// Where /nix/store is served. Mounting this over a real store is
        /// safe only inside a mount namespace, and only with --passthrough.
        #[arg(long, default_value = NIX_STORE)]
        store: PathBuf,

        /// Where the browsable tree is served.
        #[arg(long, default_value = DEFAULT_TREE)]
        tree: PathBuf,

        /// A bind mount of the host's real store, served in preference to
        /// anything in the index so locally built paths keep resolving.
        #[arg(long)]
        passthrough: Option<PathBuf>,

        /// Where fetched paths and listings are kept between runs.
        #[arg(long)]
        cache_dir: Option<PathBuf>,

        #[arg(long, default_value = DEFAULT_CACHE_URL)]
        cache_url: String,

        #[arg(long, env = DB_ENV)]
        db: Option<PathBuf>,

        /// Let other users read the mounts. Needs user_allow_other in
        /// /etc/fuse.conf.
        #[arg(long)]
        allow_other: bool,
    },

    /// Print what an executable name resolves to, and every version of it.
    Which {
        name: String,

        /// Print every package that ever shipped this executable.
        #[arg(long)]
        all: bool,
    },
}

fn default_cache_dir() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("XDG_CACHE_HOME") {
        return Ok(PathBuf::from(dir).join("omnibin"));
    }

    let home = std::env::var("HOME").context("neither XDG_CACHE_HOME nor HOME is set")?;
    Ok(PathBuf::from(home).join(".cache/omnibin"))
}

fn open_index(db: Option<PathBuf>) -> Result<Arc<Index>> {
    let Some(path) = db else {
        bail!("no database: pass --db or set {DB_ENV}");
    };

    let index = Index::open(&path)?;
    Ok(Arc::new(index))
}

/// Clear a mountpoint left behind by a daemon that was killed outright.
///
/// A process stopped with SIGKILL cannot unmount itself, and what it leaves
/// is a mountpoint whose every syscall fails with ENOTCONN. Mounting over one
/// fails, and so does looking at it, so the next run would refuse to start
/// for a reason that reads like a permissions problem. Only ENOTCONN is
/// treated this way: any other error is somebody else's mount or a real
/// problem, and unmounting it would be overreach.
fn clear_stale_mount(path: &Path) {
    let Err(err) = std::fs::metadata(path) else {
        return;
    };
    if err.raw_os_error() != Some(libc::ENOTCONN) {
        return;
    }

    eprintln!("omnibin: clearing a stale mount at {}", path.display());
    let _ = std::process::Command::new("fusermount3")
        .arg("-u")
        .arg(path)
        .status();
}

fn mount(
    store: PathBuf,
    tree: PathBuf,
    passthrough: Option<PathBuf>,
    cache_dir: Option<PathBuf>,
    cache_url: String,
    db: Option<PathBuf>,
    allow_other: bool,
) -> Result<()> {
    let index = open_index(db)?;
    let cache_dir = match cache_dir {
        Some(dir) => dir,
        None => default_cache_dir()?,
    };
    let fetcher = Arc::new(Fetcher::new(&cache_dir, &cache_url)?);

    // Mounting the lazy store over a real one without passthrough would hide
    // every path the host already has, including the one this process is
    // running from. Refuse rather than deadlock.
    if store == PathBuf::from(NIX_STORE) && passthrough.is_none() && store.exists() {
        bail!(
            "refusing to mount over {NIX_STORE} without --passthrough; \
             bind-mount the real store somewhere first"
        );
    }

    // Over /nix/store, mount at STAGED_STORE and bind once served.
    let bind_over = (store == Path::new(NIX_STORE)).then(|| store.clone());
    let fuse_store = match &bind_over {
        Some(_) => PathBuf::from(STAGED_STORE),
        None => store.clone(),
    };
    if bind_over.is_some() {
        std::fs::create_dir_all(&fuse_store)
            .with_context(|| format!("creating {}", fuse_store.display()))?;
    }

    for path in [&fuse_store, &tree] {
        clear_stale_mount(path);
    }

    let mut options = vec![MountOption::RO, MountOption::FSName("omnibin".into())];

    // AutoUnmount is implemented by fusermount and it refuses to set it up
    // without allow_other, which in turn needs user_allow_other in
    // /etc/fuse.conf. The two therefore travel together, and without them a
    // killed process leaves a stale mount for `fusermount3 -u` to clear.
    if allow_other {
        options.push(MountOption::AllowOther);
        options.push(MountOption::AutoUnmount);
    }

    // Serving /nix/store means this process's own libraries are behind the
    // mount it is servicing. A page fault on one of them would be answered by
    // a thread that is blocked on that same fault, which is a deadlock with no
    // way out. Pinning every page, the ones mapped now and the ones mapped
    // later, is what makes it safe to mount over a live store.
    if store == PathBuf::from(NIX_STORE) {
        lock_memory()?;
    }

    // Mounted while /nix/store is still the real store. See STAGED_STORE.
    let store_fs = StoreFs::new(index.clone(), fetcher, passthrough);
    let mut store_session = Session::new(store_fs, &fuse_store, &options)
        .with_context(|| format!("mounting {}", fuse_store.display()))?;

    let tree_fs = TreeFs::new(index.clone());
    let mut tree_session = Session::new(tree_fs, &tree, &options)
        .with_context(|| format!("mounting {}", tree.display()))?;

    // Unmounted from this thread, which stays in the machine's namespace.
    let mut store_unmounter = store_session.unmount_callable();
    let mut tree_unmounter = tree_session.unmount_callable();

    let namespace = match &bind_over {
        Some(_) => ServingNamespace::PrivateRealStore,
        None => ServingNamespace::Inherited,
    };
    serve(store_session, namespace)?;
    serve(tree_session, namespace)?;

    // Bind only after both serving threads have their private namespace.
    if let Some(target) = &bind_over {
        bind_mount(&fuse_store, target)?;
    }

    let names = index.meta("names")?.unwrap_or_else(|| "?".into());
    let paths = index.meta("paths")?.unwrap_or_else(|| "?".into());
    println!(
        "omnibin: {} executables over {} store paths\n  {} -> lazy store\n  {} -> browsable tree",
        names,
        paths,
        store.display(),
        tree.display()
    );

    // Both sessions run on their own threads; this one waits for a signal and
    // then unmounts them. Unmounting is the only cleanup there is: a process
    // killed with SIGKILL cannot unmount itself, which is what the stale-mount
    // check at startup exists to recover from.
    wait_for_signal()?;

    // Drop the bind first, so new processes see the real store. Programs
    // already running from the lazy store get SIGBUS once this process exits.
    if let Some(target) = &bind_over {
        detach_mount(target);
    }

    // The serving threads are not joined. A private namespace holds a copy
    // of each mount, so a session loop never sees it go away; process exit
    // ends the threads.
    tree_unmounter.unmount()?;
    store_unmounter.unmount()?;
    Ok(())
}

/// Which mount namespace a session's serving thread works in.
#[derive(Clone, Copy)]
enum ServingNamespace {
    /// The process's own. Used when /nix/store is not being replaced.
    Inherited,

    /// A private copy taken before the bind over /nix/store, so the serving
    /// thread keeps the real store. A fetch loads NSS modules and runs a
    /// decompressor from /nix/store, and the serving thread cannot answer its
    /// own lookups.
    PrivateRealStore,
}

/// Run a session's loop on a thread of its own, in the given namespace.
///
/// Returns once the thread has entered the namespace, so later mounts are
/// hidden from the thread.
fn serve<FS: Filesystem + Send + 'static>(
    mut session: Session<FS>,
    namespace: ServingNamespace,
) -> Result<()> {
    let (entered_tx, entered_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let entered = match namespace {
            ServingNamespace::Inherited => Ok(()),
            ServingNamespace::PrivateRealStore => enter_private_namespace(),
        };

        let failed = entered.is_err();
        // The caller blocks on recv, so the send cannot fail.
        let _ = entered_tx.send(entered);
        if failed {
            return Ok(());
        }

        session.run()
    });

    entered_rx
        .recv()
        .context("the serving thread exited before entering its namespace")?
        .context("entering a private mount namespace for the store")?;

    Ok(())
}

/// Move the calling thread alone into a private copy of its mount namespace.
///
/// Made private recursively, since the copied mounts start as peers of the
/// originals and would receive the bind over /nix/store.
fn enter_private_namespace() -> std::io::Result<()> {
    if unsafe { libc::unshare(libc::CLONE_NEWNS) } != 0 {
        return Err(std::io::Error::last_os_error());
    }

    let made_private = unsafe {
        libc::mount(
            std::ptr::null(),
            c"/".as_ptr(),
            std::ptr::null(),
            libc::MS_REC | libc::MS_PRIVATE,
            std::ptr::null(),
        )
    };
    if made_private != 0 {
        return Err(std::io::Error::last_os_error());
    }

    Ok(())
}

/// Bind an existing mount over another path.
fn bind_mount(source: &Path, target: &Path) -> Result<()> {
    let source_c = CString::new(source.as_os_str().as_bytes())?;
    let target_c = CString::new(target.as_os_str().as_bytes())?;

    let bound = unsafe {
        libc::mount(
            source_c.as_ptr(),
            target_c.as_ptr(),
            std::ptr::null(),
            libc::MS_BIND,
            std::ptr::null(),
        )
    };
    if bound != 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("binding {} over {}", source.display(), target.display()));
    }

    Ok(())
}

/// Lazily detach a mount. Only called on shutdown, so errors are ignored.
fn detach_mount(target: &Path) {
    let Ok(target_c) = CString::new(target.as_os_str().as_bytes()) else {
        return;
    };
    unsafe {
        libc::umount2(target_c.as_ptr(), libc::MNT_DETACH);
    }
}

/// Pin this process's pages in RAM.
///
/// Needs CAP_IPC_LOCK, which a mapped-root user namespace has, or an
/// RLIMIT_MEMLOCK large enough. Failing is fatal rather than a warning: a
/// daemon that can fault on its own text while serving /nix/store will hang
/// the namespace the first time it does, and hanging later is worse than
/// refusing now.
fn lock_memory() -> Result<()> {
    let locked = unsafe { libc::mlockall(libc::MCL_CURRENT | libc::MCL_FUTURE) };
    if locked != 0 {
        let err = std::io::Error::last_os_error();
        bail!(
            "could not lock memory ({err}); serving {NIX_STORE} needs it, \
             because a page fault in this process would deadlock on itself. \
             Raise RLIMIT_MEMLOCK, or mount somewhere other than {NIX_STORE}."
        );
    }

    Ok(())
}

/// The write end of a pipe the signal handler pokes.
///
/// A handler may only call async-signal-safe functions. Sending on a channel
/// is not one: it allocates and takes locks, so a signal delivered to a
/// worker thread already inside the allocator deadlocks the handler there and
/// the process never learns it was asked to stop. That is exactly what
/// happened here, and why a killed mount used to be left behind. write(2) is
/// safe, so the handler writes a byte and the main thread reads it.
static SIGNAL_WRITE_FD: AtomicI32 = AtomicI32::new(-1);

extern "C" fn on_signal(_signal: libc::c_int) {
    let fd = SIGNAL_WRITE_FD.load(Ordering::Relaxed);
    if fd < 0 {
        return;
    }

    let byte: u8 = 1;
    // Nothing useful to do with the result inside a handler, and the caller
    // wakes on any byte.
    unsafe {
        libc::write(fd, std::ptr::addr_of!(byte).cast(), 1);
    }
}

/// Block until the process is asked to stop.
fn wait_for_signal() -> Result<()> {
    let mut fds = [0 as libc::c_int; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error()).context("creating the signal pipe");
    }
    let (read_fd, write_fd) = (fds[0], fds[1]);
    SIGNAL_WRITE_FD.store(write_fd, Ordering::Relaxed);

    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        // Through a pointer rather than straight to an integer: casting a
        // function item to a usize is what `function_casts_as_integer` warns
        // about, and the two-step spelling is the one it asks for.
        let handler = on_signal as *const () as libc::sighandler_t;
        if unsafe { libc::signal(signal, handler) } == libc::SIG_ERR {
            return Err(std::io::Error::last_os_error())
                .context(format!("installing a handler for signal {signal}"));
        }
    }

    // One byte is the whole message. A read interrupted by a signal that is
    // not one of ours is resumed rather than treated as a shutdown.
    let mut byte = [0u8; 1];
    loop {
        let read = unsafe { libc::read(read_fd, byte.as_mut_ptr().cast(), 1) };
        if read == 1 {
            return Ok(());
        }

        let err = std::io::Error::last_os_error();
        if read < 0 && err.kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        return Err(err).context("waiting on the signal pipe");
    }
}

fn which(db: Option<PathBuf>, name: String, all: bool) -> Result<()> {
    let index = open_index(db)?;

    if all {
        for bin in index.bins_named(&name)? {
            let Some(path) = index.path_by_digest(&bin.digest)? else {
                continue;
            };
            let size = match path.nar_size {
                Some(bytes) => format!("{:.1} MB", bytes as f64 / 1e6),
                None => "?".to_string(),
            };
            println!(
                "{}@{}\t{}\t{}\t/nix/store/{}/bin/{}",
                name,
                bin.version,
                bin.attr,
                size,
                path.base_name(),
                name
            );
        }
        return Ok(());
    }

    let Some(bin) = index.latest_bin(&name)? else {
        bail!("{name}: no package ever shipped an executable by that name");
    };
    let Some(path) = index.path_by_digest(&bin.digest)? else {
        bail!("{name}: resolves to a store path the index does not hold");
    };

    println!("/nix/store/{}/bin/{}", path.base_name(), name);
    Ok(())
}

fn main() -> Result<()> {
    // Rust ignores SIGPIPE so that a failed write surfaces as an error, which
    // for a command whose output is meant to be piped into `head` means a
    // panic instead of a quiet exit. Restore the default.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    match Cli::parse().command {
        Command::Mount {
            store,
            tree,
            passthrough,
            cache_dir,
            cache_url,
            db,
            allow_other,
        } => mount(
            store,
            tree,
            passthrough,
            cache_dir,
            cache_url,
            db,
            allow_other,
        ),
        Command::Which { name, all } => {
            which(std::env::var_os(DB_ENV).map(PathBuf::from), name, all)
        }
    }
}
