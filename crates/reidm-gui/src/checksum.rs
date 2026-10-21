//! An expected checksum, as typed or pasted: `sha256:9f86d0…`, `SHA-256 9f86d0…`, or just the
//! hex, whose length says the algorithm.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Algo {
    Md5,
    Sha1,
    Sha256,
    Sha512,
}

impl Algo {
    pub const ALL: [Algo; 4] = [Algo::Md5, Algo::Sha1, Algo::Sha256, Algo::Sha512];

    pub fn wire(self) -> &'static str {
        match self {
            Algo::Md5 => "md5",
            Algo::Sha1 => "sha1",
            Algo::Sha256 => "sha256",
            Algo::Sha512 => "sha512",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Algo::Md5 => "MD5",
            Algo::Sha1 => "SHA-1",
            Algo::Sha256 => "SHA-256",
            Algo::Sha512 => "SHA-512",
        }
    }

    /// Hex digits in one of its checksums.
    fn hex_len(self) -> usize {
        match self {
            Algo::Md5 => 32,
            Algo::Sha1 => 40,
            Algo::Sha256 => 64,
            Algo::Sha512 => 128,
        }
    }

    fn named(name: &str) -> Option<Algo> {
        let n: String = name.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
        Algo::ALL.into_iter().find(|a| a.wire() == n)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checksum {
    pub algo: Algo,
    /// Lower-case hex.
    pub value: String,
}

impl Checksum {
    /// The request's `checksum` field.
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({ "algo": self.algo.wire(), "value": self.value })
    }

    /// Does a computed value match? Case doesn't matter.
    pub fn matches(&self, value: &str) -> bool {
        self.value.eq_ignore_ascii_case(value.trim())
    }
}

/// Read what was typed. Empty is no checksum; anything else must be one, or the error says why.
pub fn parse(text: &str) -> Result<Option<Checksum>, String> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(None);
    }
    // "sha256:…", "SHA-256 …", "sha512=…"; otherwise only the hex.
    let (named, hex) = match t.find([':', ' ', '=']) {
        Some(i) => (Some(&t[..i]), t[i + 1..].trim()),
        None => (None, t),
    };
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) || hex.is_empty() {
        return Err("A checksum is hexadecimal: the digits 0–9 and the letters a–f".into());
    }
    let by_length = Algo::ALL.into_iter().find(|a| a.hex_len() == hex.len());
    let algo = match named {
        Some(n) => Algo::named(n).ok_or_else(|| format!("reIDM checks MD5, SHA-1, SHA-256 and SHA-512, not “{n}”"))?,
        None => by_length.ok_or_else(|| format!("{} hex digits isn't MD5 (32), SHA-1 (40), SHA-256 (64) or SHA-512 (128)", hex.len()))?,
    };
    if hex.len() != algo.hex_len() {
        return Err(format!("A {} checksum has {} hex digits, not {}", algo.label(), algo.hex_len(), hex.len()));
    }
    Ok(Some(Checksum { algo, value: hex.to_ascii_lowercase() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA256: &str = "9F86D081884C7D659A2FEAA0C55AD015A3BF4F1B2B0B822CD15D6C15B0F00A08";

    #[test]
    fn the_usual_ways_of_writing_one() {
        let c = Checksum { algo: Algo::Sha256, value: SHA256.to_ascii_lowercase() };
        let want = Some(c.clone());
        assert_eq!(parse(SHA256).unwrap(), want, "bare hex: the length says SHA-256");
        assert_eq!(parse(&format!("sha256:{SHA256}")).unwrap(), want);
        assert_eq!(parse(&format!(" SHA-256 {SHA256} ")).unwrap(), want);
        assert_eq!(parse("md5=d41d8cd98f00b204e9800998ecf8427e").unwrap().unwrap().algo, Algo::Md5);
        assert_eq!(parse("").unwrap(), None);
        assert_eq!(c.json(), serde_json::json!({ "algo": "sha256", "value": SHA256.to_ascii_lowercase() }));
    }

    #[test]
    fn mistakes_are_explained() {
        assert!(parse("sha256:abc").unwrap_err().contains("64"));
        assert!(parse("crc32:abcd1234").unwrap_err().contains("crc32"));
        assert!(parse("not a checksum").unwrap_err().contains("hexadecimal"));
        assert!(parse("abcdef").unwrap_err().contains("6 hex digits"));
    }

    #[test]
    fn matching_ignores_case() {
        let c = parse(SHA256).unwrap().unwrap();
        assert!(c.matches(SHA256) && c.matches(&SHA256.to_ascii_lowercase()));
        assert!(!c.matches("00"));
    }
}
