//! Getting bytes and listings out of a binary cache.
//!
//! Two things are fetched and they have very different costs. A file listing
//! is about nine kilobytes and describes a whole store path; a NAR is the
//! path itself and can be hundreds of megabytes. So listings are consulted
//! freely and NARs are fetched only when something reads a file's contents.
//!
//! Decompression is done by running `zstd`, `xz`, `bzip2` or `brotli` rather
//! than by linking four decompressors into this binary. The cache has used
//! all four across thirteen years, the Nix package puts them on PATH, and a
//! subprocess per store path is invisible next to the download it is decoding.

use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use crate::index::StorePath;
use crate::nar;

/// The public cache every indexed path came from.
pub const DEFAULT_CACHE_URL: &str = "https://cache.nixos.org";

/// Decompressor for each NAR suffix the cache has published over the years.
fn decompressor_for(url: &str) -> Result<&'static [&'static str]> {
    // Ordered longest-suffix-first is unnecessary here: the four suffixes are
    // distinct and none is a suffix of another.
    const BY_SUFFIX: &[(&str, &[&str])] = &[
        (".nar.zst", &["zstd", "-dc"]),
        (".nar.xz", &["xz", "-dc"]),
        (".nar.bz2", &["bzip2", "-dc"]),
        (".nar", &["cat"]),
    ];

    for (suffix, argv) in BY_SUFFIX {
        if url.ends_with(suffix) {
            return Ok(argv);
        }
    }

    bail!("no decompressor for {url}")
}

