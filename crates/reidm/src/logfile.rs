//! The service's log on disk: what happened, for when the window or the service misbehaves and
//! nobody is watching a terminal. Kept to two small files so it can never fill a disk.
//!
//! Only the daemon writes it. CLI commands keep to stderr: several of them can run at once, and
//! two processes appending to one file would mix their events.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

/// At most this much goes into one file before it is moved aside and a new one started. With
/// the `.1` backup, the log never takes more than about 10 MB.
pub(crate) const MAX_BYTES: u64 = 5 * 1024 * 1024;

pub(crate) struct LogFile {
    path: PathBuf,
    state: Mutex<State>,
}

struct State {
    file: Option<File>,
    written: u64,
}

impl LogFile {
    pub(crate) fn new(path: PathBuf) -> Self {
        let written = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        Self { path, state: Mutex::new(State { file: None, written }) }
    }
}

fn open(path: &Path) -> io::Result<File> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let mut o = OpenOptions::new();
    o.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // A log can hold addresses and paths; only its owner should read it.
        o.mode(0o600);
    }
    o.open(path)
}

fn rotate(path: &Path, state: &mut State) -> io::Result<()> {
    state.file = None;
    let old = path.with_extension("log.1");
    // Replacing the previous backup is intended: two files are kept, not a history. If this
    // fails (a sharing violation on Windows, a read-only folder), the event is lost rather than
    // letting the file grow without end.
    std::fs::rename(path, &old)?;
    state.file = Some(open(path)?);
    state.written = 0;
    Ok(())
}

/// One event's writer, holding the lock for as long as the event is being written, so lines
/// from different threads never interleave.
pub(crate) struct LogWriter<'a> {
    path: &'a Path,
    state: MutexGuard<'a, State>,
}

impl Write for LogWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.state.file.is_none() {
            self.state.file = Some(open(self.path)?);
        }
        if self.state.written >= MAX_BYTES {
            rotate(self.path, &mut self.state)?;
        }
        self.state.file.as_mut().expect("just opened").write_all(buf)?;
        self.state.written += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.state.file.as_mut() {
            Some(f) => f.flush(),
            None => Ok(()),
        }
    }
}

impl<'a> tracing_subscriber::fmt::writer::MakeWriter<'a> for LogFile {
    type Writer = LogWriter<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        LogWriter { path: &self.path, state: self.state.lock().unwrap_or_else(|p| p.into_inner()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use tracing_subscriber::fmt::writer::MakeWriter;

    fn tmp(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("reidm-log-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn events_go_to_a_private_file_that_stays_small() {
        let d = tmp("rotate");
        let path = d.join("reidm.log");
        let log = LogFile::new(path.clone());
        let chunk = [b'x'; 1024];
        // A few pass the cap: the file is rotated before it can overshoot, and the previous
        // one is kept as `.1`.
        for _ in 0..(MAX_BYTES / 1024 + 8) {
            log.make_writer().write_all(&chunk).unwrap();
        }
        assert!(std::fs::metadata(&path).unwrap().len() <= MAX_BYTES, "the log was not rotated");
        assert!(d.join("reidm.log.1").exists(), "the previous file is kept");
        #[cfg(unix)]
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600, "only its owner can read it");
        // A new process appends to what is there rather than starting over.
        drop(log);
        let again = LogFile::new(path.clone());
        again.make_writer().write_all(b"still here\n").unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().ends_with("still here\n"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_missing_folder_is_made_on_the_first_event() {
        let d = tmp("mkdir");
        let path = d.join("data").join("reidm.log");
        let log = LogFile::new(path.clone());
        assert!(!path.exists(), "nothing is created until something is logged");
        log.make_writer().write_all(b"hello\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello\n");
        let _ = std::fs::remove_dir_all(&d);
    }
}
