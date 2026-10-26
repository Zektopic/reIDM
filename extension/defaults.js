// reIDM Integration — the settings' defaults, in one place.
//
// Loaded by the service worker (as a module import), by every content script (first in the
// manifest's list) and by the options page, so that the three can never disagree about what an
// unset setting means. A plain script rather than a module, because content scripts cannot be
// modules.

globalThis.REIDM_DEFAULTS = Object.freeze({
  port: 61002,
  capture: true,
  // Take download links on click, before the browser requests them. Only for links whose
  // address ends in one of `fileTypes`, or that carry a `download` attribute.
  interceptClicks: true,
  // Take every download the browser starts. Off: only those whose name matches `fileTypes`.
  captureAll: true,
  // The stock list.
  fileTypes:
    '3GP 7Z AAC ACE AIF APK ARJ ASF AVI BIN BZ2 DEB DMG EXE FLAC GZ GZIP IMG ISO LZH M4A M4V MKV MOV MP3 MP4 ' +
    'MPA MPE MPEG MPG MSI MSU OGG OGV PDF PKG PLJ PPS PPT QT RA RAR RM RMVB RPM SEA SIT SITX TAR TGZ TIF TIFF ' +
    'WAV WEBM WMA WMV XZ Z ZIP ZST',
  // Leave downloads smaller than this (KB) to the browser. 0 = take everything.
  minSizeKB: 0,
  // Never take downloads from these hosts (and their subdomains).
  skipHosts: [],
  captureIncognito: false,
  // Start reIDM through the native host when it is not running.
  launchReidm: true,
  // A desktop notification when reIDM takes a download, or can't.
  notifications: true,
  // The "Download file info" dialog before each download: Start, Download later, Cancel.
  askBeforeDownload: true,
  // Watch for video and audio: the network listeners, the page hook and the video button.
  sniffMedia: true,
  ignore: ['blob:', 'data:', 'chrome:', 'chrome-extension://', 'about:', 'file://', 'filesystem:'],
});
