//! *Tasks › Add YouTube playlist…*: every video of a playlist (or a channel's videos), each with
//! the resolution and codec to download it in. One choice can be applied to all of them, or each
//! changed on its own; what each video really offers is looked up in the background and shown
//! beside it, a few videos at a time.

use crate::api::Client;
use crate::model::size_human;

/// Height of one video row, for laying out only those in view.
const ROW_HEIGHT: f32 = 30.0;
use crate::theme::Palette;
use egui::{RichText, Vec2};
use reidm_core::media::{audio_selector, selector, Playlist, PlaylistEntry, VideoCodec, VideoInfo};
use std::sync::mpsc::Receiver;

/// How many videos are looked into at once. Each is a yt-dlp run of a few seconds.
const LOOKUPS_AT_ONCE: usize = 3;

const HEIGHTS: [u32; 7] = [2160, 1440, 1080, 720, 480, 360, 240];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Res {
    Best,
    Up(u32),
    Audio,
}

impl Res {
    fn label(self) -> String {
        match self {
            Res::Best => "Best".into(),
            Res::Up(h) => format!("{h}p"),
            Res::Audio => "Audio only".into(),
        }
    }
    fn all() -> Vec<Res> {
        std::iter::once(Res::Best).chain(HEIGHTS.iter().map(|h| Res::Up(*h))).chain(std::iter::once(Res::Audio)).collect()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Audio {
    M4a,
    Mp3,
    Opus,
}

impl Audio {
    const ALL: [Audio; 3] = [Audio::M4a, Audio::Mp3, Audio::Opus];
    fn label(self) -> &'static str {
        match self {
            Audio::M4a => "M4A (AAC)",
            Audio::Mp3 => "MP3",
            Audio::Opus => "Opus",
        }
    }
    fn code(self) -> &'static str {
        match self {
            Audio::M4a => "m4a",
            Audio::Mp3 => "mp3",
            Audio::Opus => "opus",
        }
    }
}

/// One choice: a resolution, and a codec (video) or a format (sound only).
#[derive(Clone, Copy, PartialEq, Eq)]
struct Choice {
    res: Res,
    codec: VideoCodec,
    audio: Audio,
}

impl Choice {
    const PRESETS: [(&'static str, &'static str, Choice); 6] = [
        ("Best quality", "The highest resolution there is, in whatever codec it comes in", Choice { res: Res::Best, codec: VideoCodec::Any, audio: Audio::M4a }),
        ("1080p H.264", "Full HD that plays on every phone, TV and editor", Choice { res: Res::Up(1080), codec: VideoCodec::H264, audio: Audio::M4a }),
        ("720p H.264", "Smaller files that still look good, and play anywhere", Choice { res: Res::Up(720), codec: VideoCodec::H264, audio: Audio::M4a }),
        ("4K", "Up to 2160p — usually VP9 or AV1, which YouTube offers at that size", Choice { res: Res::Up(2160), codec: VideoCodec::Any, audio: Audio::M4a }),
        ("Audio M4A", "Just the sound, as YouTube serves it (no conversion)", Choice { res: Res::Audio, codec: VideoCodec::Any, audio: Audio::M4a }),
        ("Audio MP3", "Just the sound, converted to MP3", Choice { res: Res::Audio, codec: VideoCodec::Any, audio: Audio::Mp3 }),
    ];

    /// What to send reIDM: a yt-dlp format selector and, for converted sound, the format.
    fn media(self) -> serde_json::Value {
        match self.res {
            Res::Audio => {
                let a = self.audio.code();
                // M4A is how YouTube serves its sound already; the others are converted.
                let convert = (self.audio != Audio::M4a).then_some(a);
                serde_json::json!({ "format": audio_selector(a), "audio": convert })
            }
            Res::Best => serde_json::json!({ "format": selector(None, self.codec) }),
            Res::Up(h) => serde_json::json!({ "format": selector(Some(h), self.codec) }),
        }
    }

    fn codec_label(self) -> &'static str {
        if self.res == Res::Audio {
            self.audio.label()
        } else {
            self.codec.label()
        }
    }
}

