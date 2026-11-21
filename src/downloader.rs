use crate::config;
use crate::engines;
use crate::types::{
    build_album_relative_path, clean_artist_name, is_bandcamp_url, is_yt_music_url,
    strip_leading_artist_from_album, CastDevice, CastSessionState, DownloadItem, DownloadRequest,
    MediaInfo, PlaylistEntry, SearchBatch,
};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::{mpsc, Mutex};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub enum UiEvent {
    EngineStatus(crate::types::EngineStatus),
    EngineActionError(String),
    InspectSuccess(u64, MediaInfo),
    InspectError(u64, String),
    SearchSuccess(u64, String, SearchBatch),
    SearchError(u64, String),
    SearchMoreSuccess(u64, SearchBatch),
    SearchMoreError(u64, String),
    MainSearchSuccess(u64, String, SearchBatch),
    MainSearchError(u64, String),
    MainSearchMoreSuccess(u64, SearchBatch),
    MainSearchMoreError(u64, String),
    DownloadUpdated(DownloadItem),
    ThumbnailLoaded(String, Vec<u8>),
    ThumbnailFailed(String),
    CastDevicesUpdated(Vec<CastDevice>),
    CastDiscoveryDone(Option<String>),
    CastStateUpdated(u64, Option<CastSessionState>),
    CastError(u64, String),
}

pub struct DownloadManager {
    pub items: Arc<Mutex<Vec<DownloadItem>>>,
    pub active_pids: Arc<Mutex<HashMap<String, u32>>>,
}

