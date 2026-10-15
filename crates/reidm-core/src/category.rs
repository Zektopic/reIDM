//! IDM's categories: each download is filed by its type into a folder of its own — Compressed,
//! Documents, Images, Music, Programs, Video — under the download folder unless the user moved
//! one. Anything else is *General* and goes to the download folder itself.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Category {
    General,
    Compressed,
    Documents,
    Images,
    Music,
    Programs,
    Video,
}

/// Every category, in the order the folder settings list them.
pub const ALL: [Category; 7] = [
    Category::General,
    Category::Compressed,
    Category::Documents,
    Category::Images,
    Category::Music,
    Category::Programs,
    Category::Video,
];

const COMPRESSED: &[&str] = &["7z", "ace", "arj", "bz2", "cab", "gz", "gzip", "lz", "lzh", "lzma", "r00", "r01", "rar", "sit", "sitx", "tar", "tbz", "tgz", "txz", "xz", "z", "zip", "zipx", "zst"];
const DOCUMENTS: &[&str] = &["azw3", "chm", "csv", "djvu", "doc", "docx", "epub", "md", "mobi", "odp", "ods", "odt", "pdf", "pps", "ppsx", "ppt", "pptx", "rtf", "txt", "xls", "xlsx", "xps"];
const IMAGES: &[&str] = &["avif", "bmp", "gif", "heic", "heif", "ico", "jfif", "jpeg", "jpg", "png", "psd", "raw", "svg", "tif", "tiff", "webp"];
const MUSIC: &[&str] = &["aac", "aif", "aiff", "alac", "ape", "flac", "m4a", "mka", "mp3", "mpa", "oga", "ogg", "opus", "ra", "wav", "wma"];
const PROGRAMS: &[&str] = &["apk", "appimage", "appx", "bin", "deb", "dmg", "exe", "img", "iso", "jar", "msi", "msix", "msu", "pkg", "rpm", "run", "sh", "xapk"];
const VIDEO: &[&str] = &["3g2", "3gp", "asf", "avi", "f4v", "flv", "m2ts", "m4v", "mkv", "mov", "mp4", "mpe", "mpeg", "mpg", "mts", "ogv", "qt", "rm", "rmvb", "ts", "vob", "webm", "wmv"];

impl Category {
    /// The category for a file name, by its extension.
    pub fn of(file_name: &str) -> Category {
        let ext = file_name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
        ALL.into_iter().find(|c| c.extensions().contains(&ext.as_str())).unwrap_or(Category::General)
    }

    /// The extensions that make a file this category, lower case.
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Category::General => &[],
            Category::Compressed => COMPRESSED,
            Category::Documents => DOCUMENTS,
            Category::Images => IMAGES,
            Category::Music => MUSIC,
            Category::Programs => PROGRAMS,
            Category::Video => VIDEO,
        }
    }

    /// Its name, which is also its folder's name under the download folder.
    pub fn name(self) -> &'static str {
        match self {
            Category::General => "General",
            Category::Compressed => "Compressed",
            Category::Documents => "Documents",
            Category::Images => "Images",
            Category::Music => "Music",
            Category::Programs => "Programs",
            Category::Video => "Video",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_are_filed_by_extension() {
        assert_eq!(Category::of("setup.EXE"), Category::Programs);
        assert_eq!(Category::of("talk 1080p.mp4"), Category::Video);
        assert_eq!(Category::of("song.m4a"), Category::Music);
        assert_eq!(Category::of("photos.tar.gz"), Category::Compressed);
        assert_eq!(Category::of("report.pdf"), Category::Documents);
        assert_eq!(Category::of("cat.webp"), Category::Images);
        assert_eq!(Category::of("README"), Category::General);
        assert_eq!(Category::of("data.json"), Category::General);
    }

    #[test]
    fn no_extension_is_in_two_categories() {
        let mut seen = std::collections::HashSet::new();
        for c in ALL {
            for e in c.extensions() {
                assert!(seen.insert(*e), "{e} is listed twice");
            }
        }
    }
}
