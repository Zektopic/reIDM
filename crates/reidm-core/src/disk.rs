//! What a download does to the disk, beyond the list: the part file's writes and its size, the
//! move into place, and the note on the finished file that says it came from the internet.
//!
//! Each needs a call the standard library does not offer on every system, so the per-system
//! code lives here, behind functions the engine calls the same way everywhere.

use std::fs::File;
use std::io;
use std::path::Path;

/// Write all of `buf` at offset `off`. Positional, so the connections of one download share one
/// file handle and none of them moves a file position the others rely on.
pub(crate) fn write_at(f: &File, buf: &[u8], off: u64) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::FileExt::write_all_at(f, buf, off)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        let (mut buf, mut off) = (buf, off);
        while !buf.is_empty() {
            match f.seek_write(buf, off) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(n) => {
                    buf = &buf[n..];
                    off += n as u64;
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

/// Give the part file its final size before any of it is written, without writing it.
///
/// - **Linux:** the space is reserved (`fallocate`), so a disk that is too small says so now,
///   with nothing downloaded, rather than hours in. File systems that can't reserve get a
///   sparse file, as before.
/// - **Windows:** the file is made sparse first. Otherwise NTFS fills everything below the first
///   write with zeros before that write returns: the last of eight connections on a 4 GB file
///   would wait for 3.5 GB of zeros, tens of seconds on a hard disk, while the server gives up
///   on it.
/// - **macOS:** APFS files are sparse already.
pub(crate) fn preallocate(f: &File, size: u64) -> io::Result<()> {
    if size == 0 {
        return f.set_len(0);
    }
    #[cfg(target_os = "linux")]
    if let Ok(len) = libc::off_t::try_from(size) {
        use std::os::fd::AsRawFd;
        // SAFETY: a system call on a descriptor this function borrows; no memory is passed.
        if unsafe { libc::fallocate(f.as_raw_fd(), 0, 0, len) } == 0 {
            return Ok(());
        }
        let e = io::Error::last_os_error();
        // Anything but "this file system can't" is a real answer: no space, most often.
        if !matches!(e.raw_os_error(), Some(libc::EOPNOTSUPP) | Some(libc::ENOSYS) | Some(libc::EINVAL)) {
            return Err(e);
        }
    }
    #[cfg(windows)]
    if let Err(e) = set_sparse(f) {
        // FAT and exFAT have no sparse files; they zero-fill whatever is done.
        tracing::debug!("cannot make a part file sparse: {e}");
    }
    f.set_len(size)
}

#[cfg(windows)]
fn set_sparse(f: &File) -> io::Result<()> {
    use std::ffi::c_void;
    use std::os::windows::io::AsRawHandle;
    // CTL_CODE(FILE_DEVICE_FILE_SYSTEM, 49, METHOD_BUFFERED, FILE_SPECIAL_ACCESS)
    const FSCTL_SET_SPARSE: u32 = 0x0009_00C4;
    #[link(name = "kernel32")]
    extern "system" {
        fn DeviceIoControl(
            device: *mut c_void,
            code: u32,
            in_buf: *const c_void,
            in_len: u32,
            out_buf: *mut c_void,
            out_len: u32,
            returned: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
    }
    let mut returned = 0u32;
    // SAFETY: the handle is borrowed from a live `File`; no buffers are passed (a null input
    // means "set sparse"), and the call is synchronous (no OVERLAPPED).
    let ok = unsafe {
        DeviceIoControl(f.as_raw_handle(), FSCTL_SET_SPARSE, std::ptr::null(), 0, std::ptr::null_mut(), 0, &mut returned, std::ptr::null_mut())
    };
    if ok != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Rename `from` to `to`, failing with [`io::ErrorKind::AlreadyExists`] instead of replacing a
/// file already at `to`. A finished download must never destroy a file it did not create.
pub(crate) fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        let (a, b) = (c_path(from)?, c_path(to)?);
        // SAFETY: two NUL-terminated paths that outlive the call.
        if unsafe { libc::renameat2(libc::AT_FDCWD, a.as_ptr(), libc::AT_FDCWD, b.as_ptr(), libc::RENAME_NOREPLACE) } == 0 {
            return Ok(());
        }
        let e = io::Error::last_os_error();
        // EINVAL: this file system can't refuse a replacement; check by hand below.
        if !matches!(e.raw_os_error(), Some(libc::EINVAL) | Some(libc::ENOSYS)) {
            return Err(e);
        }
    }
    #[cfg(target_os = "macos")]
    {
        let (a, b) = (c_path(from)?, c_path(to)?);
        // SAFETY: two NUL-terminated paths that outlive the call.
        if unsafe { libc::renamex_np(a.as_ptr(), b.as_ptr(), libc::RENAME_EXCL) } == 0 {
            return Ok(());
        }
        let e = io::Error::last_os_error();
        if !matches!(e.raw_os_error(), Some(libc::ENOTSUP) | Some(libc::EINVAL)) {
            return Err(e);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        extern "system" {
            fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
        }
        let wide = |p: &Path| p.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<u16>>();
        let (a, b) = (wide(from), wide(to));
        // SAFETY: two NUL-terminated UTF-16 paths that outlive the call. Flags 0: no
        // MOVEFILE_REPLACE_EXISTING (which `std::fs::rename` passes) and no copying between
        // drives, so an existing `to` fails with ERROR_ALREADY_EXISTS.
        if unsafe { MoveFileExW(a.as_ptr(), b.as_ptr(), 0) } != 0 {
            return Ok(());
        }
        Err(io::Error::last_os_error())
    }
    #[cfg(not(windows))]
    {
        // Where the system can't refuse for us, look first. Only a file created under that exact
        // name in the moment between the look and the rename could still be replaced.
        if to.symlink_metadata().is_ok() {
            return Err(io::ErrorKind::AlreadyExists.into());
        }
        std::fs::rename(from, to)
    }
}

#[cfg(unix)]
fn c_path(p: &Path) -> io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(p.as_os_str().as_bytes()).map_err(|_| io::ErrorKind::InvalidInput.into())
}

/// The nearest folder that exists at or above `path`: a download's folder may not have been
/// created yet, and the space that matters is that of the drive it will be on.
fn existing(path: &Path) -> Option<&Path> {
    path.ancestors().find(|p| !p.as_os_str().is_empty() && p.exists())
}

/// The space free to this user on the drive `path` is (or will be) on.
pub(crate) fn free_space(path: &Path) -> io::Result<u64> {
    let p = existing(path).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{} does not exist", path.display())))?;
    #[cfg(unix)]
    {
        let c = c_path(p)?;
        // SAFETY: a NUL-terminated path, and a zeroed struct the call fills in.
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
            return Err(io::Error::last_os_error());
        }
        // Blocks free to an unprivileged user (not the root reserve), in fragment-size units.
        #[allow(clippy::unnecessary_cast)]
        Ok((st.f_bavail as u64).saturating_mul(st.f_frsize as u64))
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        extern "system" {
            fn GetDiskFreeSpaceExW(dir: *const u16, avail: *mut u64, total: *mut u64, free: *mut u64) -> i32;
        }
        let wide: Vec<u16> = p.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut avail = 0u64;
        // SAFETY: a NUL-terminated path, one out-pointer to a local, and nulls for the rest.
        if unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut avail, std::ptr::null_mut(), std::ptr::null_mut()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(avail)
    }
}

/// Which drive `path` is on, to tell whether two folders share one.
pub(crate) fn volume(path: &Path) -> Option<String> {
    let p = existing(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        p.metadata().ok().map(|m| m.dev().to_string())
    }
    #[cfg(windows)]
    {
        // The drive letter or `\\server\share` the path starts with.
        match p.components().next()? {
            std::path::Component::Prefix(pre) => Some(pre.as_os_str().to_string_lossy().to_ascii_lowercase()),
            _ => None,
        }
    }
}

/// How much of a part file of `size` bytes still needs space on the drive. Where the space
/// was reserved up front (Linux `fallocate`), nothing; where the file fills in as it is written
/// (sparse files), what is not written yet.
pub(crate) fn unreserved(part: &Path, size: u64, written: u64) -> u64 {
    #[cfg(unix)]
    if let Ok(m) = part.metadata() {
        use std::os::unix::fs::MetadataExt;
        return size.saturating_sub(m.blocks().saturating_mul(512));
    }
    let _ = part;
    size.saturating_sub(written)
}

/// An address as a browser records it next to a download: without a user name or password.
#[cfg(any(windows, test))]
fn public_url(raw: &str) -> Option<String> {
    let mut u = url::Url::parse(raw).ok()?;
    let _ = u.set_username("");
    let _ = u.set_password(None);
    Some(u.to_string())
}

/// The `Zone.Identifier` stream Windows reads to know a file came from the internet (zone 3),
/// and from where: what browsers write, and what SmartScreen and Office's Protected View act on.
#[cfg(any(windows, test))]
fn zone_identifier(url: &str, referrer: Option<&str>) -> String {
    let mut s = String::from("[ZoneTransfer]\r\nZoneId=3\r\n");
    if let Some(r) = referrer.and_then(public_url) {
        s.push_str(&format!("ReferrerUrl={r}\r\n"));
    }
    if let Some(u) = public_url(url) {
        s.push_str(&format!("HostUrl={u}\r\n"));
    }
    s
}

/// The `com.apple.quarantine` value macOS's Gatekeeper reads: "downloaded, not yet approved"
/// (0081), when, and by what.
#[cfg(any(target_os = "macos", test))]
fn quarantine_value(unix_secs: u64) -> String {
    format!("0081;{unix_secs:08x};reIDM;")
}

/// Mark a finished download as having come from the internet, as the browser that handed it
/// over would have: the system then warns before it runs, and opens documents in protected
/// mode. Without this, taking a download from the browser would quietly lower the user's
/// protection. A file system that can't hold the mark (FAT, a network share) just goes without.
///
/// Folders are left alone; a torrent's files are marked one by one.
pub(crate) fn mark_from_internet(path: &Path, url: &str, referrer: Option<&str>) {
    if !path.is_file() {
        return;
    }
    #[cfg(windows)]
    {
        let mut ads = path.as_os_str().to_owned();
        ads.push(":Zone.Identifier");
        if let Err(e) = std::fs::write(std::path::PathBuf::from(ads), zone_identifier(url, referrer)) {
            tracing::debug!("cannot mark {} as downloaded: {e}", path.display());
        }
    }
    #[cfg(target_os = "macos")]
    {
        let _ = (url, referrer);
        let value = quarantine_value(crate::now_secs());
        if let (Ok(p), Ok(name)) = (c_path(path), std::ffi::CString::new("com.apple.quarantine")) {
            // SAFETY: NUL-terminated path and name, and a value buffer with its length, all
            // outliving the call.
            let r = unsafe { libc::setxattr(p.as_ptr(), name.as_ptr(), value.as_ptr().cast(), value.len(), 0, 0) };
            if r != 0 {
                tracing::debug!("cannot mark {} as downloaded: {}", path.display(), io::Error::last_os_error());
            }
        }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    let _ = (url, referrer);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_zone_identifier_says_internet_and_drops_credentials() {
        let z = zone_identifier("https://user:secret@dl.example/a.exe?x=1", Some("https://dl.example/page"));
        assert_eq!(z, "[ZoneTransfer]\r\nZoneId=3\r\nReferrerUrl=https://dl.example/page\r\nHostUrl=https://dl.example/a.exe?x=1\r\n");
        assert_eq!(zone_identifier("not a url", None), "[ZoneTransfer]\r\nZoneId=3\r\n");
    }

    #[test]
    fn the_quarantine_value_is_what_gatekeeper_reads() {
        assert_eq!(quarantine_value(0x6500_0000), "0081;65000000;reIDM;");
    }

    #[test]
    fn free_space_is_measured_for_folders_that_do_not_exist_yet() {
        let d = std::env::temp_dir();
        let free = free_space(&d.join("not-yet").join("deeper")).unwrap();
        assert!(free > 0);
        assert_eq!(volume(&d.join("not-yet")), volume(&d));
        let p = d.join(format!("reidm-unreserved-{}", std::process::id()));
        let f = std::fs::File::create(&p).unwrap();
        preallocate(&f, 4 << 20).unwrap();
        // Reserved (Linux) or sparse: either way, never more than the file's size is asked for,
        // and what is written is never asked for again.
        assert!(unreserved(&p, 4 << 20, 0) <= 4 << 20);
        write_at(&f, &vec![1u8; 1 << 20], 0).unwrap();
        assert!(unreserved(&p, 4 << 20, 1 << 20) <= 3 << 20);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn a_rename_never_replaces() {
        let d = std::env::temp_dir().join(format!("reidm-disk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let (a, b) = (d.join("a"), d.join("b"));
        std::fs::write(&a, b"new").unwrap();
        std::fs::write(&b, b"theirs").unwrap();
        assert_eq!(rename_noreplace(&a, &b).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&b).unwrap(), b"theirs");
        std::fs::remove_file(&b).unwrap();
        rename_noreplace(&a, &b).unwrap();
        assert_eq!(std::fs::read(&b).unwrap(), b"new");
    }

    #[test]
    fn preallocation_sizes_the_file_and_keeps_what_is_written() {
        let p = std::env::temp_dir().join(format!("reidm-prealloc-{}", std::process::id()));
        let f = std::fs::OpenOptions::new().create(true).truncate(true).read(true).write(true).open(&p).unwrap();
        write_at(&f, b"head", 0).unwrap();
        preallocate(&f, 3 << 20).unwrap();
        write_at(&f, b"tail", (3 << 20) - 4).unwrap();
        let got = std::fs::read(&p).unwrap();
        assert_eq!(got.len(), 3 << 20);
        assert_eq!(&got[..4], b"head");
        assert_eq!(&got[got.len() - 4..], b"tail");
        let _ = std::fs::remove_file(&p);
    }
}