struct Row {
    entry: PlaylistEntry,
    on: bool,
    choice: Choice,
    info: Option<Result<VideoInfo, String>>,
    lookup: Option<Receiver<Result<VideoInfo, String>>>,
}

impl Row {
    /// What this row will actually get, from what the video offers: "1080p H.264 · ~240 MB".
    fn outcome(&self) -> String {
        let Some(Ok(info)) = &self.info else { return String::new() };
        if self.choice.res == Res::Audio {
            return format!("sound · {}", self.choice.audio.label());
        }
        let cap = match self.choice.res {
            Res::Up(h) => h,
            _ => u32::MAX,
        };
        // As the selector chooses: the tallest at most `cap` in the codec asked for; failing
        // that, the tallest at most `cap` in any codec; failing that, the smallest there is.
        let fits = || info.heights.iter().filter(|h| h.height <= cap);
        let wanted = self.choice.codec;
        let in_codec = (wanted != VideoCodec::Any).then(|| fits().find(|h| h.codecs.contains(&wanted))).flatten();
        let Some(hi) = in_codec.or_else(|| fits().next()).or(info.heights.last()) else { return "no video formats".into() };
        let codec = if in_codec.is_some() { Some(wanted) } else { hi.codecs.last().copied() };
        let mut s = format!("{}p", hi.height);
        if let Some(c) = codec {
            s.push_str(&format!(" {}", c.label()));
        }
        if wanted != VideoCodec::Any && in_codec.is_none() {
            s.push_str(&format!(" (no {})", wanted.label()));
        }
        if let Some(sz) = hi.size {
            s.push_str(&format!(" · ~{}", size_human(sz)));
        }
        s
    }

    /// The file's name, after the video as IDM names it; yt-dlp sets the real extension.
    fn file_name(&self) -> String {
        let ext = if self.choice.res == Res::Audio { self.choice.audio.code() } else { "mp4" };
        format!("{}.{ext}", reidm_core::sanitize_filename(&self.entry.title))
    }

    fn offers(&self) -> String {
        match &self.info {
            None => "checking…".into(),
            Some(Err(_)) => "could not check".into(),
            Some(Ok(i)) => match i.heights.first() {
                Some(top) => {
                    let mut codecs: Vec<&str> = Vec::new();
                    for h in &i.heights {
                        for c in &h.codecs {
                            if !codecs.contains(&c.label()) {
                                codecs.push(c.label());
                            }
                        }
                    }
                    format!("up to {}p · {}", top.height, codecs.join(", "))
                }
                None => "sound only".into(),
            },
        }
    }
}

#[derive(Default)]
pub struct PlaylistWindow {
    pub open: bool,
    url: String,
    loading: Option<Receiver<Result<Playlist, String>>>,
    error: Option<String>,
    title: String,
    rows: Vec<Row>,
    all: Option<Choice>,
    own_folder: bool,
    /// The Video and Music folders, for "a folder named after the playlist".
    video_dir: String,
    music_dir: String,
    sending: Option<Receiver<(usize, Vec<String>)>>,
}

impl PlaylistWindow {
    /// Waiting on the service, so the window must keep drawing to notice the answer.
    pub fn busy(&self) -> bool {
        self.loading.is_some() || self.sending.is_some() || self.rows.iter().any(|r| r.lookup.is_some())
    }
}

impl PlaylistWindow {
    pub fn open_with(&mut self, url: Option<String>, video_dir: String, music_dir: String) {
        self.open = true;
        self.own_folder = true;
        self.video_dir = video_dir;
        self.music_dir = music_dir;
        if let Some(u) = url {
            self.url = u;
        }
    }

