//! Checksums: the one a download is expected to have, and working one out for a file.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::io::Read;
use std::path::Path;

/// The hashes download pages publish.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Algo {
    Md5,
    Sha1,
    Sha256,
    Sha512,
}

impl Algo {
    pub fn name(self) -> &'static str {
        match self {
            Algo::Md5 => "md5",
            Algo::Sha1 => "sha1",
            Algo::Sha256 => "sha256",
            Algo::Sha512 => "sha512",
        }
    }

    pub fn from_name(s: &str) -> Option<Algo> {
        match s.trim().to_ascii_lowercase().replace('-', "").as_str() {
            "md5" => Some(Algo::Md5),
            "sha1" => Some(Algo::Sha1),
            "sha256" => Some(Algo::Sha256),
            "sha512" => Some(Algo::Sha512),
            _ => None,
        }
    }

    /// Hex digits in one of its values.
    fn hex_len(self) -> usize {
        match self {
            Algo::Md5 => 32,
            Algo::Sha1 => 40,
            Algo::Sha256 => 64,
            Algo::Sha512 => 128,
        }
    }

    /// Which hash a value of this many hex digits is, for a value given without its name.
    fn of_len(n: usize) -> Option<Algo> {
        [Algo::Md5, Algo::Sha1, Algo::Sha256, Algo::Sha512].into_iter().find(|a| a.hex_len() == n)
    }
}

/// The checksum a download should have.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checksum {
    pub algo: Algo,
    /// Lower-case hex.
    pub value: String,
}

impl Checksum {
    /// `sha256:9f86…`, or a bare value whose length says which hash it is.
    pub fn parse(s: &str) -> Result<Checksum> {
        let s = s.trim();
        let (algo, value) = match s.split_once([':', '=']) {
            Some((a, v)) => (Algo::from_name(a).ok_or_else(|| anyhow::anyhow!("{a:?} is not a checksum reIDM knows: use md5, sha1, sha256 or sha512"))?, v),
            None => (Algo::of_len(s.len()).ok_or_else(|| anyhow::anyhow!("say which checksum this is, like sha256:{s}"))?, s),
        };
        Checksum { algo, value: value.to_string() }.checked()
    }

    /// The same, checked and in lower case: what the list keeps.
    pub fn checked(self) -> Result<Checksum> {
        let value = self.value.trim().to_ascii_lowercase();
        if value.len() != self.algo.hex_len() || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("a {} checksum is {} hexadecimal digits; {:?} is not one", self.algo.name(), self.algo.hex_len(), self.value.trim());
        }
        Ok(Checksum { algo: self.algo, value })
    }
}

fn digest<D: Digest>(mut r: impl Read) -> std::io::Result<String> {
    let mut d = D::new();
    // Large enough that a big file is read in few calls, small enough for any computer.
    let mut buf = vec![0u8; 1 << 20];
    loop {
        match r.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => d.update(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(d.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// The file's checksum, as lower-case hex. Reads the whole file: call it off the async runtime.
pub fn of_file(path: &Path, algo: Algo) -> std::io::Result<String> {
    let f = std::fs::File::open(path)?;
    match algo {
        Algo::Md5 => digest::<md5::Md5>(f),
        Algo::Sha1 => digest::<sha1::Sha1>(f),
        Algo::Sha256 => digest::<sha2::Sha256>(f),
        Algo::Sha512 => digest::<sha2::Sha512>(f),
    }
}

impl crate::Manager {
    /// Check a finished download against the checksum it was given, if any, and record the
    /// answer. A file that doesn't match is kept where it is — it may be the expected value
    /// that is wrong — and the download fails saying so.
    pub(crate) async fn verify_checksum(&self, id: u64) -> Result<()> {
        let Some((path, want)) = self.get(id).and_then(|it| Some((it.path.clone(), it.request.checksum?))) else { return Ok(()) };
        let p = path.clone();
        let got = tokio::task::spawn_blocking(move || of_file(&p, want.algo)).await.map_err(std::io::Error::other)?;
        let got = got.map_err(|e| anyhow::anyhow!("cannot read {} to check its checksum: {e}", path.display()))?;
        let ok = got == want.value;
        let mut db = self.0.db.lock().unwrap();
        if let Some(it) = db.items.get_mut(&id) {
            it.checksum_ok = Some(ok);
        }
        drop(db);
        if !ok {
            bail!(
                "checksum mismatch: expected {} {}, got {got}. The file is kept at {}: check the expected value (resume to check again), or restart the download to fetch it again",
                want.algo.name(),
                want.value,
                path.display()
            );
        }
        Ok(())
    }

    /// The checksum of a finished download's file, worked out now (for its properties).
    pub async fn checksum_of(&self, id: u64, algo: Algo) -> Result<String> {
        let it = self.get(id).ok_or_else(|| anyhow::anyhow!("there is no download #{id}"))?;
        if it.status != crate::Status::Done && it.checksum_ok != Some(false) {
            bail!("the download is not finished yet");
        }
        if !it.path.is_file() {
            bail!("{} is not a single file (a torrent of several files has a checksum per file, which its own pieces already checked)", it.path.display());
        }
        let p = it.path.clone();
        tokio::task::spawn_blocking(move || of_file(&p, algo)).await.map_err(std::io::Error::other)?.map_err(|e| anyhow::anyhow!("cannot read {}: {e}", it.path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_values() {
        let p = std::env::temp_dir().join(format!("reidm-sum-{}", std::process::id()));
        std::fs::write(&p, b"abc").unwrap();
        assert_eq!(of_file(&p, Algo::Md5).unwrap(), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(of_file(&p, Algo::Sha1).unwrap(), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(of_file(&p, Algo::Sha256).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert!(of_file(&p, Algo::Sha512).unwrap().starts_with("ddaf35a193617aba"));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn parsing() {
        let c = Checksum::parse("SHA-256:BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD").unwrap();
        assert_eq!(c.algo, Algo::Sha256);
        assert_eq!(c.value, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(Checksum::parse("900150983cd24fb0d6963f7d28e17f72").unwrap().algo, Algo::Md5, "known by its length");
        assert!(Checksum::parse("sha256:abc").unwrap_err().to_string().contains("64 hexadecimal digits"));
        assert!(Checksum::parse("crc32:abcd").unwrap_err().to_string().contains("md5, sha1, sha256 or sha512"));
        assert!(Checksum::parse("xyz").unwrap_err().to_string().contains("sha256:xyz"));
        assert!(Checksum::parse(&format!("md5:{}", "g".repeat(32))).is_err(), "not hex");
    }
}