impl DownloadManager {
    pub fn new() -> Self {
        let mut initial = Vec::new();
        if let Ok(content) = fs::read_to_string(config::history_file_path()) {
            if let Ok(mut parsed) = serde_json::from_str::<Vec<DownloadItem>>(&content) {
                for item in &mut parsed {
                    if item.status == "downloading"
                        || item.status == "queued"
                        || item.status == "processing"
                    {
                        item.status = "cancelled".to_string();
                        item.error = Some("Application closed while downloading".to_string());
                    }
                }
                initial = parsed;
            }
        }
        Self {
            items: Arc::new(Mutex::new(initial)),
            active_pids: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn persist(&self) {
        let items = self.items.lock().await;
        let dir = config::data_dir();
        let _ = fs::create_dir_all(&dir);
        if let Ok(json) = serde_json::to_string_pretty(&*items) {
            let _ = fs::write(config::history_file_path(), json);
        }
    }
}

pub fn append_cookie_args(args: &mut Vec<String>) {
    let cookies_path = config::cookies_file_path();
    if cookies_path.exists() {
        args.push("--cookies".to_string());
        args.push(cookies_path.to_string_lossy().to_string());
        return;
    }
    let settings = config::load_settings();
    if !settings.cookie_browser.is_empty() && settings.cookie_browser != "none" {
        args.push("--cookies-from-browser".to_string());
        args.push(settings.cookie_browser);
    }
}

#[cfg(target_os = "windows")]
const CP1250_HIGH: [char; 128] = [
    '€', '\u{FFFD}', '‚', '\u{FFFD}', '„', '…', '†', '‡', '\u{FFFD}', '‰', 'Š', '‹', 'Ś', 'Ť', 'Ž', 'Ź',
    '\u{FFFD}', '‘', '’', '“', '”', '•', '–', '—', '\u{FFFD}', '™', 'š', '›', 'ś', 'ť', 'ž', 'ź',
    '\u{00A0}', 'ˇ', '˘', 'Ł', '¤', 'Ą', '¦', '§', '¨', '©', 'Ş', '«', '¬', '\u{00AD}', '®', 'Ż',
    '°', '±', '˛', 'ł', '´', 'µ', '¶', '·', '¸', 'ą', 'ş', '»', 'Ľ', '˝', 'ľ', 'ż',
    'Ŕ', 'Á', 'Â', 'Ă', 'Ä', 'Ĺ', 'Ć', 'Ç', 'Č', 'É', 'Ę', 'Ë', 'Ě', 'Í', 'Î', 'Ď',
    'Đ', 'Ń', 'Ň', 'Ó', 'Ô', 'Ő', 'Ö', '×', 'Ř', 'Ů', 'Ú', 'Ű', 'Ü', 'Ý', 'Ţ', 'ß',
    'ŕ', 'á', 'â', 'ă', 'ä', 'ĺ', 'ć', 'ç', 'č', 'é', 'ę', 'ë', 'ě', 'í', 'î', 'ď',
    'đ', 'ń', 'ň', 'ó', 'ô', 'ő', 'ö', '÷', 'ř', 'ů', 'ú', 'ű', 'ü', 'ý', 'ţ', '˙',
];

pub fn decode_ytdlp_bytes(bytes: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    #[cfg(target_os = "windows")]
    {
        return bytes
            .iter()
            .map(|&b| {
                if b < 0x80 {
                    b as char
                } else {
                    CP1250_HIGH[(b - 0x80) as usize]
                }
            })
            .collect();
    }
    #[cfg(not(target_os = "windows"))]
    {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

pub fn format_ytdlp_error(raw: &str) -> String {
    if raw.contains("Sign in to confirm") || raw.contains("not a bot") {
        return "[BOT_CHECK] YouTube requested bot verification.".to_string();
    }
    if raw.contains("could not find") && raw.contains("cookies database") {
        return "[NO_BROWSER_DB] Selected browser cookies database not found.".to_string();
    }
    if raw.contains("no impersonate target is available") || raw.contains("attempting impersonation") {
        return format!(
            "{}\n\nHint (Linux): Install python-curl-cffi (e.g. 'sudo pacman -S python-curl-cffi') or click 'Update yt-dlp' in Settings.",
            raw.trim()
        );
    }
    if raw.trim().is_empty() {
        "Failed to process the provided link.".to_string()
    } else {
        raw.trim().to_string()
    }
}


fn parse_height_from_format(fmt: &serde_json::Value) -> Option<u32> {
    let ext = fmt.get("ext").and_then(|v| v.as_str()).unwrap_or("");
    let vcodec = fmt.get("vcodec").and_then(|v| v.as_str()).unwrap_or("");
    if ext.eq_ignore_ascii_case("mhtml") || vcodec.eq_ignore_ascii_case("none") {
        return None;
    }
    if let Some(h) = fmt.get("height").and_then(|v| v.as_u64()) {
        if h >= 144 {
            return Some(h as u32);
        }
    }
    if let Some(res) = fmt.get("resolution").and_then(|v| v.as_str()) {
        if let Some((_, h_str)) = res.split_once('x') {
            if let Ok(h) = h_str.trim().parse::<u32>() {
                if h >= 144 {
                    return Some(h);
                }
            }
        }
    }
    None
}

struct AnalyzedFormats {
    heights: Vec<u32>,
    has_flac: bool,
    video_formats: Vec<String>,
    audio_formats: Vec<String>,
    audio_qualities: Vec<String>,
}

fn parse_bitrate_hint(fmt: &serde_json::Value) -> f64 {
    for key in ["format_id", "format", "format_note"] {
        if let Some(s) = fmt.get(key).and_then(|v| v.as_str()) {
            let lower = s.to_ascii_lowercase();
            for rate in [320.0, 256.0, 192.0, 160.0, 128.0, 96.0, 64.0] {
                let tag = format!("{}", rate as u32);
                if lower.contains(&format!("-{}", tag))
                    || lower.contains(&format!("{}k", tag))
                    || lower == tag
                {
                    return rate;
                }
            }
        }
    }
    0.0
}

fn analyze_formats(formats: &[serde_json::Value], is_bandcamp: bool) -> AnalyzedFormats {
    let mut heights: Vec<u32> = formats
        .iter()
        .filter_map(parse_height_from_format)
        .collect();
    heights.sort_unstable_by(|a, b| b.cmp(a));
    heights.dedup();

    let mut has_video_stream = false;
    let mut has_v_mp4 = false;
    let mut has_v_webm = false;
    let mut has_v_mkv = false;

    let mut has_audio_stream = false;
    let mut has_flac = false;
    let mut has_wav = false;
    let mut has_m4a = false;
    let mut has_opus = false;
    let mut has_opus_hq = false;
    let mut max_abr: f64 = 0.0;

    for fmt in formats {
        let ext = fmt
            .get("ext")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext == "mhtml" {
            continue;
        }
        let vcodec = fmt
            .get("vcodec")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let acodec = fmt
            .get("acodec")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        let is_video = vcodec != "none"
            && (parse_height_from_format(fmt).is_some() || (!vcodec.is_empty() && vcodec != "none"));
        if is_video {
            has_video_stream = true;
            match ext.as_str() {
                "mp4" | "m4v" | "mov" => has_v_mp4 = true,
                "webm" => has_v_webm = true,
                "mkv" => has_v_mkv = true,
                _ => {}
            }
        }

        let is_audio_ext = matches!(
            ext.as_str(),
            "mp3" | "m4a" | "aac" | "opus" | "ogg" | "oga" | "flac" | "wav" | "alac" | "webm"
        );
        let is_audio = (!acodec.is_empty() && acodec != "none")
            || (vcodec == "none" && is_audio_ext);
        if is_audio {
            has_audio_stream = true;
            if acodec == "flac" || acodec == "alac" || ext == "flac" {
                has_flac = true;
            }
            if ext == "wav" || acodec.starts_with("pcm") {
                has_wav = true;
                has_flac = true;
            }
            if matches!(ext.as_str(), "m4a" | "aac" | "mp4")
                || acodec.starts_with("mp4a")
                || acodec == "aac"
            {
                has_m4a = true;
            }
            if ext == "opus" || ext == "webm" || acodec == "opus" {
                has_opus = true;
            }

            let mut abr = fmt
                .get("abr")
                .and_then(|v| v.as_f64())
                .or_else(|| {
                    if vcodec == "none" || vcodec.is_empty() {
                        fmt.get("tbr").and_then(|v| v.as_f64())
                    } else {
                        None
                    }
                })
                .unwrap_or(0.0);
            if abr <= 0.0 {
                abr = parse_bitrate_hint(fmt);
            }
            if abr > max_abr {
                max_abr = abr;
            }
            if acodec == "opus" && abr >= 115.0 {
                has_opus_hq = true;
            }
        }
    }

    let mut video_formats = Vec::new();
    if has_video_stream {
        if has_v_mp4 {
            video_formats.push("mp4".to_string());
        }
        if has_v_mkv || (has_v_mp4 && has_v_webm) {
            video_formats.push("mkv".to_string());
        }
        if has_v_webm {
            video_formats.push("webm".to_string());
        }
        if video_formats.is_empty() {
            video_formats.push("mp4".to_string());
        }
    }

    let mut audio_formats = Vec::new();
    if has_audio_stream {
        audio_formats.push("mp3".to_string());
        if has_flac {
            audio_formats.push("flac".to_string());
        }
        if has_m4a {
            audio_formats.push("m4a".to_string());
        }
        if has_opus {
            audio_formats.push("opus".to_string());
        }
        if has_wav || has_flac {
            audio_formats.push("wav".to_string());
        }
    }

    let audio_qualities = if !has_audio_stream {
        Vec::new()
    } else if has_flac || has_opus_hq || max_abr >= 280.0 {
        vec![
            "320".to_string(),
            "256".to_string(),
            "192".to_string(),
            "128".to_string(),
        ]
    } else if max_abr >= 220.0 {
        vec!["256".to_string(), "192".to_string(), "128".to_string()]
    } else if max_abr >= 150.0 {
        vec!["192".to_string(), "128".to_string()]
    } else if max_abr >= 100.0 {
        vec!["128".to_string()]
    } else if max_abr > 0.0 {
        vec![format!("{}", (max_abr.round() as u32).max(64))]
    } else if is_bandcamp {
        vec!["128".to_string()]
    } else {
        vec![
            "320".to_string(),
            "256".to_string(),
            "192".to_string(),
            "128".to_string(),
        ]
    };

    AnalyzedFormats {
        heights,
        has_flac,
        video_formats,
        audio_formats,
        audio_qualities,
    }
}

pub async fn inspect_media(url: String) -> Result<MediaInfo, String> {
    let mut bin = if is_bandcamp_url(&url) {
        engines::ensure_ytdlp_with_impersonation().await?
    } else {
        engines::ensure_ytdlp_binary().await?
    };
    let _ = engines::ensure_js_runtime_if_needed().await;

    let mut out_opt = None;
    let mut ejs_args = Vec::new();
    for attempt in 0..2 {
        ejs_args = engines::ytdlp_ejs_args(&bin).await;
        let mut args = vec![
            "--encoding".to_string(),
            "utf-8".to_string(),
            "--dump-single-json".to_string(),
            "--no-warnings".to_string(),
            "--skip-download".to_string(),
            "--flat-playlist".to_string(),
        ];
        args.extend(ejs_args.iter().cloned());
        append_cookie_args(&mut args);
        args.push(url.clone());

        let mut cmd = Command::new(&bin);
        engines::configure_ytdlp_command(&mut cmd);
        cmd.args(&args).stdout(Stdio::piped()).stderr(Stdio::piped());

        #[cfg(target_os = "windows")]
        cmd.creation_flags(0x08000000);

        let out = tokio::time::timeout(std::time::Duration::from_secs(35), cmd.output())
            .await
            .map_err(|_| "Timed out while inspecting the link.".to_string())?
            .map_err(|e| format!("Failed to start yt-dlp: {}", e))?;

        if out.status.success() {
            out_opt = Some(out);
            break;
        }

        let err = decode_ytdlp_bytes(&out.stderr);
        if attempt == 0
            && (err.contains("no impersonate target is available")
                || err.contains("attempting impersonation"))
        {
            if let Ok(upgraded) = engines::ensure_ytdlp_with_impersonation().await {
                if upgraded != bin {
                    bin = upgraded;
                    continue;
                }
            }
        }
        return Err(format_ytdlp_error(&err));
    }

    let out = out_opt.ok_or_else(|| "Failed to inspect the link.".to_string())?;

    let val: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("Metadata parse error: {}", e))?;

    let raw_entries = val.get("entries").and_then(|e| e.as_array());
    let is_playlist =
        val.get("_type").and_then(|v| v.as_str()) == Some("playlist") || raw_entries.is_some();
    let playlist_count = raw_entries.map(|a| a.len());

    let entries: Vec<PlaylistEntry> = raw_entries
        .map(|arr| {
            arr.iter()
                .enumerate()
                .map(|(idx, entry)| {
                    let item_title = entry
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown track")
                        .to_string();
                    let raw_item_url = entry
                        .get("url")
                        .or_else(|| entry.get("webpage_url"))
                        .and_then(|v| v.as_str())
                        .unwrap_or(&url);
                    let item_url = if raw_item_url.starts_with('/') {
                        if let Some(scheme_end) = url.find("://") {
                            let rest = &url[scheme_end + 3..];
                            let host_end = rest.find('/').unwrap_or(rest.len());
                            format!("{}{}", &url[..scheme_end + 3 + host_end], raw_item_url)
                        } else {
                            raw_item_url.to_string()
                        }
                    } else {
                        raw_item_url.to_string()
                    };
                    PlaylistEntry {
                        index: idx + 1,
                        title: item_title,
                        duration: entry.get("duration").and_then(|d| d.as_f64()),
                        url: item_url,
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let mut formats = val
        .get("formats")
        .and_then(|f| f.as_array())
        .cloned()
        .unwrap_or_default();

    let mut sample_thumb: Option<String> = None;
    let mut sample_val_opt: Option<serde_json::Value> = None;

    let extractor_name = val
        .get("extractor_key")
        .or_else(|| val.get("extractor"))
        .and_then(|v| v.as_str())
        .unwrap_or("Web")
        .to_string();
    let is_bandcamp_source =
        is_bandcamp_url(&url) || extractor_name.to_ascii_lowercase().contains("bandcamp");

    let is_valid_artist_val = |v: Option<&serde_json::Value>| -> bool {
        v.and_then(|x| x.as_str())
            .map(|s| !clean_artist_name(s).is_empty())
            .unwrap_or(false)
    };

    let has_playlist_artist = is_valid_artist_val(val.get("album_artist"))
        || is_valid_artist_val(val.get("artist"))
        || is_valid_artist_val(val.get("playlist_uploader"))
        || is_valid_artist_val(val.get("uploader"))
        || is_valid_artist_val(val.get("channel"))
        || raw_entries
            .and_then(|arr| arr.first())
            .map(|e| {
                is_valid_artist_val(e.get("album_artist"))
                    || is_valid_artist_val(e.get("artist"))
            })
            .unwrap_or(false);

    if formats.is_empty() || (is_playlist && !has_playlist_artist) {
        if let Some(first_entry) = raw_entries.and_then(|arr| arr.first()) {
            if formats.is_empty() {
                if let Some(entry_fmts) = first_entry.get("formats").and_then(|f| f.as_array()) {
                    formats = entry_fmts.clone();
                }
            }
            if is_playlist && (formats.is_empty() || !has_playlist_artist) {
                let sample_url = first_entry
                    .get("webpage_url")
                    .or_else(|| first_entry.get("url"))
                    .and_then(|v| v.as_str())
                    .map(|s| {
                        if s.starts_with('/') {
                            if let Some(scheme_end) = url.find("://") {
                                let rest = &url[scheme_end + 3..];
                                let host_end = rest.find('/').unwrap_or(rest.len());
                                format!("{}{}", &url[..scheme_end + 3 + host_end], s)
                            } else {
                                s.to_string()
                            }
                        } else {
                            s.to_string()
                        }
                    })
                    .filter(|s| s.starts_with("http://") || s.starts_with("https://"))
                    .or_else(|| {
                        let lower_url = url.to_ascii_lowercase();
                        if lower_url.contains("youtube.com") || lower_url.contains("youtu.be") {
                            first_entry
                                .get("id")
                                .and_then(|v| v.as_str())
                                .map(|vid| format!("https://www.youtube.com/watch?v={}", vid))
                        } else {
                            None
                        }
                    });

                if let Some(sample) = sample_url {
                    let mut sample_args = vec![
                        "--encoding".to_string(),
                        "utf-8".to_string(),
                        "--dump-single-json".to_string(),
                        "--no-warnings".to_string(),
                        "--skip-download".to_string(),
                        "--no-playlist".to_string(),
                    ];
                    sample_args.extend(ejs_args.iter().cloned());
                    append_cookie_args(&mut sample_args);
                    sample_args.push(sample);

                    let mut sample_cmd = Command::new(&bin);
                    engines::configure_ytdlp_command(&mut sample_cmd);
                    sample_cmd
                        .args(&sample_args)
                        .stdout(Stdio::piped())
                        .stderr(Stdio::null());
                    #[cfg(target_os = "windows")]
                    sample_cmd.creation_flags(0x08000000);

                    if let Ok(Ok(sample_out)) = tokio::time::timeout(
                        std::time::Duration::from_secs(20),
                        sample_cmd.output(),
                    )
                    .await
                    {
                        if sample_out.status.success() {
                            if let Ok(sample_val) =
                                serde_json::from_slice::<serde_json::Value>(&sample_out.stdout)
                            {
                                if formats.is_empty() {
                                    if let Some(sf) =
                                        sample_val.get("formats").and_then(|f| f.as_array())
                                    {
                                        formats = sf.clone();
                                    }
                                }
                                sample_thumb = sample_val
                                    .get("thumbnail")
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string());
                                sample_val_opt = Some(sample_val);
                            }
                        }
                    }
                }
            }
        }
    }

    let analyzed = analyze_formats(&formats, is_bandcamp_source);

    let thumbnail = sample_thumb
        .or_else(|| {
            val.get("thumbnail")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .or_else(|| {
            val.get("thumbnails")
                .and_then(|t| t.as_array())
                .and_then(|arr| arr.last())
                .and_then(|last| last.get("url"))
                .and_then(|u| u.as_str())
                .map(|s| s.to_string())
        })
        .or_else(|| {
            raw_entries
                .and_then(|arr| arr.first())
                .and_then(|e| {
                    e.get("thumbnail")
                        .and_then(|v| v.as_str())
                        .or_else(|| {
                            e.get("thumbnails")
                                .and_then(|t| t.as_array())
                                .and_then(|arr| arr.last())
                                .and_then(|last| last.get("url"))
                                .and_then(|u| u.as_str())
                        })
                })
                .map(|s| s.to_string())
        })
        .or_else(|| {
            let lower_url = url.to_ascii_lowercase();
            if lower_url.contains("youtube.com") || lower_url.contains("youtu.be") {
                raw_entries
                    .and_then(|arr| arr.first())
                    .and_then(|e| e.get("id").and_then(|v| v.as_str()))
                    .map(|vid| format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", vid))
            } else {
                None
            }
        });

    let webpage_url = val
        .get("webpage_url")
        .and_then(|v| v.as_str())
        .unwrap_or(&url)
        .to_string();

    let raw_title = val
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown Title");

    let sample_ref = sample_val_opt.as_ref();

    let clean_json_artist = |v: Option<&serde_json::Value>, split_commas: bool| -> Option<String> {
        let val_ref = v?;
        if let Some(s) = val_ref.as_str() {
            let candidate = if split_commas && s.contains(", ") {
                let parts: Vec<&str> = s.split(", ").collect();
                let is_single_band_with_amp = parts.len() == 2 && parts[1].contains(" & ");
                if is_single_band_with_amp {
                    s
                } else {
                    parts[0]
                }
            } else {
                s
            };
            let cleaned = clean_artist_name(candidate);
            if !cleaned.is_empty() {
                return Some(cleaned);
            }
        } else if let Some(arr) = val_ref.as_array() {
            for item in arr {
                if let Some(s) = item.as_str() {
                    let cleaned = clean_artist_name(s);
                    if !cleaned.is_empty() {
                        return Some(cleaned);
                    }
                }
            }
        }
        None
    };

    let majority_entry_artist = || -> Option<String> {
        let arr = raw_entries?;
        let mut counts: HashMap<String, usize> = HashMap::new();
        let mut first_seen: Option<String> = None;
        for entry in arr.iter().take(20) {
            let found = clean_json_artist(entry.get("album_artist"), false)
                .or_else(|| clean_json_artist(entry.get("artists"), false))
                .or_else(|| clean_json_artist(entry.get("artist"), true))
                .or_else(|| clean_json_artist(entry.get("uploader"), false))
                .or_else(|| clean_json_artist(entry.get("channel"), false));
            if let Some(name) = found {
                if first_seen.is_none() {
                    first_seen = Some(name.clone());
                }
                *counts.entry(name).or_insert(0) += 1;
            }
        }
        counts
            .into_iter()
            .max_by_key(|(_, c)| *c)
            .map(|(name, _)| name)
            .or(first_seen)
    };

    let album_artist_val = clean_json_artist(val.get("album_artist"), false)
        .or_else(|| clean_json_artist(val.get("artists"), false))
        .or_else(|| clean_json_artist(val.get("artist"), true))
        .or_else(|| clean_json_artist(val.get("playlist_uploader"), false))
        .or_else(|| clean_json_artist(val.get("playlist_creator"), false))
        .or_else(|| clean_json_artist(val.get("uploader"), false))
        .or_else(|| clean_json_artist(val.get("channel"), false))
        .or_else(|| clean_json_artist(val.get("creator"), false))
        .or_else(|| {
            raw_entries.and_then(|arr| arr.first()).and_then(|e| {
                clean_json_artist(e.get("album_artist"), false)
                    .or_else(|| clean_json_artist(e.get("artists"), false))
                    .or_else(|| clean_json_artist(e.get("artist"), true))
            })
        })
        .or_else(|| {
            sample_ref.and_then(|s| {
                clean_json_artist(s.get("album_artist"), false)
                    .or_else(|| clean_json_artist(s.get("artists"), false))
                    .or_else(|| clean_json_artist(s.get("artist"), true))
            })
        })
        .or_else(majority_entry_artist)
        .or_else(|| {
            sample_ref.and_then(|s| {
                clean_json_artist(s.get("uploader"), false)
                    .or_else(|| clean_json_artist(s.get("channel"), false))
                    .or_else(|| clean_json_artist(s.get("creator"), false))
            })
        });

    let raw_uploader = album_artist_val
        .clone()
        .or_else(|| {
            val.get("playlist_uploader")
                .or_else(|| val.get("uploader"))
                .or_else(|| val.get("channel"))
                .or_else(|| val.get("creator"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "Unknown Artist".to_string());

    let artist_for_strip = album_artist_val.as_deref().unwrap_or(&raw_uploader);

    let album_val = val
        .get("album")
        .or_else(|| {
            raw_entries
                .and_then(|arr| arr.first())
                .and_then(|e| e.get("album"))
        })
        .or_else(|| sample_ref.and_then(|s| s.get("album")))
        .or_else(|| val.get("playlist_title"))
        .and_then(|v| v.as_str())
        .map(|s| strip_leading_artist_from_album(artist_for_strip, s))
        .filter(|s| !s.is_empty())
        .or_else(|| {
            if is_playlist {
                let cleaned = strip_leading_artist_from_album(artist_for_strip, raw_title);
                if cleaned.is_empty() {
                    None
                } else {
                    Some(cleaned)
                }
            } else {
                None
            }
        });

    let is_yt_music = is_yt_music_url(&url) || is_yt_music_url(&webpage_url);

    Ok(MediaInfo {
        url: webpage_url,
        title: raw_title.to_string(),
        uploader: raw_uploader,
        album: album_val,
        album_artist: album_artist_val,
        duration: val.get("duration").and_then(|v| v.as_f64()),
        thumbnail,
        extractor: extractor_name,
        is_playlist,
        is_yt_music,
        has_flac: analyzed.has_flac,
        playlist_count,
        available_heights: analyzed.heights,
        available_video_formats: analyzed.video_formats,
        available_audio_formats: analyzed.audio_formats,
        available_audio_qualities: analyzed.audio_qualities,
        entries,
    })
}

pub async fn enqueue_download(
    manager: Arc<DownloadManager>,
    tx: mpsc::UnboundedSender<UiEvent>,
    req: DownloadRequest,
) -> Result<DownloadItem, String> {
    let settings = config::load_settings();
    let out_dir = req
        .output_dir
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(settings.download_dir);

    let _ = fs::create_dir_all(&out_dir);

    let initial_title = req
        .album
        .clone()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| req.title.clone().filter(|s| !s.trim().is_empty()))
        .unwrap_or_else(|| req.url.clone());

    let initial_uploader = req
        .album_artist
        .clone()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| req.uploader.clone().filter(|s| !s.trim().is_empty()))
        .unwrap_or_else(|| "Inspecting source...".to_string());

    let item = DownloadItem {
        id: Uuid::new_v4().to_string(),
        url: req.url.clone(),
        title: initial_title,
        uploader: initial_uploader,
        thumbnail: req.thumbnail.clone(),
        extractor: req
            .extractor
            .clone()
            .unwrap_or_else(|| "yt-dlp".to_string()),
        mode: req.mode.clone(),
        format: req.format.clone(),
        quality: req.quality.clone(),
        status: "queued".to_string(),
        progress: 0.0,
        speed: "—".to_string(),
        eta: "—".to_string(),
        total_size: "—".to_string(),
        playlist_progress: if req.download_playlist {
            Some("Preparing playlist / album...".to_string())
        } else {
            None
        },
        output_path: None,
        output_dir: out_dir,
        error: None,
        created_at: chrono::Utc::now().to_rfc3339(),
        request: req,
    };

    {
        let mut items = manager.items.lock().await;
        items.insert(0, item.clone());
    }
    manager.persist().await;
    let _ = tx.send(UiEvent::DownloadUpdated(item.clone()));

    let mgr_clone = manager.clone();
    let tx_clone = tx.clone();
    let id_clone = item.id.clone();

    tokio::spawn(async move {
        run_download_task(mgr_clone, tx_clone, id_clone).await;
    });

    Ok(item)
}

async fn run_download_task(
    manager: Arc<DownloadManager>,
    tx: mpsc::UnboundedSender<UiEvent>,
    id: String,
) {
    // Respect max_concurrent setting
    loop {
        let max_concurrent = config::load_settings().max_concurrent.max(1);
        let (active_count, is_cancelled) = {
            let items = manager.items.lock().await;
            let active = items
                .iter()
                .filter(|i| i.id != id && (i.status == "downloading" || i.status == "processing"))
                .count();
            let cancelled = items
                .iter()
                .find(|i| i.id == id)
                .map(|i| i.status == "cancelled")
                .unwrap_or(true);
            (active, cancelled)
        };
        if is_cancelled {
            return;
        }
        if active_count < max_concurrent {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(350)).await;
    }

    let item_opt = {
        let items = manager.items.lock().await;
        items.iter().find(|i| i.id == id).cloned()
    };

    let Some(mut item) = item_opt else { return };

    item.status = "downloading".to_string();
    update_item(&manager, &tx, item.clone()).await;

    let needs_impersonation_hint = is_bandcamp_url(&item.url)
        || item.extractor.to_ascii_lowercase().contains("bandcamp");
    let mut bin = match if needs_impersonation_hint {
        engines::ensure_ytdlp_with_impersonation().await
    } else {
        engines::ensure_ytdlp_binary().await
    } {
        Ok(b) => b,
        Err(e) => {
            item.status = "failed".to_string();
            item.error = Some(format!("yt-dlp auto-install error: {}", e));
            update_item(&manager, &tx, item).await;
            return;
        }
    };

    let ffmpeg_dir = match engines::ensure_ffmpeg_location().await {
        Ok(dir) => dir,
        Err(e) => {
            item.status = "failed".to_string();
            item.error = Some(format!("ffmpeg auto-install error: {}", e));
            update_item(&manager, &tx, item).await;
            return;
        }
    };
    let ffmpeg_bin = engines::resolve_ffmpeg_binary().map(|(p, _)| p);
    let ffprobe_bin = engines::resolve_ffprobe_binary();
    let _ = engines::ensure_js_runtime_if_needed().await;

    if is_item_cancelled_or_missing(&manager, &id).await {
        return;
    }

    // For playlists / YouTube Music / Bandcamp albums, resolve "<Album Artist>/<Album Title>" nested folder cleanly
    let mut target_base_dir = PathBuf::from(&item.output_dir);
    let mut resolved_album_rel_path: Option<PathBuf> = None;
    let is_bandcamp = is_bandcamp_url(&item.url)
        || item.extractor.to_ascii_lowercase().contains("bandcamp");
    let use_artist_album_folder =
        item.mode == "audio" && (item.request.download_playlist || is_bandcamp);

    if use_artist_album_folder {
        let has_artist = item
            .request
            .album_artist
            .as_deref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        let has_album = item
            .request
            .album
            .as_deref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);

        if has_artist && (has_album || !item.request.download_playlist) {
            let artist = item.request.album_artist.as_deref().unwrap_or("");
            let album = item.request.album.as_deref().unwrap_or("");
            resolved_album_rel_path = build_album_relative_path(artist, album);
        }
        if resolved_album_rel_path.is_none() || item.thumbnail.is_none() {
            if let Ok(info) = inspect_media(item.url.clone()).await {
                if resolved_album_rel_path.is_none() {
                    let artist = info.album_artist.as_deref().unwrap_or(&info.uploader);
                    let album = if info.is_playlist || item.request.download_playlist {
                        info.album.as_deref().unwrap_or(&info.title)
                    } else {
                        info.album.as_deref().unwrap_or("")
                    };
                    resolved_album_rel_path = build_album_relative_path(artist, album);
                    item.title = info.album.clone().unwrap_or(info.title);
                    item.uploader = info.album_artist.clone().unwrap_or(info.uploader);
                }
                if item.thumbnail.is_none() {
                    item.thumbnail = info.thumbnail;
                }
                item.extractor = info.extractor;
                update_item(&manager, &tx, item.clone()).await;
            }
        }
    }

    let is_bandcamp = is_bandcamp || item.extractor.to_ascii_lowercase().contains("bandcamp");

    let include_id = config::load_settings().include_id_in_filename;
    let single_title_expr = if is_bandcamp && item.mode == "audio" {
        "%(track,title).180B"
    } else {
        "%(title).180B"
    };
    let file_tpl = if include_id {
        format!("{} [%(id)s].%(ext)s", single_title_expr)
    } else {
        format!("{}.%(ext)s", single_title_expr)
    };

    let album_track_tpl = if is_bandcamp {
        "%(playlist_index|01)02d - %(track,title).160B.%(ext)s"
    } else {
        "%(playlist_index|01)02d - %(title).160B.%(ext)s"
    };

    let output_template = if item.mode == "audio" && item.request.download_playlist {
        if let Some(rel_path) = resolved_album_rel_path {
            target_base_dir = target_base_dir.join(&rel_path);
            let _ = fs::create_dir_all(&target_base_dir);
            item.output_dir = target_base_dir.to_string_lossy().to_string();
            update_item(&manager, &tx, item.clone()).await;
            album_track_tpl.to_string()
        } else {
            format!(
                "%(album_artist,artist,playlist_uploader,playlist_creator,uploader,channel|Unknown Artist).80B/%(album,playlist_title,title|Album).100B/{}",
                album_track_tpl
            )
        }
    } else if use_artist_album_folder {
        if let Some(rel_path) = resolved_album_rel_path {
            target_base_dir = target_base_dir.join(&rel_path);
            let _ = fs::create_dir_all(&target_base_dir);
            item.output_dir = target_base_dir.to_string_lossy().to_string();
            update_item(&manager, &tx, item.clone()).await;
            file_tpl
        } else {
            format!(
                "%(album_artist,artist,uploader,channel|Unknown Artist).80B/%(album|Singles).100B/{}",
                file_tpl
            )
        }
    } else if item.request.download_playlist {
        // Video playlists: never number video files
        format!("%(playlist_title,title|Playlist).100B/{}", file_tpl)
    } else {
        file_tpl
    };

    let mut args: Vec<String> = vec![
        "--encoding".to_string(),
        "utf-8".to_string(),
        "--progress".to_string(),
        "--newline".to_string(),
        "--no-colors".to_string(),
        "--windows-filenames".to_string(),
        "--no-write-thumbnail".to_string(),
        "--no-keep-video".to_string(),
        "--progress-template".to_string(),
        "download:VD_PROG|%(progress._percent_str)s|%(progress._speed_str)s|%(progress._eta_str)s|%(progress._total_bytes_str)s|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.total_bytes_estimate)s|%(info.playlist_index)s|%(info.n_entries)s|%(info.title)s".to_string(),
        "--print".to_string(),
        "after_move:VD_FILE|%(filepath)s".to_string(),
        "-P".to_string(),
        target_base_dir.to_string_lossy().to_string(),
        "-o".to_string(),
        output_template,
    ];

    args.extend(engines::ytdlp_ejs_args(&bin).await);

    if let Some(dir) = ffmpeg_dir {
        args.push("--ffmpeg-location".to_string());
        args.push(dir.to_string_lossy().to_string());
    }

    append_cookie_args(&mut args);

    args.push("--concurrent-fragments".to_string());
    args.push("8".to_string());
    args.push("--retries".to_string());
    args.push("10".to_string());
    args.push("--fragment-retries".to_string());
    args.push("10".to_string());

    if item.request.download_playlist {
        args.push("--yes-playlist".to_string());
        args.push("--ignore-errors".to_string());
        if item.mode == "audio" {
            args.push("--parse-metadata".to_string());
            args.push("playlist_index:%(track_number)s".to_string());
        }
    } else {
        args.push("--no-playlist".to_string());
    }

    if item.mode == "audio" {
        if item.format == "best" {
            // Prioritize FLAC if natively available, otherwise Opus/best audio extracted cleanly without .webm
            args.push("-f".to_string());
            args.push(
                "bestaudio[acodec=flac]/bestaudio[ext=flac]/bestaudio[acodec=opus]/bestaudio/best"
                    .to_string(),
            );
            args.push("-x".to_string());
            args.push("--audio-format".to_string());
            args.push("best".to_string());
            args.push("--audio-quality".to_string());
            args.push("0".to_string());
        } else {
            if item.format == "flac" || item.format == "wav" {
                args.push("-f".to_string());
                args.push("bestaudio[acodec=flac]/bestaudio[ext=flac]/bestaudio/best".to_string());
            } else {
                args.push("-f".to_string());
                args.push("bestaudio/best".to_string());
            }
            args.push("-x".to_string());
            args.push("--audio-format".to_string());
            args.push(item.format.clone());
            if item.format != "flac" && item.format != "wav" {
                let q = match item.quality.as_str() {
                    "best" | "320" => "320K".to_string(),
                    other if other.chars().all(|c| c.is_ascii_digit()) && !other.is_empty() => {
                        format!("{}K", other)
                    }
                    _ => "320K".to_string(),
                };
                args.push("--audio-quality".to_string());
                args.push(q);
            }
        }
    } else {
        let fmt_selector = if item.quality == "best" {
            "bestvideo+bestaudio/best".to_string()
        } else if let Ok(h) = item.quality.trim().parse::<u32>() {
            format!("bestvideo[height<={0}]+bestaudio/best[height<={0}]", h)
        } else {
            "bestvideo+bestaudio/best".to_string()
        };
        args.push("-f".to_string());
        args.push(fmt_selector);
        if item.format == "mp4" {
            args.push("-S".to_string());
            args.push("vext:mp4,aext:m4a".to_string());
        } else if item.format == "webm" {
            args.push("-S".to_string());
            args.push("vext:webm,aext:opus".to_string());
        }
        args.push("--merge-output-format".to_string());
        args.push(item.format.clone());
        args.push("--remux-video".to_string());
        args.push(item.format.clone());
    }

    // Never pass yt-dlp --embed-thumbnail (it triggers Python/mutagen errors and leaves per-track covers).
    // Cover artwork is saved once per album (cover.jpg) and embedded natively via pure C ffmpeg where supported.
    if item.request.embed_metadata {
        args.push("--embed-metadata".to_string());
    }
    if item.request.subtitles && item.mode == "video" {
        args.push("--write-subs".to_string());
        args.push("--sub-langs".to_string());
        args.push("hu,en".to_string());
    }

    args.push(item.url.clone());

    let mut collected_files: Vec<PathBuf> = Vec::new();
    let mut succeeded = false;
    let mut stderr_text = String::new();

    for attempt in 0..2 {
        if is_item_cancelled_or_missing(&manager, &id).await {
            return;
        }

        let mut cmd = Command::new(&bin);
        engines::configure_ytdlp_command(&mut cmd);
        cmd.args(&args)
            .env("PYTHONUNBUFFERED", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        #[cfg(target_os = "windows")]
        cmd.creation_flags(0x08000000);

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                item.status = "failed".to_string();
                item.error = Some(format!("Process launch error: {}", e));
                update_item(&manager, &tx, item).await;
                return;
            }
        };

        if let Some(pid) = child.id() {
            let mut pids = manager.active_pids.lock().await;
            pids.insert(id.clone(), pid);
        }

        let stderr_handle = child.stderr.take().map(|mut stderr| {
            tokio::spawn(async move {
                use tokio::io::AsyncReadExt;
                let mut raw = Vec::new();
                let mut chunk = [0u8; 4096];
                while let Ok(n) = stderr.read(&mut chunk).await {
                    if n == 0 {
                        break;
                    }
                    let take = n.min(16384usize.saturating_sub(raw.len()));
                    raw.extend_from_slice(&chunk[..take]);
                    if raw.len() >= 16384 {
                        break;
                    }
                }
                decode_ytdlp_bytes(&raw)
            })
        });

        if let Some(mut stdout) = child.stdout.take() {
            use tokio::io::AsyncReadExt;
            let mut buf = [0u8; 4096];
            let mut pending = Vec::<u8>::new();

            loop {
                let n = match stdout.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(_) => break,
                };
                pending.extend_from_slice(&buf[..n]);

                while let Some(pos) = pending.iter().position(|&b| b == b'\n' || b == b'\r') {
                    let line_bytes: Vec<u8> = pending.drain(..=pos).collect();
                    let decoded = decode_ytdlp_bytes(&line_bytes);
                    let line = decoded.trim();
                    if line.is_empty() {
                        continue;
                    }
                    handle_ytdlp_stdout_line(
                        line,
                        &mut item,
                        &mut collected_files,
                        &manager,
                        &tx,
                    )
                    .await;
                }
            }

            if !pending.is_empty() {
                let decoded = decode_ytdlp_bytes(&pending);
                let line = decoded.trim();
                if !line.is_empty() {
                    handle_ytdlp_stdout_line(
                        line,
                        &mut item,
                        &mut collected_files,
                        &manager,
                        &tx,
                    )
                    .await;
                }
            }
        }

        let status = child.wait().await;
        stderr_text = match stderr_handle {
            Some(handle) => handle.await.unwrap_or_default(),
            None => String::new(),
        };

        {
            let mut pids = manager.active_pids.lock().await;
            pids.remove(&id);
        }

        if is_item_cancelled_or_missing(&manager, &id).await {
            return;
        }

        succeeded = status.as_ref().map(|e| e.success()).unwrap_or(false);
        if !succeeded
            && attempt == 0
            && (stderr_text.contains("no impersonate target is available")
                || stderr_text.contains("attempting impersonation"))
        {
            if let Ok(upgraded) = engines::ensure_ytdlp_with_impersonation().await {
                if upgraded != bin {
                    bin = upgraded;
                    continue;
                }
            }
        }
        break;
    }

    if is_item_cancelled_or_missing(&manager, &id).await {
        return;
    }

    if succeeded || !collected_files.is_empty() {
        item.status = "processing".to_string();
        update_item(&manager, &tx, item.clone()).await;
        finalize_downloaded_media(
            &mut item,
            &mut collected_files,
            ffmpeg_bin.as_deref(),
            ffprobe_bin.as_deref(),
        )
        .await;
    }

    if succeeded {
        item.status = "completed".to_string();
        item.progress = 100.0;
        item.eta = "00:00".to_string();
        item.speed = "—".to_string();
        item.error = None;
    } else {
        item.status = "failed".to_string();
        if item.error.is_none() {
            if !stderr_text.trim().is_empty() {
                item.error = Some(format_ytdlp_error(&stderr_text));
            } else {
                item.error = Some(
                    "Download failed (verify the link or ffmpeg installation).".to_string(),
                );
            }
        }
    }
    update_item(&manager, &tx, item).await;
}

async fn convert_image_file_to_jpg(
    src_img: &Path,
    dest_jpg: &Path,
    ffmpeg_bin: Option<&Path>,
) -> Result<(), String> {
    let src_owned = src_img.to_path_buf();
    let dest_owned = dest_jpg.to_path_buf();
    let rust_decode = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let bytes = fs::read(&src_owned).map_err(|e| e.to_string())?;
        let img =
            image::load_from_memory(&bytes).map_err(|e| format!("Cover decode error: {}", e))?;
        let rgb = img.to_rgb8();
        let tmp = dest_owned.with_file_name(format!(".vd_cover_{}.jpg", Uuid::new_v4()));
        if let Err(e) = rgb.save_with_format(&tmp, image::ImageFormat::Jpeg) {
            let _ = fs::remove_file(&tmp);
            return Err(format!("Cover save error: {}", e));
        }
        if let Err(e) = fs::rename(&tmp, &dest_owned) {
            let _ = fs::remove_file(&tmp);
            return Err(format!("Cover rename error: {}", e));
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?;

    if rust_decode.is_ok() {
        return Ok(());
    }

    if let Some(ffmpeg) = ffmpeg_bin {
        let tmp = dest_jpg.with_file_name(format!(".vd_cover_ff_{}.jpg", Uuid::new_v4()));
        let mut cmd = Command::new(ffmpeg);
        cmd.arg("-y")
            .arg("-i")
            .arg(src_img)
            .args(["-frames:v", "1", "-q:v", "2"])
            .arg(&tmp)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(target_os = "windows")]
        cmd.creation_flags(0x08000000);
        let ok = cmd.status().await.map(|s| s.success()).unwrap_or(false);
        if ok && tmp.is_file() && fs::rename(&tmp, dest_jpg).is_ok() {
            return Ok(());
        }
        let _ = fs::remove_file(&tmp);
    }

    rust_decode
}

async fn download_cover_jpg(
    thumb_url: &str,
    dest_jpg: &Path,
    ffmpeg_bin: Option<&Path>,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36")
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .get(thumb_url)
        .send()
        .await
        .map_err(|e| format!("Cover download failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("Cover HTTP {}", resp.status()));
    }

    let bytes = resp.bytes().await.map_err(|e| e.to_string())?.to_vec();
    let raw_tmp = std::env::temp_dir().join(format!("vd_raw_cover_{}.img", Uuid::new_v4()));
    fs::write(&raw_tmp, &bytes).map_err(|e| e.to_string())?;
    let res = convert_image_file_to_jpg(&raw_tmp, dest_jpg, ffmpeg_bin).await;
    let _ = fs::remove_file(&raw_tmp);
    res
}

async fn fetch_album_cover_via_ytdlp(
    url: &str,
    dest_jpg: &Path,
    ffmpeg_bin: Option<&Path>,
) -> Result<(), String> {
    let ytdlp_bin = if is_bandcamp_url(url) {
        engines::ensure_ytdlp_with_impersonation().await?
    } else {
        engines::ensure_ytdlp_binary().await?
    };
    let tmp_dir = std::env::temp_dir().join(format!("vd_album_cover_{}", Uuid::new_v4()));
    fs::create_dir_all(&tmp_dir).map_err(|e| e.to_string())?;

    let mut args = vec![
        "--encoding".to_string(),
        "utf-8".to_string(),
        "--skip-download".to_string(),
        "--write-thumbnail".to_string(),
        "--ignore-errors".to_string(),
        "--no-warnings".to_string(),
        "--no-colors".to_string(),
        "--playlist-items".to_string(),
        "1".to_string(),
        "-P".to_string(),
        tmp_dir.to_string_lossy().to_string(),
        "-o".to_string(),
        "cover.%(ext)s".to_string(),
    ];
    let _ = engines::ensure_js_runtime_if_needed().await;
    args.extend(engines::ytdlp_ejs_args(&ytdlp_bin).await);
    if let Some(dir) = ffmpeg_bin.and_then(|p| p.parent()) {
        args.push("--ffmpeg-location".to_string());
        args.push(dir.to_string_lossy().to_string());
    }
    append_cookie_args(&mut args);
    args.push(url.to_string());

    let mut cmd = Command::new(&ytdlp_bin);
    engines::configure_ytdlp_command(&mut cmd);
    cmd.args(&args).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000);

    let _ = tokio::time::timeout(std::time::Duration::from_secs(40), cmd.output()).await;

    let mut found_img: Option<(u64, PathBuf)> = None;
    if let Ok(entries) = fs::read_dir(&tmp_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if !p.is_file() {
                continue;
            }
            let ext = p
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if matches!(ext.as_str(), "part" | "ytdl" | "temp") {
                continue;
            }
            let len = p.metadata().map(|m| m.len()).unwrap_or(0);
            if len > 0 && found_img.as_ref().map(|(best_len, _)| len > *best_len).unwrap_or(true) {
                found_img = Some((len, p));
            }
        }
    }

    let result = if let Some((_, img_path)) = found_img {
        convert_image_file_to_jpg(&img_path, dest_jpg, ffmpeg_bin).await
    } else {
        Err("No cover image downloaded by yt-dlp".to_string())
    };

    let _ = fs::remove_dir_all(&tmp_dir);
    result
}

async fn probe_audio_codec(
    media_path: &Path,
    ffmpeg_bin: &Path,
    ffprobe_bin: Option<&Path>,
) -> Option<String> {
    if let Some(ffprobe) = ffprobe_bin {
        let mut cmd = Command::new(ffprobe);
        cmd.args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=codec_name",
            "-of",
            "default=nw=1:nk=1",
        ])
        .arg(media_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
        #[cfg(target_os = "windows")]
        cmd.creation_flags(0x08000000);
        if let Ok(out) = cmd.output().await {
            if out.status.success() {
                let codec = String::from_utf8_lossy(&out.stdout)
                    .trim()
                    .to_ascii_lowercase();
                if !codec.is_empty() {
                    return Some(codec);
                }
            }
        }
    }

    let mut cmd = Command::new(ffmpeg_bin);
    cmd.arg("-i")
        .arg(media_path)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000);
    if let Ok(out) = cmd.output().await {
        let stderr = String::from_utf8_lossy(&out.stderr).to_ascii_lowercase();
        for codec in ["opus", "flac", "aac", "vorbis", "mp3", "alac"] {
            if stderr.contains(&format!("audio: {}", codec)) {
                return Some(codec.to_string());
            }
        }
    }
    None
}

async fn convert_leftover_webm_audio(
    webm_path: &Path,
    ffmpeg_bin: &Path,
    ffprobe_bin: Option<&Path>,
) -> Option<PathBuf> {
    let codec = probe_audio_codec(webm_path, ffmpeg_bin, ffprobe_bin).await;
    let (target_ext, codec_args): (&str, &[&str]) = match codec.as_deref() {
        Some("opus") => ("opus", &["-vn", "-c:a", "copy"]),
        Some("vorbis") => ("ogg", &["-vn", "-c:a", "copy"]),
        Some("aac") => ("m4a", &["-vn", "-c:a", "copy"]),
        Some("flac") => ("flac", &["-vn", "-c:a", "copy"]),
        Some("mp3") => ("mp3", &["-vn", "-c:a", "copy"]),
        _ => ("opus", &["-vn", "-c:a", "libopus", "-b:a", "192k"]),
    };

    let out_path = webm_path.with_extension(target_ext);
    let tmp_out = webm_path.with_file_name(format!(".vd_remux_{}.{}", Uuid::new_v4(), target_ext));

    let mut cmd = Command::new(ffmpeg_bin);
    cmd.arg("-y")
        .arg("-i")
        .arg(webm_path)
        .args(codec_args)
        .arg(&tmp_out)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000);

    let ok = cmd
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false);

    if ok && tmp_out.exists() {
        if fs::rename(&tmp_out, &out_path).is_ok() {
            let _ = fs::remove_file(webm_path);
            return Some(out_path);
        }
    }
    let _ = fs::remove_file(&tmp_out);
    None
}

async fn embed_cover_with_ffmpeg(media_path: &Path, cover_jpg: &Path, ffmpeg_bin: &Path) {
    let Some(ext) = media_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
    else {
        return;
    };

    // Pure ffmpeg stream-copy attached artwork works reliably for mp3, m4a, flac, mp4 without Python/mutagen
    let is_supported = matches!(ext.as_str(), "mp3" | "m4a" | "flac" | "mp4");
    if !is_supported || !media_path.exists() || !cover_jpg.exists() {
        return;
    }

    let tmp_out = media_path.with_file_name(format!(".vd_art_{}.{}", Uuid::new_v4(), ext));
    let mut cmd = Command::new(ffmpeg_bin);
    cmd.arg("-y")
        .arg("-i")
        .arg(media_path)
        .arg("-i")
        .arg(cover_jpg);

    if ext == "mp4" {
        cmd.args(["-map", "0", "-map", "1", "-c", "copy", "-disposition:v:1", "attached_pic"]);
    } else {
        cmd.args(["-map", "0:a", "-map", "1:v", "-c", "copy", "-disposition:v:0", "attached_pic"]);
        if ext == "mp3" {
            cmd.args(["-id3v2_version", "3"]);
        }
    }

    cmd.arg(&tmp_out)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000);

    let ok = cmd.status().await.map(|s| s.success()).unwrap_or(false);
    if ok && tmp_out.exists() {
        if fs::rename(&tmp_out, media_path).is_err() {
            let _ = fs::remove_file(&tmp_out);
        }
    } else {
        let _ = fs::remove_file(&tmp_out);
    }
}

async fn finalize_downloaded_media(
    item: &mut DownloadItem,
    collected_files: &mut Vec<PathBuf>,
    ffmpeg_bin: Option<&Path>,
    ffprobe_bin: Option<&Path>,
) {
    let output_dir_path = PathBuf::from(&item.output_dir);

    // 1. In audio mode, ensure NO .webm / video container files remain anywhere in the audio output
    if item.mode == "audio" {
        if let Some(ffmpeg) = ffmpeg_bin {
            if item.request.download_playlist && output_dir_path.is_dir() {
                if let Ok(entries) = fs::read_dir(&output_dir_path) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if p.is_file() {
                            if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                                if ext.eq_ignore_ascii_case("webm")
                                    || ext.eq_ignore_ascii_case("mkv")
                                    || ext.eq_ignore_ascii_case("mp4")
                                {
                                    if !collected_files.contains(&p) {
                                        collected_files.push(p);
                                    }
                                }
                            }
                        }
                    }
                }
            }

            let snapshot = collected_files.clone();
            for p in snapshot {
                if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                    if ext.eq_ignore_ascii_case("webm")
                        || (item.request.download_playlist
                            && (ext.eq_ignore_ascii_case("mkv") || ext.eq_ignore_ascii_case("mp4")))
                    {
                        if let Some(converted) =
                            convert_leftover_webm_audio(&p, ffmpeg, ffprobe_bin).await
                        {
                            if item.output_path.as_deref() == Some(p.to_string_lossy().as_ref()) {
                                item.output_path = Some(converted.to_string_lossy().to_string());
                            }
                            if let Some(pos) = collected_files.iter().position(|x| x == &p) {
                                collected_files[pos] = converted;
                            }
                        }
                    } else if !item.request.download_playlist {
                        let sibling_webm = p.with_extension("webm");
                        if sibling_webm.is_file() {
                            let _ = fs::remove_file(sibling_webm);
                        }
                    }
                }
            }
        }

        // Remove any stray per-track thumbnail/temp files in album folders (keep only cover.jpg + music)
        if item.request.download_playlist && output_dir_path.is_dir() {
            if let Ok(entries) = fs::read_dir(&output_dir_path) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if !p.is_file() {
                        continue;
                    }
                    let fname = p
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    if fname == "cover.jpg" {
                        continue;
                    }
                    let ext = p
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    if fname.contains(".part")
                        || matches!(
                            ext.as_str(),
                            "webp" | "jpg" | "jpeg" | "png" | "part" | "ytdl" | "temp"
                        )
                    {
                        let _ = fs::remove_file(&p);
                    }
                }
            }
        }
    } else if matches!(item.format.as_str(), "mp4" | "mkv" | "webm") {
        // Ensure video container always matches the user's selected format (e.g. mkv even when 1 stream was downloaded as webm)
        if let Some(ffmpeg) = ffmpeg_bin {
            if collected_files.is_empty() {
                if let Some(out_str) = item.output_path.as_deref() {
                    let pb = PathBuf::from(out_str);
                    if pb.is_file() {
                        collected_files.push(pb);
                    }
                }
            }
            let snapshot = collected_files.clone();
            for p in snapshot {
                if !p.is_file() {
                    continue;
                }
                let ext = p
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if ext != item.format {
                    if let Some(remuxed) = remux_video_container(&p, &item.format, ffmpeg).await {
                        if item.output_path.as_deref() == Some(p.to_string_lossy().as_ref()) {
                            item.output_path = Some(remuxed.to_string_lossy().to_string());
                        }
                        if let Some(pos) = collected_files.iter().position(|x| x == &p) {
                            collected_files[pos] = remuxed;
                        }
                    }
                }
            }
        }
    }

    // Clean up any leftover yt-dlp fragment/partial files (e.g., *.m4a.part-Frag1688, *.part, *.ytdl, *.temp)
    if output_dir_path.is_dir() {
        let mut known_stems: Vec<String> = collected_files
            .iter()
            .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(|s| s.to_string()))
            .collect();
        if let Some(out_p) = item.output_path.as_deref() {
            if let Some(stem) = Path::new(out_p).file_stem().and_then(|s| s.to_str()) {
                if !known_stems.iter().any(|k| k == stem) {
                    known_stems.push(stem.to_string());
                }
            }
        }
        if let Ok(entries) = fs::read_dir(&output_dir_path) {
            for entry in entries.flatten() {
                let p = entry.path();
                if !p.is_file() {
                    continue;
                }
                let fname = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                let fname_lower = fname.to_ascii_lowercase();
                let is_temp = fname_lower.contains(".part")
                    || fname_lower.ends_with(".ytdl")
                    || fname_lower.ends_with(".temp");
                if !is_temp {
                    continue;
                }
                let matches_this_item = item.request.download_playlist
                    || known_stems.iter().any(|stem| fname.starts_with(stem.as_str()));
                if matches_this_item {
                    let _ = fs::remove_file(&p);
                }
            }
        }
    }

    // 2. Cover artwork is ONLY for music / album downloads (never download or embed cover art for videos)
    if item.mode == "audio" {
        if item.request.download_playlist {
            let _ = fs::create_dir_all(&output_dir_path);
            let cover_path = output_dir_path.join("cover.jpg");
            if item.request.embed_thumbnail && !cover_path.exists() {
                let mut got_cover = false;
                if let Some(thumb_url) = item.thumbnail.as_deref() {
                    if download_cover_jpg(thumb_url, &cover_path, ffmpeg_bin)
                        .await
                        .is_ok()
                    {
                        got_cover = cover_path.is_file();
                    }
                }
                if !got_cover {
                    let _ = fetch_album_cover_via_ytdlp(&item.url, &cover_path, ffmpeg_bin).await;
                }
            }
            if item.request.embed_thumbnail && cover_path.exists() {
                if collected_files.is_empty() && output_dir_path.is_dir() {
                    if let Ok(entries) = fs::read_dir(&output_dir_path) {
                        for entry in entries.flatten() {
                            let p = entry.path();
                            if p.is_file() {
                                collected_files.push(p);
                            }
                        }
                    }
                }
                if let Some(ffmpeg) = ffmpeg_bin {
                    for p in collected_files.iter() {
                        embed_cover_with_ffmpeg(p, &cover_path, ffmpeg).await;
                    }
                }
            }
        } else if item.request.embed_thumbnail {
            if let (Some(ffmpeg), Some(thumb_url), Some(out_str)) = (
                ffmpeg_bin,
                item.thumbnail.as_deref(),
                item.output_path.as_deref(),
            ) {
                let out_p = PathBuf::from(out_str);
                let tmp_cover =
                    std::env::temp_dir().join(format!("vd_single_cover_{}.jpg", item.id));
                if download_cover_jpg(thumb_url, &tmp_cover, Some(ffmpeg))
                    .await
                    .is_ok()
                {
                    embed_cover_with_ffmpeg(&out_p, &tmp_cover, ffmpeg).await;
                }
                let _ = fs::remove_file(&tmp_cover);
            }
        }
    }
}

