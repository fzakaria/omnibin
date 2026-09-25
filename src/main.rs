//! omnibin: every binary nixpkgs ever shipped, on your PATH.

mod fetch;
mod index;
mod nar;
mod storefs;
mod treefs;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use fuser::MountOption;
use std::path::PathBuf;
use std::sync::Arc;

use fetch::{Fetcher, DEFAULT_CACHE_URL};
use index::Index;
use storefs::StoreFs;
use treefs::TreeFs;

/// Where the Nix store has to appear. Store paths are absolute and baked into
/// every binary's interpreter and RPATH, so the lazy store is only useful at
/// this exact path.
const NIX_STORE: &str = "/nix/store";

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

    let mut options = vec![MountOption::RO, MountOption::FSName("omnibin".into())];

    // AutoUnmount is implemented by fusermount and it refuses to set it up
    // without allow_other, which in turn needs user_allow_other in
    // /etc/fuse.conf. The two therefore travel together, and without them a
    // killed process leaves a stale mount for `fusermount3 -u` to clear.
    if allow_other {
        options.push(MountOption::AllowOther);
        options.push(MountOption::AutoUnmount);
    }

    let store_fs = StoreFs::new(index.clone(), fetcher, passthrough);
    let store_session = fuser::spawn_mount2(store_fs, &store, &options)
        .with_context(|| format!("mounting {}", store.display()))?;

    let tree_fs = TreeFs::new(index.clone());
    let tree_session = fuser::spawn_mount2(tree_fs, &tree, &options)
        .with_context(|| format!("mounting {}", tree.display()))?;

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
    // lets the guards unmount on the way out.
    let (tx, rx) = std::sync::mpsc::channel();
    ctrl_c(tx)?;
    let _ = rx.recv();

    drop(tree_session);
    drop(store_session);
    Ok(())
}

/// Deliver one message when the process is interrupted.
fn ctrl_c(tx: std::sync::mpsc::Sender<()>) -> Result<()> {
    // A raw handler rather than a signal crate: the only thing it has to do is
    // wake the main thread up.
    static SENDER: std::sync::OnceLock<std::sync::Mutex<Option<std::sync::mpsc::Sender<()>>>> =
        std::sync::OnceLock::new();
    SENDER
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .unwrap()
        .replace(tx);

    extern "C" fn handler(_signal: libc::c_int) {
        if let Some(cell) = SENDER.get() {
            if let Ok(guard) = cell.lock() {
                if let Some(tx) = guard.as_ref() {
                    let _ = tx.send(());
                }
            }
        }
    }

    unsafe {
        libc::signal(libc::SIGINT, handler as libc::sighandler_t);
        libc::signal(libc::SIGTERM, handler as libc::sighandler_t);
    }

    Ok(())
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
    match Cli::parse().command {
        Command::Mount {
            store,
            tree,
            passthrough,
            cache_dir,
            cache_url,
            db,
            allow_other,
        } => mount(store, tree, passthrough, cache_dir, cache_url, db, allow_other),
        Command::Which { name, all } => which(std::env::var_os(DB_ENV).map(PathBuf::from), name, all),
    }
}