pub struct Fetcher {
    cache_url: String,
    /// Where unpacked store paths live, one directory per digest.
    store_dir: PathBuf,
    /// Where fetched `.ls` documents are kept, so a second look at a path
    /// costs nothing even when the listings artifact is not installed.
    listing_dir: PathBuf,
    /// One lock per digest, so two processes reading the same package at the
    /// same time download it once.
    in_flight: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl Fetcher {
    pub fn new(cache_dir: &Path, cache_url: &str) -> Result<Self> {
        let store_dir = cache_dir.join("store");
        let listing_dir = cache_dir.join("listings");
        fs::create_dir_all(&store_dir)?;
        fs::create_dir_all(&listing_dir)?;

        Ok(Self {
            cache_url: cache_url.trim_end_matches('/').to_string(),
            store_dir,
            listing_dir,
            in_flight: Mutex::new(HashMap::new()),
        })
    }

    /// Where a digest's unpacked contents are, fetching them if they are not
    /// there yet. This is the only call in omnibin that transfers a NAR.
    pub fn materialize(&self, path: &StorePath) -> Result<PathBuf> {
        let dest = self.store_dir.join(&path.digest);
        if dest.exists() {
            return Ok(dest);
        }

        // Take this digest's lock before checking again: whoever held it may
        // have been unpacking the very path being asked for.
        let lock = {
            let mut in_flight = self.in_flight.lock().unwrap();
            in_flight
                .entry(path.digest.clone())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let _held = lock.lock().unwrap();
        if dest.exists() {
            return Ok(dest);
        }

        // Unpack beside the destination and rename, so a killed process
        // leaves a stale temporary rather than a half-populated store path
        // that later looks complete.
        let staging = self.store_dir.join(format!(".{}.tmp", path.digest));
        if staging.exists() {
            fs::remove_dir_all(&staging)?;
        }

        self.unpack_nar(&path.nar_url, &staging)
            .with_context(|| format!("fetching {}", path.base_name()))?;
        fs::rename(&staging, &dest)?;

        Ok(dest)
    }

    fn unpack_nar(&self, nar_url: &str, dest: &Path) -> Result<()> {
        let url = format!("{}/{}", self.cache_url, nar_url);
        let response = ureq::get(&url)
            .call()
            .with_context(|| format!("GET {url}"))?;

        let argv = decompressor_for(nar_url)?;
        let mut child = Command::new(argv[0])
            .args(&argv[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .with_context(|| format!("spawning {}", argv[0]))?;

        // The body is pumped into the decompressor on another thread, because
        // this one has to be reading the NAR out of it at the same time or
        // both pipes deadlock on a path bigger than a pipe buffer.
        let mut body = response.into_reader();
        let mut stdin = child.stdin.take().expect("stdin was piped");
        let pump = std::thread::spawn(move || std::io::copy(&mut body, &mut stdin));

        let stdout = child.stdout.take().expect("stdout was piped");
        let unpacked = nar::unpack(stdout, dest);

        let copied = pump.join().expect("pump thread panicked");
        let status = child.wait()?;

        // The unpack error is the useful one — a decompressor that failed
        // usually did so because the transfer did — so it is reported first.
        unpacked?;
        copied.context("streaming the NAR into the decompressor")?;
        if !status.success() {
            bail!("{} exited with {status}", argv[0]);
        }

        Ok(())
    }

    /// Where a digest's contents would be, whether or not they are there yet.
    pub fn store_path_dir(&self, digest: &str) -> PathBuf {
        self.store_dir.join(digest)
    }

    /// Whether a digest has already been unpacked, which is what lets the
    /// filesystem answer from real files instead of from a listing.
    pub fn is_materialized(&self, digest: &str) -> bool {
        self.store_path_dir(digest).exists()
    }

    /// A store path's file listing, from the on-disk cache or the network.
    ///
    /// `None` means the cache has no listing for this digest, which happens
    /// for a small number of paths and for anything published without one.
    /// The caller falls back to materializing the path, which always works.
    pub fn listing(&self, digest: &str) -> Result<Option<Value>> {
        // The cache file holds the `.ls` document as published; the caller
        // wants its root node, the same thing the network path returns.
        let cached = self.listing_dir.join(format!("{digest}.json"));
        if cached.exists() {
            let bytes = fs::read(&cached)?;
            if bytes.is_empty() {
                return Ok(None);
            }
            let value: Value = serde_json::from_slice(&bytes)?;
            return Ok(value.get("root").cloned());
        }

        let url = format!("{}/{}.ls", self.cache_url, digest);
        let response = match ureq::get(&url).call() {
            Ok(response) => response,
            // A 404 is an answer, and remembering it as an empty file keeps a
            // path with no listing from being asked about again.
            Err(ureq::Error::Status(404, _)) => {
                fs::write(&cached, b"")?;
                return Ok(None);
            }
            Err(e) => return Err(e).context(format!("GET {url}")),
        };

        let encoding = response
            .header("Content-Encoding")
            .unwrap_or("identity")
            .to_string();
        let mut body = Vec::new();
        response.into_reader().read_to_end(&mut body)?;

        let json = decode_listing(&body, &encoding)?;

        // A malformed listing is a known defect in a handful of published
        // objects. Record it as absent rather than failing the lookup.
        let Ok(value) = serde_json::from_slice::<Value>(&json) else {
            fs::write(&cached, b"")?;
            return Ok(None);
        };

        fs::write(&cached, &json)?;
        Ok(value.get("root").cloned())
    }
}

/// Decompress a `.ls` body. The magic bytes are trusted over the declared
/// encoding, because the cache serves some objects as a zstd frame with the
/// header missing.
fn decode_listing(body: &[u8], encoding: &str) -> Result<Vec<u8>> {
    if body.starts_with(b"{") {
        return Ok(body.to_vec());
    }

    let argv: &[&str] = if body.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        &["zstd", "-dc"]
    } else if body.starts_with(&[0x1f, 0x8b]) {
        &["gzip", "-dc"]
    } else if encoding == "br" || encoding == "identity" {
        &["brotli", "-dc"]
    } else {
        bail!("unknown listing encoding {encoding:?}");
    };

    let mut child = Command::new(argv[0])
        .args(&argv[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;

    let mut stdin = child.stdin.take().expect("stdin was piped");
    let owned = body.to_vec();
    let pump = std::thread::spawn(move || std::io::Write::write_all(&mut stdin, &owned));

    let mut out = Vec::new();
    child
        .stdout
        .take()
        .expect("stdout was piped")
        .read_to_end(&mut out)?;
    pump.join().expect("pump thread panicked")?;
    child.wait()?;

    Ok(out)
}