async fn remux_video_container(
    input_path: &Path,
    target_ext: &str,
    ffmpeg_bin: &Path,
) -> Option<PathBuf> {
    let out_path = input_path.with_extension(target_ext);
    let tmp_out =
        input_path.with_file_name(format!(".vd_vremux_{}.{}", Uuid::new_v4(), target_ext));

    let mut cmd = Command::new(ffmpeg_bin);
    cmd.arg("-y")
        .arg("-i")
        .arg(input_path)
        .args(["-map", "0", "-c", "copy"])
        .arg(&tmp_out)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000);

    let mut ok = cmd
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false);

    if (!ok || !tmp_out.exists()) && target_ext == "mp4" {
        let _ = fs::remove_file(&tmp_out);
        let mut fallback = Command::new(ffmpeg_bin);
        fallback
            .arg("-y")
            .arg("-i")
            .arg(input_path)
            .args(["-map", "0", "-c:v", "copy", "-c:a", "aac", "-b:a", "192k"])
            .arg(&tmp_out)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(target_os = "windows")]
        fallback.creation_flags(0x08000000);
        ok = fallback
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false);
    }

    if ok && tmp_out.exists() {
        if fs::rename(&tmp_out, &out_path).is_ok() {
            let _ = fs::remove_file(input_path);
            return Some(out_path);
        }
    }
    let _ = fs::remove_file(&tmp_out);
    None
}

