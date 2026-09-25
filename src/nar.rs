//! Reading a NAR into a directory.
//!
//! The format is four cases and a framing rule: every token is a 64-bit
//! little-endian length followed by that many bytes, padded out to the next
//! multiple of eight. `nix-store --restore` would do this too, but shelling
//! out to Nix to unpack a path would make the filesystem depend on a Nix
//! installation it otherwise has no use for.

use anyhow::{bail, Result};
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::Path;

/// NAR pads every token out to this boundary.
const PAD: usize = 8;

/// The archive's opening token.
const MAGIC: &str = "nix-archive-1";

/// Refuse a token longer than this. Real NAR tokens are a type name, a field
/// name, a file name or a file body; a length beyond this is a corrupt stream,
/// and allocating on it would be the first thing to go wrong.
const MAX_TOKEN_BYTES: u64 = 64 * 1024 * 1024 * 1024;

/// Mode bits for a regular file, with and without the executable bit.
const MODE_REGULAR: u32 = 0o444;
const MODE_EXECUTABLE: u32 = 0o555;

struct Reader<R: Read> {
    inner: R,
}

impl<R: Read> Reader<R> {
    fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        self.inner.read_exact(buf)?;
        Ok(())
    }

    fn u64(&mut self) -> Result<u64> {
        let mut buf = [0u8; 8];
        self.read_exact(&mut buf)?;
        Ok(u64::from_le_bytes(buf))
    }

    /// Read one length-prefixed token and discard its padding.
    fn token(&mut self) -> Result<Vec<u8>> {
        let len = self.u64()?;
        if len > MAX_TOKEN_BYTES {
            bail!("nar: token of {len} bytes is not plausible");
        }

        let mut buf = vec![0u8; len as usize];
        self.read_exact(&mut buf)?;
        self.padding(len)?;
        Ok(buf)
    }

    fn string(&mut self) -> Result<String> {
        Ok(String::from_utf8(self.token()?)?)
    }

    fn padding(&mut self, len: u64) -> Result<()> {
        let extra = (PAD - (len as usize % PAD)) % PAD;
        if extra == 0 {
            return Ok(());
        }

        let mut buf = [0u8; PAD];
        self.read_exact(&mut buf[..extra])
    }

    /// Copy a file body straight through, rather than buffering it as a token.
    /// A NAR body can be a gigabyte and there is no reason for it to be in
    /// memory on the way to disk.
    fn copy_body(&mut self, len: u64, out: &mut fs::File) -> Result<()> {
        let mut remaining = len;
        let mut buf = vec![0u8; 1 << 20];
        while remaining > 0 {
            let want = remaining.min(buf.len() as u64) as usize;
            self.read_exact(&mut buf[..want])?;
            out.write_all(&buf[..want])?;
            remaining -= want as u64;
        }

        self.padding(len)
    }

    fn expect(&mut self, want: &str) -> Result<()> {
        let got = self.string()?;
        if got != want {
            bail!("nar: expected {want:?}, got {got:?}");
        }
        Ok(())
    }
}

/// Unpack a NAR stream into `dest`, which must not already exist.
pub fn unpack<R: Read>(stream: R, dest: &Path) -> Result<()> {
    let mut reader = Reader { inner: stream };
    reader.expect(MAGIC)?;
    node(&mut reader, dest)
}

/// Read one node at `dest`, being a regular file, a symlink or a directory.
fn node<R: Read>(r: &mut Reader<R>, dest: &Path) -> Result<()> {
    r.expect("(")?;
    r.expect("type")?;

    match r.string()?.as_str() {
        "regular" => regular(r, dest),
        "symlink" => {
            r.expect("target")?;
            let target = r.string()?;
            symlink(target, dest)?;
            r.expect(")")?;
            Ok(())
        }
        "directory" => directory(r, dest),
        other => bail!("nar: unknown node type {other:?}"),
    }
}

fn regular<R: Read>(r: &mut Reader<R>, dest: &Path) -> Result<()> {
    // The executable flag is an optional field carrying an empty value, so it
    // is only known after reading the next field name.
    let mut mode = MODE_REGULAR;
    let mut field = r.string()?;
    if field == "executable" {
        r.expect("")?;
        mode = MODE_EXECUTABLE;
        field = r.string()?;
    }

    if field != "contents" {
        bail!("nar: expected contents, got {field:?}");
    }

    let len = r.u64()?;
    let mut file = fs::File::create(dest)?;
    r.copy_body(len, &mut file)?;
    fs::set_permissions(dest, fs::Permissions::from_mode(mode))?;

    r.expect(")")
}

fn directory<R: Read>(r: &mut Reader<R>, dest: &Path) -> Result<()> {
    fs::create_dir(dest)?;

    loop {
        match r.string()?.as_str() {
            ")" => return Ok(()),
            "entry" => {}
            other => bail!("nar: expected entry or ), got {other:?}"),
        }

        r.expect("(")?;
        r.expect("name")?;
        let name = r.string()?;

        // A NAR entry name is one path component. Anything else is either a
        // corrupt archive or an attempt to write outside the destination.
        if name.is_empty() || name.contains('/') || name == "." || name == ".." {
            bail!("nar: unsafe entry name {name:?}");
        }

        r.expect("node")?;
        node(r, &dest.join(name))?;
        r.expect(")")?;
    }
}
