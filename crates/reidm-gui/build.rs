//! Stamp the application icon into the Windows executable so Explorer and the taskbar show it
//! even when the process is not running. Other platforms carry the icon in their bundle metadata.
//!
//! Failure is not fatal: a missing resource compiler should not break the build.

fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../packaging/icon/reidm.ico");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed the Windows icon: {e}");
        }
    }
    println!("cargo:rerun-if-changed=../../packaging/icon/reidm.ico");
}
