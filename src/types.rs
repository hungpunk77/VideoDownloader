use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppLanguage {
    En,
    De,
    Hu,
}

impl Default for AppLanguage {
    fn default() -> Self {
        Self::En
    }
}

impl AppLanguage {
    pub fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::De => "de",
            Self::Hu => "hu",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::En => "English",
            Self::De => "Deutsch",
            Self::Hu => "Magyar",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    Light,
    Dark,
}

impl Default for ThemeMode {
    fn default() -> Self {
        Self::Dark
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub download_dir: String,
    pub default_mode: String,          // "video" | "audio"
    pub default_video_format: String,  // "mp4" | "mkv" | "webm"
    pub default_video_quality: String, // "best" | "2160" | "1440" | "1080" | "720" | "480"
    pub default_audio_format: String,  // "best" | "flac" | "mp3" | "m4a" | "opus" | "wav"
    pub default_audio_quality: String, // "best" | "320" | "256" | "192" | "128"
    pub embed_thumbnail: bool,
    pub embed_metadata: bool,
    pub download_subtitles: bool,
    pub auto_check_ytdlp: bool,
    pub auto_paste_clipboard: bool,
    pub max_concurrent: usize,
    #[serde(default = "default_cookie_browser")]
    pub cookie_browser: String,
    #[serde(default)]
    pub has_cookies_file: bool,
    #[serde(default)]
    pub language: AppLanguage,
    #[serde(default)]
    pub theme: ThemeMode,
    #[serde(default = "default_true")]
    pub include_id_in_filename: bool,
    #[serde(default = "default_font_size")]
    pub font_size: f32,
    #[serde(default = "default_small_font_size")]
    pub small_font_size: f32,
}

fn default_cookie_browser() -> String {
    "none".to_string()
}

fn default_true() -> bool {
    true
}

fn default_font_size() -> f32 {
    14.0
}

fn default_small_font_size() -> f32 {
    12.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    pub installed: bool,
    pub current_version: Option<String>,
    pub latest_version: Option<String>,
    pub update_available: bool,
    pub binary_path: Option<String>,
    pub ffmpeg_installed: bool,
    pub ffmpeg_version: Option<String>,
    pub ffmpeg_latest_version: Option<String>,
    pub ffmpeg_update_available: bool,
    pub ffmpeg_path: Option<String>,
    pub ffmpeg_source: String, // "local" | "system" | "missing"
    #[serde(default)]
    pub js_runtime_installed: bool,
    #[serde(default)]
    pub js_runtime_name: Option<String>,
    #[serde(default)]
    pub js_runtime_path: Option<String>,
    #[serde(default)]
    pub js_runtime_version: Option<String>,
    #[serde(default)]
    pub js_runtime_latest_version: Option<String>,
    #[serde(default)]
    pub js_runtime_update_available: bool,
    #[serde(default)]
    pub js_runtime_source: String, // "local" | "system" | "missing"
    pub release_url: String,
    pub checked_at: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistEntry {
    pub index: usize,
    pub title: String,
    pub duration: Option<f64>,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaInfo {
    pub url: String,
    pub title: String,
    pub uploader: String,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub duration: Option<f64>,
    pub thumbnail: Option<String>,
    pub extractor: String,
    pub is_playlist: bool,
    pub is_yt_music: bool,
    pub has_flac: bool,
    pub playlist_count: Option<usize>,
    pub available_heights: Vec<u32>,
    #[serde(default)]
    pub available_video_formats: Vec<String>,
    #[serde(default)]
    pub available_audio_formats: Vec<String>,
    #[serde(default)]
    pub available_audio_qualities: Vec<String>,
    pub entries: Vec<PlaylistEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MusicSearchSource {
    Youtube,
    YoutubeMusic,
    Bandcamp,
    Dailymotion,
    Videa,
    OkRu,
}

impl MusicSearchSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Youtube => "YouTube",
            Self::YoutubeMusic => "YouTube Music",
            Self::Bandcamp => "Bandcamp",
            Self::Dailymotion => "Dailymotion",
            Self::Videa => "Videa.hu",
            Self::OkRu => "ok.ru",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MusicSearchItemKind {
    Video,
    Song,
    Album,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MusicSearchResult {
    pub id: String,
    pub source: MusicSearchSource,
    pub kind: MusicSearchItemKind,
    pub release_type: Option<String>,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub year: Option<String>,
    pub duration_text: Option<String>,
    pub url: String,
    pub thumbnail: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SearchPagination {
    pub query: String,
    pub include_ytm: bool,

    pub yt_continuation: Option<String>,
    pub yt_has_more: bool,

    pub ytm_songs_continuation: Option<String>,
    pub ytm_albums_continuation: Option<String>,
    pub ytm_has_more: bool,

    pub dm_next_page: u32,
    pub dm_total: Option<usize>,
    pub dm_has_more: bool,

    pub okru_offset: usize,
    pub okru_total: Option<usize>,
    pub okru_has_more: bool,

    pub videa_cache_id: Option<String>,
    pub videa_sort: String,
    pub videa_last_item_id: Option<String>,
    pub videa_item_count: usize,
    pub videa_total: Option<usize>,
    pub videa_has_more: bool,
}

impl SearchPagination {
    pub fn has_more_for_source(&self, source: Option<MusicSearchSource>) -> bool {
        match source {
            Some(MusicSearchSource::Youtube) => self.yt_has_more,
            Some(MusicSearchSource::YoutubeMusic) => self.ytm_has_more,
            Some(MusicSearchSource::Bandcamp) => false,
            Some(MusicSearchSource::Dailymotion) => self.dm_has_more,
            Some(MusicSearchSource::Videa) => self.videa_has_more,
            Some(MusicSearchSource::OkRu) => self.okru_has_more,
            None => {
                self.yt_has_more
                    || self.ytm_has_more
                    || self.dm_has_more
                    || self.videa_has_more
                    || self.okru_has_more
            }
        }
    }

    pub fn total_for_source(&self, source: MusicSearchSource) -> Option<usize> {
        match source {
            MusicSearchSource::Youtube => None,
            MusicSearchSource::YoutubeMusic => None,
            MusicSearchSource::Bandcamp => None,
            MusicSearchSource::Dailymotion => self.dm_total,
            MusicSearchSource::Videa => self.videa_total,
            MusicSearchSource::OkRu => self.okru_total,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SearchBatch {
    pub results: Vec<MusicSearchResult>,
    pub pagination: SearchPagination,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CastDevice {
    pub id: String,
    pub name: String,
    pub model: String,
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone)]
pub struct CastMediaItem {
    pub title: String,
    pub artist: String,
    pub url: String,
    pub thumbnail: Option<String>,
    pub is_audio: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastPlayerStatus {
    Connecting,
    Buffering,
    Playing,
    Paused,
    Idle,
}

#[derive(Debug, Clone)]
pub struct CastSessionState {
    pub device: CastDevice,
    pub title: String,
    pub artist: String,
    pub thumbnail: Option<String>,
    pub status: CastPlayerStatus,
    pub is_audio: bool,
    pub current_time: f64,
    pub duration: Option<f64>,
    pub seekable: bool,
    pub volume: f32,
    pub muted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadRequest {
    pub url: String,
    pub title: Option<String>,
    pub uploader: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub thumbnail: Option<String>,
    pub extractor: Option<String>,
    pub mode: String,    // "video" | "audio"
    pub format: String,  // "mp4" | "mkv" | "webm" | "best" | "flac" | "mp3" | "m4a" | "opus" | "wav"
    pub quality: String, // "best" | "2160" | "1440" | "1080" | "720" | "480" | "320" | "256" | "192" | "128"
    pub subtitles: bool,
    pub embed_thumbnail: bool,
    pub embed_metadata: bool,
    #[serde(default)]
    pub download_playlist: bool,
    pub output_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadItem {
    pub id: String,
    pub url: String,
    pub title: String,
    pub uploader: String,
    pub thumbnail: Option<String>,
    pub extractor: String,
    pub mode: String,
    pub format: String,
    pub quality: String,
    pub status: String, // "queued" | "downloading" | "processing" | "completed" | "failed" | "cancelled"
    pub progress: f64,
    pub speed: String,
    pub eta: String,
    pub total_size: String,
    pub playlist_progress: Option<String>,
    pub output_path: Option<String>,
    pub output_dir: String,
    pub error: Option<String>,
    pub created_at: String,
    pub request: DownloadRequest,
}

pub fn sanitize_folder_component(raw: &str, max_chars: usize) -> String {
    let mut cleaned: String = raw
        .trim()
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => ' ',
            _ => c,
        })
        .collect();

    while cleaned.contains("  ") {
        cleaned = cleaned.replace("  ", " ");
    }

    let trimmed = cleaned.trim().trim_matches('.').trim();
    if trimmed.is_empty() {
        return String::new();
    }

    trimmed.chars().take(max_chars).collect()
}

pub fn clean_artist_name(raw: &str) -> String {
    let trimmed = raw.trim();
    let without_topic = trimmed
        .strip_suffix(" - Topic")
        .or_else(|| trimmed.strip_suffix(" - topic"))
        .unwrap_or(trimmed)
        .trim();

    if without_topic.eq_ignore_ascii_case("Release")
        || without_topic.eq_ignore_ascii_case("Unknown Artist")
        || without_topic.eq_ignore_ascii_case("Ismeretlen feltöltő")
        || without_topic.eq_ignore_ascii_case("Ismeretlen csatorna")
    {
        return String::new();
    }

    sanitize_folder_component(without_topic, 80)
}

pub fn clean_album_title(raw: &str) -> String {
    let trimmed = raw.trim();
    let without_prefix = trimmed
        .strip_prefix("Album - ")
        .or_else(|| trimmed.strip_prefix("Album – "))
        .unwrap_or(trimmed)
        .trim();

    sanitize_folder_component(without_prefix, 100)
}

pub fn strip_leading_artist_from_album(artist: &str, album: &str) -> String {
    let a = clean_artist_name(artist);
    let t = clean_album_title(album);
    if a.is_empty() || t.is_empty() {
        return t;
    }

    let a_lower = a.to_ascii_lowercase();
    let t_lower = t.to_ascii_lowercase();
    for sep in [" - ", " – ", " — ", ": "] {
        let prefix = format!("{}{}", a_lower, sep);
        if t_lower.starts_with(&prefix) {
            if let Some(rest) = t.get(prefix.len()..).map(str::trim) {
                if !rest.is_empty() {
                    return sanitize_folder_component(rest, 100);
                }
            }
        }
    }

    t
}

pub fn build_album_relative_path(artist: &str, album: &str) -> Option<std::path::PathBuf> {
    let a = clean_artist_name(artist);
    let t = strip_leading_artist_from_album(artist, album);

    match (a.is_empty(), t.is_empty()) {
        (false, false) => Some(std::path::PathBuf::from(a).join(t)),
        (true, false) => Some(std::path::PathBuf::from(t)),
        (false, true) => Some(std::path::PathBuf::from(a)),
        (true, true) => None,
    }
}

pub fn build_album_folder_name(artist: &str, album: &str) -> Option<String> {
    build_album_relative_path(artist, album).map(|p| p.to_string_lossy().to_string())
}

pub fn is_yt_music_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    lower.contains("music.youtube.com")
}

pub fn is_bandcamp_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    lower.contains("bandcamp.com")
}

pub fn sanitize_bandcamp_url(raw: &str) -> String {
    let trimmed = raw.trim();
    let slice = if let Some(idx) = trimmed.rfind("https://") {
        &trimmed[idx..]
    } else if let Some(idx) = trimmed.rfind("http://") {
        &trimmed[idx..]
    } else {
        trimmed
    };
    let clean = slice.split('?').next().unwrap_or(slice).trim();
    if clean.starts_with("//") {
        format!("https:{}", clean)
    } else {
        clean.to_string()
    }
}

pub fn has_single_video_in_url(url: &str) -> bool {
    let trimmed = url.trim();
    let lower = trimmed.to_ascii_lowercase();
    if !lower.starts_with("http://") && !lower.starts_with("https://") {
        return false;
    }
    let is_yt = lower.contains("youtube.com") || lower.contains("youtu.be");
    if !is_yt {
        return false;
    }
    if lower.contains("youtu.be/") {
        if let Some((_, rest)) = lower.split_once("youtu.be/") {
            let vid = rest.split(['?', '#', '/']).next().unwrap_or("");
            if !vid.is_empty() {
                return true;
            }
        }
    }
    if lower.contains("/shorts/") {
        return true;
    }
    if let Some((_, query_and_frag)) = lower.split_once('?') {
        let query = query_and_frag.split('#').next().unwrap_or(query_and_frag);
        return query
            .split('&')
            .any(|kv| kv.starts_with("v=") && kv.len() > 2);
    }
    false
}

pub fn is_youtube_mix_or_radio_url(url: &str) -> bool {
    if !has_single_video_in_url(url) {
        return false;
    }
    let lower = url.trim().to_ascii_lowercase();
    if let Some((_, query_and_frag)) = lower.split_once('?') {
        let query = query_and_frag.split('#').next().unwrap_or(query_and_frag);
        return query
            .split('&')
            .any(|kv| kv.starts_with("list=rd") || kv == "start_radio=1");
    }
    false
}

pub fn strip_youtube_playlist_params(url: &str) -> String {
    let trimmed = url.trim();
    if !has_single_video_in_url(trimmed) {
        return trimmed.to_string();
    }
    let (without_frag, frag) = match trimmed.split_once('#') {
        Some((before, f)) => (before, Some(f)),
        None => (trimmed, None),
    };
    let Some((base, query)) = without_frag.split_once('?') else {
        return trimmed.to_string();
    };
    let kept: Vec<&str> = query
        .split('&')
        .filter(|kv| {
            let key = kv.split('=').next().unwrap_or("").to_ascii_lowercase();
            !matches!(key.as_str(), "list" | "index" | "start_radio")
        })
        .collect();
    let mut out = if kept.is_empty() {
        base.to_string()
    } else {
        format!("{}?{}", base, kept.join("&"))
    };
    if let Some(f) = frag {
        out.push('#');
        out.push_str(f);
    }
    out
}

pub fn detect_playlist_intent(url: &str) -> bool {
    let trimmed = url.trim();
    let lower = trimmed.to_ascii_lowercase();
    if !lower.starts_with("http://") && !lower.starts_with("https://") {
        return false;
    }
    if lower.contains("/playlist") || lower.contains("/browse/mpreb") {
        return true;
    }
    if is_bandcamp_url(&lower) && (lower.contains("/album/") || lower.ends_with("/music") || lower.contains("/music?")) {
        return true;
    }
    if let Some((_, query_and_frag)) = lower.split_once('?') {
        let query = query_and_frag.split('#').next().unwrap_or(query_and_frag);
        let has_list = query
            .split('&')
            .any(|kv| kv.starts_with("list=") && kv.len() > 5);
        let has_v = query
            .split('&')
            .any(|kv| kv.starts_with("v=") && kv.len() > 2);
        let is_mix_or_radio = query
            .split('&')
            .any(|kv| kv.starts_with("list=rd") || kv == "start_radio=1");
        if has_list && (!has_v || (lower.contains("music.youtube.com") && !is_mix_or_radio)) {
            return true;
        }
    }
    false
}

pub fn looks_like_url(value: &str) -> bool {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://"))
        && trimmed.len() > 10
        && !trimmed.contains(char::is_whitespace)
}

pub fn extract_domain_label(url: &str) -> String {
    let without_scheme = url
        .trim()
        .strip_prefix("https://")
        .or_else(|| url.trim().strip_prefix("http://"))
        .unwrap_or(url.trim());
    let host = without_scheme.split('/').next().unwrap_or("Web");
    let host = host.split(':').next().unwrap_or(host);
    host.strip_prefix("www.").unwrap_or(host).to_string()
}

pub fn format_duration(seconds: Option<f64>) -> String {
    match seconds {
        Some(s) if s.is_finite() && s > 0.0 => {
            let total = s.floor() as u64;
            let hrs = total / 3600;
            let mins = (total % 3600) / 60;
            let secs = total % 60;
            if hrs > 0 {
                format!("{}:{:02}:{:02}", hrs, mins, secs)
            } else {
                format!("{:02}:{:02}", mins, secs)
            }
        }
        _ => "—".to_string(),
    }
}