    /// Draw the window. Returns a message for the status bar when something was queued.
    pub fn show(&mut self, ctx: &egui::Context, api: &Client, p: &Palette) -> Option<String> {
        if !self.open {
            return None;
        }
        self.poll(api);
        let mut open = true;
        let mut queue: Option<bool> = None; // Some(later)
        egui::Window::new("Add YouTube playlist")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size(Vec2::new(960.0, 620.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.url).hint_text("https://www.youtube.com/playlist?list=…  (or a channel's /videos page)").desired_width(720.0));
                    let busy = self.loading.is_some();
                    if ui.add_enabled(!busy && !self.url.trim().is_empty(), egui::Button::new("List videos")).clicked() {
                        self.error = None;
                        self.rows.clear();
                        self.loading = Some(api.playlist_async(self.url.trim()));
                    }
                });
                if self.loading.is_some() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Listing the videos… (the first time, reIDM gets yt-dlp first; that takes a minute)");
                    });
                }
                if let Some(e) = &self.error {
                    ui.label(RichText::new(e).color(p.err));
                }
                if self.rows.is_empty() {
                    return;
                }
                ui.add_space(4.0);
                let picked = self.rows.iter().filter(|r| r.on).count();
                let checked = self.rows.iter().filter(|r| r.info.is_some()).count();
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&self.title).strong().size(15.0));
                    ui.label(RichText::new(format!("{} videos · {picked} selected · {checked} checked", self.rows.len())).color(p.text_dim));
                });
                ui.add_space(4.0);

                // One choice for every video: presets, or resolution and codec by hand.
                ui.horizontal_wrapped(|ui| {
                    ui.label("For every selected video:");
                    for (name, tip, c) in Choice::PRESETS {
                        if ui.button(name).on_hover_text(tip).clicked() {
                            self.set_all(c);
                        }
                    }
                });
                ui.horizontal(|ui| {
                    let mut c = self.all.unwrap_or(Choice::PRESETS[1].2);
                    ui.label("or");
                    choice_editor(ui, "all", &mut c);
                    self.all = Some(c);
                    if ui.button("Apply to selected").clicked() {
                        self.set_all(c);
                    }
                    ui.separator();
                    if ui.button("Select all").clicked() {
                        self.rows.iter_mut().for_each(|r| r.on = true);
                    }
                    if ui.button("Select none").clicked() {
                        self.rows.iter_mut().for_each(|r| r.on = false);
                    }
                });
                ui.separator();

                let footer = 74.0;
                // Only the rows in view are laid out: a channel can list thousands of videos,
                // each with two drop-downs.
                let n = self.rows.len();
                egui::ScrollArea::vertical().max_height((ui.available_height() - footer).max(160.0)).auto_shrink([false, false]).show_rows(ui, ROW_HEIGHT, n, |ui, range| {
                    egui::Grid::new("playlist").num_columns(7).striped(true).spacing(Vec2::new(10.0, 6.0)).min_row_height(ROW_HEIGHT - 6.0).show(ui, |ui| {
                        if range.start == 0 {
                            for h in ["", "#", "Title", "Length", "Available", "Download as", "You get"] {
                                ui.label(RichText::new(h).color(p.text_dim).size(12.0));
                            }
                            ui.end_row();
                        }
                        let start = range.start;
                        for (i, r) in self.rows[range].iter_mut().enumerate().map(|(k, r)| (k + start, r)) {
                            ui.checkbox(&mut r.on, "");
                            ui.label(RichText::new(format!("{}", i + 1)).color(p.text_dim));
                            ui.horizontal(|ui| {
                                ui.set_width(280.0);
                                ui.add(egui::Label::new(truncate(&r.entry.title, 60)).truncate()).on_hover_text(&r.entry.title);
                            });
                            ui.label(r.entry.duration.map(duration).unwrap_or_default());
                            ui.label(RichText::new(r.offers()).color(p.text_dim).size(12.0));
                            ui.horizontal(|ui| choice_editor(ui, &format!("row{i}"), &mut r.choice));
                            ui.label(RichText::new(r.outcome()).size(12.0));
                            ui.end_row();
                        }
                    });
                });

                ui.separator();
                let folder = self.folder_preview();
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.own_folder, "Save in a folder named after the playlist");
                    if let Some(f) = &folder {
                        ui.label(RichText::new(f).color(p.text_dim).size(12.0));
                    }
                });
                ui.horizontal(|ui| {
                    let ready = picked > 0 && self.sending.is_none();
                    if ui.add_enabled(ready, egui::Button::new(RichText::new(format!("Download {picked}")).strong())).clicked() {
                        queue = Some(false);
                    }
                    if ui.add_enabled(ready, egui::Button::new("Download later")).on_hover_text("Into the scheduled queue, to start with the scheduler").clicked() {
                        queue = Some(true);
                    }
                    if self.sending.is_some() {
                        ui.spinner();
                        ui.label("Adding to reIDM…");
                    }
                });
            });
        if let Some(later) = queue {
            self.send(api, later);
        }
        if !open {
            self.open = false;
        }
        // The result of sending, once it is in.
        if let Some(rx) = &self.sending {
            if let Ok((n, errors)) = rx.try_recv() {
                self.sending = None;
                return Some(if errors.is_empty() {
                    self.open = false;
                    format!("{n} video{} added to reIDM", if n == 1 { "" } else { "s" })
                } else {
                    self.error = Some(format!("{} could not be added: {}", errors.len(), errors[0]));
                    format!("{n} added, {} failed", errors.len())
                });
            }
        }
        None
    }

    fn set_all(&mut self, c: Choice) {
        self.all = Some(c);
        for r in self.rows.iter_mut().filter(|r| r.on) {
            r.choice = c;
        }
    }

    fn folder_name(&self) -> String {
        reidm_core::sanitize_filename(&self.title)
    }

    fn folder_preview(&self) -> Option<String> {
        if !self.own_folder {
            return None;
        }
        let audio = self.rows.iter().filter(|r| r.on).all(|r| r.choice.res == Res::Audio);
        let base = if audio { &self.music_dir } else { &self.video_dir };
        (!base.is_empty()).then(|| std::path::Path::new(base).join(self.folder_name()).to_string_lossy().into_owned())
    }

    /// The listing, and each video's lookup, as they come in; and start the next lookups.
    fn poll(&mut self, api: &Client) {
        if let Some(rx) = &self.loading {
            if let Ok(r) = rx.try_recv() {
                self.loading = None;
                match r {
                    Ok(pl) => {
                        self.title = pl.title;
                        let c = self.all.unwrap_or(Choice::PRESETS[1].2);
                        self.rows = pl.entries.into_iter().map(|entry| Row { entry, on: true, choice: c, info: None, lookup: None }).collect();
                    }
                    Err(e) => self.error = Some(e),
                }
            }
        }
        for r in &mut self.rows {
            if let Some(rx) = &r.lookup {
                if let Ok(res) = rx.try_recv() {
                    r.info = Some(res);
                    r.lookup = None;
                }
            }
        }
        let running = self.rows.iter().filter(|r| r.lookup.is_some()).count();
        for r in self.rows.iter_mut().filter(|r| r.info.is_none() && r.lookup.is_none()).take(LOOKUPS_AT_ONCE.saturating_sub(running)) {
            r.lookup = Some(api.video_info_async(&r.entry.url));
        }
    }

    fn send(&mut self, api: &Client, later: bool) {
        let folder = self.own_folder.then(|| self.folder_preview()).flatten();
        let items: Vec<(String, serde_json::Value, String)> = self.rows.iter().filter(|r| r.on).map(|r| (r.entry.url.clone(), r.choice.media(), r.file_name())).collect();
        self.sending = Some(api.add_videos_async(items, folder, later));
    }
}

/// Resolution, then codec (or, for sound only, its format).
fn choice_editor(ui: &mut egui::Ui, id: &str, c: &mut Choice) {
    egui::ComboBox::from_id_salt(format!("{id}-res")).width(96.0).selected_text(c.res.label()).show_ui(ui, |ui| {
        for r in Res::all() {
            ui.selectable_value(&mut c.res, r, r.label());
        }
    });
    let label = c.codec_label();
    egui::ComboBox::from_id_salt(format!("{id}-codec")).width(120.0).selected_text(label).show_ui(ui, |ui| {
        if c.res == Res::Audio {
            for a in Audio::ALL {
                ui.selectable_value(&mut c.audio, a, a.label());
            }
        } else {
            for v in VideoCodec::ALL {
                ui.selectable_value(&mut c.codec, v, v.label());
            }
        }
    });
}

fn duration(secs: f64) -> String {
    let s = secs as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n - 1).collect::<String>())
    }
}
