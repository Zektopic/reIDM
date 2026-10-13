//! Wire format spoken between the IDM browser extension and the download manager.
//!
//! Derived from the extension's serializer/parser and the `IDMMsgHost` native-messaging
//! relay (see `docs/PROTOCOL.md`). A frame is ASCII/UTF-8 text:
//!
//! ```text
//! MSG#<id>#<type>#<class>#<flags>[:<arg>]*[,<key>=<value>]*;
//! ```
//!
//! * `<arg>` is an integer or decimal number.
//! * `<value>` is either a bare integer, or `<byte-len>:<bytes>` for strings.

use std::collections::BTreeMap;
use std::fmt::Write as _;

#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    Int(i64),
    Float(f64),
}

impl Arg {
    pub fn as_i64(&self) -> i64 {
        match self {
            Arg::Int(i) => *i,
            Arg::Float(f) => *f as i64,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    Str(String),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Message {
    pub id: u32,
    pub ty: u32,
    /// Third header field (the extension sends 5 for hello, 1/2 for requests/replies).
    pub class: u32,
    /// Fourth header field: bit flags. Bit 0 = "sender expects a reply (type 1)".
    pub flags: u32,
    pub args: Vec<Arg>,
    pub keys: BTreeMap<u32, Value>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    /// Not enough bytes yet (stream framing: wait for more).
    Incomplete,
    Malformed(&'static str),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Incomplete => write!(f, "incomplete frame"),
            ParseError::Malformed(m) => write!(f, "malformed frame: {m}"),
        }
    }
}
impl std::error::Error for ParseError {}

impl Message {
    pub fn new(id: u32, ty: u32, class: u32, flags: u32) -> Self {
        Self { id, ty, class, flags, ..Default::default() }
    }

    pub fn with_args(mut self, args: impl IntoIterator<Item = i64>) -> Self {
        self.args = args.into_iter().map(Arg::Int).collect();
        self
    }

    pub fn with_str(mut self, key: u32, v: impl Into<String>) -> Self {
        self.keys.insert(key, Value::Str(v.into()));
        self
    }

    pub fn with_int(mut self, key: u32, v: i64) -> Self {
        self.keys.insert(key, Value::Int(v));
        self
    }

    pub fn str_key(&self, key: u32) -> Option<&str> {
        self.keys.get(&key).and_then(Value::as_str)
    }

    pub fn arg(&self, i: usize) -> Option<i64> {
        self.args.get(i).map(Arg::as_i64)
    }

    /// Serialise to the on-the-wire text form (including the trailing `;`).
    pub fn encode(&self) -> String {
        let mut s = String::new();
        let _ = write!(s, "MSG#{}#{}#{}#{}", self.id, self.ty, self.class, self.flags);
        for a in &self.args {
            match a {
                Arg::Int(i) => {
                    let _ = write!(s, ":{i}");
                }
                Arg::Float(f) => {
                    let _ = write!(s, ":{f:?}");
                }
            }
        }
        for (k, v) in &self.keys {
            match v {
                Value::Int(i) => {
                    let _ = write!(s, ",{k}={i}");
                }
                Value::Str(t) => {
                    let _ = write!(s, ",{k}={}:{t}", t.len());
                }
            }
        }
        s.push(';');
        s
    }

    /// Parse one frame from the start of `input`. Returns the message and bytes consumed.
    pub fn decode(input: &[u8]) -> Result<(Message, usize), ParseError> {
        // header: MSG#id#type#flags#j followed by one of : , ;
        let hdr_end = input
            .iter()
            .position(|b| matches!(b, b':' | b',' | b';'))
            .ok_or(ParseError::Incomplete)?;
        let hdr = std::str::from_utf8(&input[..hdr_end]).map_err(|_| ParseError::Malformed("utf8 header"))?;
        let parts: Vec<&str> = hdr.split('#').collect();
        if parts.len() != 5 || parts[0] != "MSG" {
            return Err(ParseError::Malformed("bad header"));
        }
        let num = |s: &str| s.parse::<u32>().map_err(|_| ParseError::Malformed("bad header number"));
        let mut m = Message::new(num(parts[1])?, num(parts[2])?, num(parts[3])?, num(parts[4])?);
        let mut term = input[hdr_end];
        let mut p = hdr_end + 1;

        if term == b':' {
            let end = input[p..]
                .iter()
                .position(|b| matches!(b, b',' | b';'))
                .ok_or(ParseError::Incomplete)?;
            let txt = std::str::from_utf8(&input[p..p + end]).map_err(|_| ParseError::Malformed("utf8 args"))?;
            for a in txt.split(':') {
                if a.contains('.') {
                    m.args.push(Arg::Float(a.parse().map_err(|_| ParseError::Malformed("bad float arg"))?));
                } else {
                    m.args.push(Arg::Int(a.parse().map_err(|_| ParseError::Malformed("bad int arg"))?));
                }
            }
            term = input[p + end];
            p += end + 1;
        }

        while term == b',' {
            let end = input[p..]
                .iter()
                .position(|b| matches!(b, b':' | b',' | b';'))
                .ok_or(ParseError::Incomplete)?;
            let kv = std::str::from_utf8(&input[p..p + end]).map_err(|_| ParseError::Malformed("utf8 key"))?;
            let (k, v) = kv.split_once('=').ok_or(ParseError::Malformed("key without ="))?;
            let key: u32 = k.parse().map_err(|_| ParseError::Malformed("bad key"))?;
            let sep = input[p + end];
            let val: i64 = v.parse().map_err(|_| ParseError::Malformed("bad key value"))?;
            p += end + 1;
            if sep == b':' {
                let n = val as usize;
                if input.len() < p + n + 1 {
                    return Err(ParseError::Incomplete);
                }
                let s = String::from_utf8_lossy(&input[p..p + n]).into_owned();
                m.keys.insert(key, Value::Str(s));
                p += n;
                term = input[p];
                if term != b',' && term != b';' {
                    return Err(ParseError::Malformed("expected , or ; after string"));
                }
                p += 1;
            } else {
                m.keys.insert(key, Value::Int(val));
                term = sep;
            }
        }
        if term != b';' {
            return Err(ParseError::Malformed("missing terminator"));
        }
        Ok((m, p))
    }
}

/// Native-messaging payload helpers. `IDMMsgHost` receives each message from the browser as a
/// JSON *string* and forwards its unescaped UTF-8 bytes; replies are re-escaped the same way.
pub mod native {
    /// Browser JSON string -> raw frame text.
    pub fn unwrap_json_string(payload: &[u8]) -> Option<String> {
        serde_json::from_slice::<String>(payload).ok()
    }
    /// Raw frame text -> JSON string payload for the browser.
    pub fn wrap_json_string(frame: &str) -> Vec<u8> {
        serde_json::to_vec(frame).expect("string serialises")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let m = Message::new(7, 14, 0, 1)
            .with_args([1, -2])
            .with_str(6, "https://example.com/a b.zip")
            .with_int(8, 4)
            .with_str(51, "héllo=1");
        let enc = m.encode();
        let (d, n) = Message::decode(enc.as_bytes()).unwrap();
        assert_eq!(n, enc.len());
        assert_eq!(d, m);
    }

    #[test]
    fn matches_extension_hello_shape() {
        // S(this,2,5,0,[113,93,1031,0,16845059,cid,15,3],{112:..,113:..})
        let raw = "MSG#1#2#0#5:113:93:1031:0:16845059:0:15:3,112=8:Chrome/1,113=3:abc;";
        let (m, n) = Message::decode(raw.as_bytes()).unwrap();
        assert_eq!(n, raw.len());
        assert_eq!(m.ty, 2);
        assert_eq!(m.args.len(), 8);
        assert_eq!(m.str_key(112), Some("Chrome/1"));
    }

    #[test]
    fn incomplete() {
        assert_eq!(Message::decode(b"MSG#1#2#0#5:1:2"), Err(ParseError::Incomplete));
    }
}