async fn handle_ytdlp_stdout_line(
    line: &str,
    item: &mut DownloadItem,
    collected_files: &mut Vec<PathBuf>,
    manager: &Arc<DownloadManager>,
    tx: &mpsc::UnboundedSender<UiEvent>,
) {
    if let Some(rest) = line.strip_prefix("VD_PROG|") {
        let parts: Vec<&str> = rest.split('|').collect();
        if parts.len() >= 7 {
            let pct_str = parts[0].trim().trim_end_matches('%').trim();
            let mut parsed_pct = pct_str.parse::<f64>().ok();

            if parsed_pct.is_none() {
                let dl = parts[4].trim().parse::<f64>().ok();
                let tot = parts[5]
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .or_else(|| parts[6].trim().parse::<f64>().ok());
                if let (Some(d), Some(t)) = (dl, tot) {
                    if t > 0.0 {
                        parsed_pct = Some((d / t) * 100.0);
                    }
                }
            }

            let speed = parts[1].trim();
            if !speed.is_empty() && speed != "NA" && speed != "Unknown B/s" {
                item.speed = speed.to_string();
            }
            let eta = parts[2].trim();
            if !eta.is_empty() && eta != "NA" && eta != "Unknown" {
                item.eta = eta.to_string();
            }
            let total_sz = parts[3].trim();
            if !total_sz.is_empty() && total_sz != "NA" && total_sz != "Unknown B" {
                item.total_size = total_sz.to_string();
            }

            let track_pct = parsed_pct.map(|p| p.clamp(0.0, 100.0));

            if item.request.download_playlist && parts.len() >= 10 {
                let pl_idx = parts[7].trim();
                let pl_total = parts[8].trim();
                let track_title = parts[9..].join("|");
                let track_title = track_title.trim();
                if pl_idx != "NA" && pl_total != "NA" {
                    item.playlist_progress = Some(format!(
                        "{}. / {} · {}",
                        pl_idx,
                        pl_total,
                        if track_title.is_empty() || track_title == "NA" {
                            "Processing track"
                        } else {
                            track_title
                        }
                    ));
                    if let (Ok(idx), Ok(total)) = (pl_idx.parse::<f64>(), pl_total.parse::<f64>()) {
                        if total >= 1.0 && idx >= 1.0 {
                            let tp = track_pct.unwrap_or(0.0);
                            item.progress =
                                ((((idx - 1.0) + (tp / 100.0)) / total) * 100.0).clamp(0.0, 99.9);
                        } else if let Some(tp) = track_pct {
                            item.progress = tp;
                        }
                    } else if let Some(tp) = track_pct {
                        item.progress = tp;
                    }
                } else if let Some(tp) = track_pct {
                    item.progress = tp;
                }
            } else {
                if parts.len() >= 10
                    && (item.title.starts_with("http://") || item.title.starts_with("https://"))
                {
                    let track_title = parts[9..].join("|");
                    let track_title = track_title.trim();
                    if !track_title.is_empty() && track_title != "NA" {
                        item.title = track_title.to_string();
                    }
                }
                if let Some(tp) = track_pct {
                    item.progress = tp;
                }
            }

            update_item_live(manager, tx, item.clone()).await;
        }
    } else if let Some(filepath) = line.strip_prefix("VD_FILE|") {
        let fp = filepath.trim();
        if !fp.is_empty() && !fp.contains('\u{FFFD}') {
            let pb = PathBuf::from(fp);
            if pb.exists() {
                if !collected_files.contains(&pb) {
                    collected_files.push(pb.clone());
                }
                item.output_path = Some(fp.to_string());
                if item.request.download_playlist {
                    if let Some(parent) = pb.parent().filter(|p| p.is_dir()) {
                        item.output_dir = parent.to_string_lossy().to_string();
                    }
                }
            }
        }
    } else if let Some(dl_rest) = line.strip_prefix("[download]") {
        // Fallback standard yt-dlp progress line parsing if --progress-template wasn't matched
        if let Some(pct_token) = dl_rest
            .split_whitespace()
            .find(|tok| tok.ends_with('%'))
            .and_then(|tok| tok.trim_end_matches('%').parse::<f64>().ok())
        {
            item.progress = pct_token.clamp(0.0, 100.0);
            update_item_live(manager, tx, item.clone()).await;
        }
    }
}

