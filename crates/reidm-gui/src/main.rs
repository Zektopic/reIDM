//! reIDM desktop application.
//!
//! The window is a front-end for the reIDM daemon: it starts one if none is running, then reads
//! the download list over the daemon's local JSON API. Downloads therefore continue when the
//! window is closed, and the browser extension talks to the same daemon.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod app;
mod autostart;
mod checksum;
mod dropped;
mod icons;
mod model;
mod notify;
mod options;
mod picker;
mod playlist;
mod torrent;
mod prefs;
mod queue;
mod theme;
mod tray;
mod view;
mod when_done;
mod worker;

/// A PNG as straight RGBA8, with its width and height.
pub(crate) fn decode_png(png: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..info.buffer_size()].to_vec(),
        png::ColorType::Rgb => {
            let n = info.buffer_size() / 3;
            let mut out = Vec::with_capacity(n * 4);
            for i in 0..n {
                out.extend_from_slice(&[buf[i * 3], buf[i * 3 + 1], buf[i * 3 + 2], 255]);
            }
            out
        }
        _ => return None,
    };
    Some((rgba, info.width, info.height))
}

/// The window/taskbar icon, decoded from the PNG shipped in the repository.
fn window_icon() -> Option<egui::IconData> {
    const PNG: &[u8] = include_bytes!("../../../packaging/icon/reidm-256.png");
    let (rgba, width, height) = decode_png(PNG)?;
    Some(egui::IconData { rgba, width, height })
}

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let background = args.iter().any(|a| a == "--background");

    // `--background` is what the login autostart runs: make sure the service is up, then exit
    // without showing a window. With a window, the poller starts the service instead, so the
    // window shows at once and says what it is waiting for.
    if background {
        if !api::ensure_daemon() {
            eprintln!("reIDM: could not start the background download service. Is reidm installed next to this program?");
            std::process::exit(1);
        }
        return Ok(());
    }

    let dark = args.iter().any(|a| a == "--dark");
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1080.0, 640.0])
        .with_min_inner_size([720.0, 420.0])
        .with_title("reIDM");
    if let Some(icon) = window_icon() {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("reIDM", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, dark)))))
}
