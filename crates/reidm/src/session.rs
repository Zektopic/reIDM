//! Transport-independent protocol logic: one `Session` per connected extension (or CLI) client.

use reidm_core::{Manager, Request};
use reidm_proto::{Message, Value};
use std::sync::atomic::{AtomicU32, Ordering};

/// Protocol revision we advertise. The extension gates features on this number
/// (>= 113 means the server supplies every pattern list itself).
const SERVER_PROTO: i64 = 113;
/// Version we report as the "manager" (6.43 build 15, encoded major<<24|minor<<16|build<<8).
const MANAGER_VERSION: i64 = (6 << 24) | (43 << 16) | (15 << 8);

/// File types captured from browser downloads by default (IDM's stock list).
const DEFAULT_FILE_TYPES: &str = "3GP|7Z|AAC|ACE|AIF|ARJ|ASF|AVI|BIN|BZ2|DEB|DMG|EXE|GZ|GZIP|IMG|ISO|LZH|M4A|M4V|MKV|MOV|MP3|MP4|MPA|MPE|MPEG|MPG|MSI|MSU|OGG|OGV|PDF|PKG|PLJ|PPS|PPT|QT|RA|RAR|RM|RPM|SEA|SIT|SITX|TAR|TIF|TIFF|WAV|WMA|WMV|XZ|Z|ZIP|ZST";

/// Keys a user holds while clicking, as the extension encodes them (`va` in `background.js`
/// 6.43.1: Shift 1, Ctrl 2, Alt 4, Insert 8, Delete 16).
const KEY_ALT: i64 = 4;
const KEY_INSERT: i64 = 8;
/// IDM's own defaults: hold Insert to force a capture the file-type list would not make, hold
/// Alt to let the browser keep a download. Settings args 0 and 1 carry them; the extension
/// stores them as `j[-32]`/`j[-33]` and tests them in `rb` (force) and `qb` (prevent). Sending
/// 0 for both, as reIDM did before, silently disabled both keys.
const FORCE_KEYS: i64 = KEY_INSERT;
const PREVENT_KEYS: i64 = KEY_ALT;

/// Property names the content script hooks to spot XHR/fetch downloads (key 23).
const PAGE_HOOKS: &str = "XMLHttpRequest|open|send|setRequestHeader|upload|loadend|responseText|fetch|headers|body|text|json|getReader|pipeThrough|Headers|accept|*/*|application/vnd.t1c|Response|url|get";

// Message types sent by the extension.
const T_KEEPALIVE: u32 = 0;
const T_REPLY: u32 = 1;
const T_HELLO: u32 = 2;
const T_INTERCEPT: u32 = 13;
const T_DOWNLOAD: u32 = 14;
// Message types sent by the server.
const T_HELLO_ACK: u32 = 3;
const T_SETTINGS: u32 = 4;
/// Types from here up were reIDM's own additions to the protocol (pause 1001, resume 1002,
/// remove 1003), which the real extension never sends. They are no longer honoured: this socket
/// has no authentication — any local process, another user's included, can connect — and
/// "remove and delete the file" must not be one connection away. The CLI uses the
/// token-protected HTTP API instead.
pub const T_RESERVED: u32 = 1000;

// Reply codes understood by the extension (`mc()` in background.js).
const R_IGNORE: i64 = 0;
const R_TAKEN: i64 = 3;

// Keys
const K_URL: u32 = 6;
const K_PAGE: u32 = 7;
const K_NAME: u32 = 100;
const K_REFERER: u32 = 50;
const K_COOKIE: u32 = 51;
const K_UA: u32 = 54;

static NEXT_ID: AtomicU32 = AtomicU32::new(1);

pub struct Session {
    mgr: Manager,
    /// Dispatches every message to the bundled handler; counts hellos for diagnostics.
    pub hello_seen: bool,
}

fn nid() -> u32 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// The URL may carry IDM's `idmdwnlmfv9://` marker in front of the real scheme.
fn clean_url(u: &str) -> String {
    match u.find("://") {
        Some(i) if u[..i].starts_with("idmdwnlmfv") => u[i + 3..].to_string(),
        _ => u.to_string(),
    }
}

impl Session {
    pub fn new(mgr: Manager) -> Self {
        Self { mgr, hello_seen: false }
    }

    fn reply(req: &Message, args: &[i64]) -> Message {
        let mut m = Message::new(nid(), T_REPLY, 2, 0);
        m.args.push(reidm_proto::Arg::Int(req.id as i64));
        m.args.extend(args.iter().map(|a| reidm_proto::Arg::Int(*a)));
        m
    }

    fn hello_ack(&self) -> Vec<Message> {
        // type 3: [proto, ?, ?, ?, dpi, manager-version, client-id, ?, zoom%]
        let ack = Message::new(nid(), T_HELLO_ACK, 1, 0).with_args([SERVER_PROTO, 0, 0, 0, 96, MANAGER_VERSION, 1, 0, 100]);
        // type 4: capture settings. flags: 64 auto-capture, 32 sniffing UI, 256/512 menu items, 1024 ext. mode
        // args: [force-capture keys, prevent-capture keys, manager version, feature bits]
        let mut set = Message::new(nid(), T_SETTINGS, 1, 64 | 32 | 256 | 512 | 1024).with_args([FORCE_KEYS, PREVENT_KEYS, MANAGER_VERSION, 0]);
        set.keys.insert(1, Value::Str(DEFAULT_FILE_TYPES.into()));
        set.keys.insert(23, Value::Str(PAGE_HOOKS.into()));
        vec![ack, set]
    }