async fn is_item_cancelled_or_missing(manager: &DownloadManager, id: &str) -> bool {
    let items = manager.items.lock().await;
    items
        .iter()
        .find(|i| i.id == id)
        .map(|i| i.status == "cancelled")
        .unwrap_or(true)
}

async fn update_item_live(
    manager: &Arc<DownloadManager>,
    tx: &mpsc::UnboundedSender<UiEvent>,
    updated: DownloadItem,
) {
    let mut sent = false;
    {
        let mut items = manager.items.lock().await;
        if let Some(pos) = items.iter().position(|i| i.id == updated.id) {
            if items[pos].status != "cancelled" || updated.status == "cancelled" {
                items[pos] = updated.clone();
                sent = true;
            }
        }
    }
    if sent {
        let _ = tx.send(UiEvent::DownloadUpdated(updated));
    }
}

async fn update_item(
    manager: &Arc<DownloadManager>,
    tx: &mpsc::UnboundedSender<UiEvent>,
    updated: DownloadItem,
) {
    let mut sent = false;
    {
        let mut items = manager.items.lock().await;
        if let Some(pos) = items.iter().position(|i| i.id == updated.id) {
            if items[pos].status != "cancelled" || updated.status == "cancelled" {
                items[pos] = updated.clone();
                sent = true;
            }
        }
    }
    if sent {
        manager.persist().await;
        let _ = tx.send(UiEvent::DownloadUpdated(updated));
    }
}

fn kill_pid_blocking(pid: u32) {
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = std::process::Command::new("taskkill");
        cmd.args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x08000000);
        let _ = cmd.status();
    }
}

pub async fn cancel_download(
    manager: Arc<DownloadManager>,
    tx: mpsc::UnboundedSender<UiEvent>,
    id: String,
) {
    if let Some(pid) = manager.active_pids.lock().await.remove(&id) {
        let _ = tokio::task::spawn_blocking(move || kill_pid_blocking(pid)).await;
    }

    let mut updated_item = None;
    {
        let mut items = manager.items.lock().await;
        if let Some(item) = items.iter_mut().find(|i| i.id == id) {
            item.status = "cancelled".to_string();
            item.error = Some("Cancelled by user".to_string());
            updated_item = Some(item.clone());
        }
    }
    manager.persist().await;
    if let Some(item) = updated_item {
        let _ = tx.send(UiEvent::DownloadUpdated(item));
    }
}

pub async fn remove_download(manager: Arc<DownloadManager>, id: String) {
    if let Some(pid) = manager.active_pids.lock().await.remove(&id) {
        let _ = tokio::task::spawn_blocking(move || kill_pid_blocking(pid)).await;
    }
    {
        let mut items = manager.items.lock().await;
        items.retain(|i| i.id != id);
    }
    manager.persist().await;
}

pub async fn cancel_all_active(manager: &DownloadManager) {
    let pids: Vec<u32> = {
        let mut guard = manager.active_pids.lock().await;
        guard.drain().map(|(_, pid)| pid).collect()
    };
    if !pids.is_empty() {
        let _ = tokio::task::spawn_blocking(move || {
            for pid in pids {
                kill_pid_blocking(pid);
            }
        })
        .await;
    }
    let mut changed = false;
    {
        let mut items = manager.items.lock().await;
        for item in items.iter_mut() {
            if item.status == "downloading" || item.status == "queued" || item.status == "processing" {
                item.status = "cancelled".to_string();
                item.error = Some("Application closed while downloading".to_string());
                changed = true;
            }
        }
    }
    if changed {
        manager.persist().await;
    }
}

pub fn open_folder(path_str: &str) {
    let target = Path::new(path_str);
    let folder = if target.is_file() {
        target.parent().unwrap_or(target)
    } else {
        target
    };
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(folder).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer").arg(folder).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(folder).spawn();
    }
}