    fn request_from(m: &Message) -> Option<Request> {
        let url = clean_url(m.str_key(K_URL)?);
        let nonempty = |k| m.str_key(k).filter(|s| !s.is_empty()).map(str::to_string);
        Some(Request {
            url,
            referer: nonempty(K_REFERER).or_else(|| nonempty(K_PAGE)),
            cookie: nonempty(K_COOKIE),
            user_agent: nonempty(K_UA),
            filename: nonempty(K_NAME),
            dir: None,
            media: None,
            torrent: None,
            checksum: None,
        })
    }

    /// Handle one inbound message and return the messages to send back.
    /// Queue one download off the async runtime: choosing its name reads the download folder,
    /// which on a synced or slow one can take a while.
    async fn queue(&self, r: Request) -> bool {
        let mgr = self.mgr.clone();
        match tokio::task::spawn_blocking(move || mgr.add(r)).await {
            Ok(Ok(id)) => {
                tracing::info!("queued download #{id}");
                true
            }
            Ok(Err(e)) => {
                tracing::warn!("rejected download: {e:#}");
                false
            }
            Err(e) => {
                tracing::warn!("the download could not be queued: {e}");
                false
            }
        }
    }

    pub async fn handle(&mut self, m: &Message) -> Vec<Message> {
        let wants_reply = m.flags & 1 != 0;
        match m.ty {
            T_HELLO => {
                self.hello_seen = true;
                tracing::info!("extension hello (args={:?})", m.args.iter().map(|a| a.as_i64()).collect::<Vec<_>>());
                self.hello_ack()
            }
            T_KEEPALIVE => vec![Self::reply(m, &[])],
            T_DOWNLOAD => {
                // The reply tells the extension whether we took the download. Answering "taken"
                // when we could not queue anything would make the browser silently drop it.
                let taken = match Self::request_from(m) {
                    Some(r) => self.queue(r).await,
                    None => {
                        tracing::debug!("type 14 without URL");
                        false
                    }
                };
                if wants_reply {
                    vec![Self::reply(m, &[if taken { R_TAKEN } else { R_IGNORE }])]
                } else {
                    vec![]
                }
            }
            T_INTERCEPT if m.flags & 128 != 0 => {
                // The browser is about to save a file itself; claim it if it is plain http(s).
                let taken = match Self::request_from(m) {
                    Some(r) => self.queue(r).await,
                    None => false,
                };
                vec![Self::reply(m, &[if taken { R_TAKEN } else { R_IGNORE }])]
            }
            T_INTERCEPT => {
                // Passive request sniffing: we do not capture from raw network events.
                if wants_reply { vec![Self::reply(m, &[R_IGNORE])] } else { vec![] }
            }
            T_REPLY => vec![], // answer to something we sent; nothing pending
            t if t >= T_RESERVED => {
                tracing::warn!("ignoring control message type {t} on the extension socket; use the HTTP API");
                vec![]
            }
            _ => {
                if wants_reply { vec![Self::reply(m, &[])] } else { vec![] }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mgr() -> Manager {
        let d = std::env::temp_dir().join(format!("reidm-test-{}", std::process::id()));
        Manager::open(&d, d.clone(), 2).unwrap()
    }

    #[tokio::test]
    async fn hello_gets_ack_and_settings() {
        let mut s = Session::new(mgr());
        let out = s.handle(&Message::new(1, T_HELLO, 5, 0).with_args([113, 93])).await;
        assert_eq!(out.iter().map(|m| m.ty).collect::<Vec<_>>(), vec![T_HELLO_ACK, T_SETTINGS]);
        // encodes to something the parser accepts
        for m in &out {
            Message::decode(m.encode().as_bytes()).unwrap();
        }
    }

    #[tokio::test]
    async fn settings_enable_idms_alt_and_insert_keys() {
        let mut s = Session::new(mgr());
        let out = s.handle(&Message::new(1, T_HELLO, 5, 0).with_args([113, 93])).await;
        let settings = out.iter().find(|m| m.ty == T_SETTINGS).unwrap();
        assert_eq!(settings.arg(0), Some(8), "Insert forces a capture");
        assert_eq!(settings.arg(1), Some(4), "Alt lets the browser keep a download");
        assert_eq!(settings.arg(2), Some(MANAGER_VERSION));
    }

    #[tokio::test]
    async fn keepalive_is_answered() {
        let mut s = Session::new(mgr());
        let out = s.handle(&Message::new(9, T_KEEPALIVE, 1, 1)).await;
        assert_eq!(out[0].ty, T_REPLY);
        assert_eq!(out[0].arg(0), Some(9));
    }

    #[tokio::test]
    async fn control_messages_cannot_remove_downloads() {
        let d = std::env::temp_dir().join(format!("reidm-test-ctl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let mgr = Manager::open(&d, d.clone(), 2).unwrap();
        let id = mgr.add_with(Request { url: "https://example.invalid/a.bin".into(), ..Default::default() }, reidm_core::AddOptions { later: true, ..Default::default() }).unwrap().id;
        let mut s = Session::new(mgr.clone());
        for ty in [1001, 1002, 1003] {
            // `1003 id 1` used to remove the download and delete its file.
            let out = s.handle(&Message::new(1, ty, 1, 1).with_args([id as i64, 1])).await;
            assert!(out.is_empty(), "type {ty} must get no answer that it was done");
        }
        assert!(mgr.get(id).is_some(), "the download is still listed");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn strips_idm_scheme_marker() {
        assert_eq!(clean_url("idmdwnlmfv9://https://a/b"), "https://a/b");
        assert_eq!(clean_url("https://a/b"), "https://a/b");
    }
}
