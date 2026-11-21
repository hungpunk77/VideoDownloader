use crate::config;
use crate::downloader::{self, DownloadManager, UiEvent};
use crate::engines;
use crate::i18n;
use crate::search;
use crate::types::{
    build_album_folder_name, detect_playlist_intent, extract_domain_label, format_duration,
    has_single_video_in_url, is_bandcamp_url, is_yt_music_url, is_youtube_mix_or_radio_url,
    looks_like_url, strip_youtube_playlist_params, AppLanguage, AppSettings, DownloadItem,
    DownloadRequest, EngineStatus, MediaInfo, MusicSearchItemKind, MusicSearchResult,
    MusicSearchSource, ThemeMode,
};
use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveTab {
    Download,
    Search,
    Manager,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchTypeFilter {
    All,
    Videos,
    Albums,
    Songs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchSourceFilter {
    All,
    YoutubeMusic,
    Bandcamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManagerFilter {
    All,
    Active,
    Completed,
    Failed,
}

enum ThumbnailState {
    Loading,
    Ready(egui::TextureHandle),
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingClipboardAction {
    PasteAndInspect,
}

pub struct VideoDownloaderApp {
    rt: Runtime,
    manager: Arc<DownloadManager>,
    tx: mpsc::UnboundedSender<UiEvent>,
    rx: mpsc::UnboundedReceiver<UiEvent>,

    active_tab: ActiveTab,
    settings: AppSettings,
    settings_draft: AppSettings,
    settings_saved_notice: bool,

    // Engine status
    engine_status: Option<EngineStatus>,
    checking_engines: bool,
    updating_ytdlp: bool,
    updating_ffmpeg: bool,
    updating_deno: bool,
    banner_dismissed: bool,
    engine_error: Option<String>,

    // Studio state
    url: String,
    mode: String,
    video_format: String,
    video_quality: String,
    audio_format: String,
    audio_quality: String,
    subtitles: bool,
    embed_thumbnail: bool,
    embed_metadata: bool,
    download_playlist: bool,
    inspecting: bool,
    media_info: Option<MediaInfo>,
    studio_error: Option<String>,
    studio_is_auth_error: bool,
    clipboard_notice: Option<String>,
    pending_clipboard: Option<PendingClipboardAction>,
    pending_clipboard_frames: u8,

    // Downloads cache (synced from DownloadManager)
    downloads: Vec<DownloadItem>,

    // Manager filter
    mgr_filter: ManagerFilter,
    mgr_search: String,

    // Settings cookies input
    cookies_text: String,
    cookie_status_msg: Option<String>,
    cookie_error_msg: Option<String>,

    // Thumbnails
    thumbnails: HashMap<String, ThumbnailState>,

    // Main tab unified multi-platform search
    main_searching: bool,
    main_search_results: Vec<MusicSearchResult>,
    main_search_last_query: String,
    main_search_error: Option<String>,
    main_search_type_filter: SearchTypeFilter,
    main_search_source_filter: Option<MusicSearchSource>,
    main_search_selected_ids: HashSet<String>,
    main_search_queued_notice: Option<String>,
    main_search_seq: u64,

    // Music Search (YouTube Music & Bandcamp)
    search_query: String,
    search_include_ytm: bool,
    search_include_bandcamp: bool,
    search_type_filter: SearchTypeFilter,
    search_source_filter: SearchSourceFilter,
    searching: bool,
    search_results: Vec<MusicSearchResult>,
    search_last_query: String,
    search_error: Option<String>,
    search_selected_ids: HashSet<String>,
    search_queued_notice: Option<String>,

    // Borderless window & exit confirmation
    show_exit_confirm: bool,
    confirmed_exit: bool,
    pending_font_apply: bool,
    inspect_seq: u64,
    search_seq: u64,
}

impl VideoDownloaderApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let rt = Runtime::new().expect("Failed to initialize Tokio runtime");
        let manager = Arc::new(DownloadManager::new());
        let (tx, rx) = mpsc::unbounded_channel();

        let settings = config::load_settings();
        apply_visuals(&cc.egui_ctx, settings.theme);
        apply_font_styles(&cc.egui_ctx, settings.font_size, settings.small_font_size);

        let initial_downloads = rt.block_on(async {
            let guard = manager.items.lock().await;
            guard.clone()
        });

        let mut app = Self {
            rt,
            manager,
            tx,
            rx,
            active_tab: ActiveTab::Download,
            settings: settings.clone(),
            settings_draft: settings.clone(),
            settings_saved_notice: false,
            engine_status: None,
            checking_engines: false,
            updating_ytdlp: false,
            updating_ffmpeg: false,
            updating_deno: false,
            banner_dismissed: false,
            engine_error: None,
            url: String::new(),
            mode: settings.default_mode.clone(),
            video_format: settings.default_video_format.clone(),
            video_quality: settings.default_video_quality.clone(),
            audio_format: settings.default_audio_format.clone(),
            audio_quality: settings.default_audio_quality.clone(),
            subtitles: settings.download_subtitles,
            embed_thumbnail: settings.embed_thumbnail,
            embed_metadata: settings.embed_metadata,
            download_playlist: false,
            inspecting: false,
            media_info: None,
            studio_error: None,
            studio_is_auth_error: false,
            clipboard_notice: None,
            pending_clipboard: None,
            pending_clipboard_frames: 0,
            downloads: initial_downloads,
            mgr_filter: ManagerFilter::All,
            mgr_search: String::new(),
            cookies_text: String::new(),
            cookie_status_msg: None,
            cookie_error_msg: None,
            thumbnails: HashMap::new(),
            main_searching: false,
            main_search_results: Vec::new(),
            main_search_last_query: String::new(),
            main_search_error: None,
            main_search_type_filter: SearchTypeFilter::All,
            main_search_source_filter: None,
            main_search_selected_ids: HashSet::new(),
            main_search_queued_notice: None,
            main_search_seq: 0,
            search_query: String::new(),
            search_include_ytm: true,
            search_include_bandcamp: true,
            search_type_filter: SearchTypeFilter::All,
            search_source_filter: SearchSourceFilter::All,
            searching: false,
            search_results: Vec::new(),
            search_last_query: String::new(),
            search_error: None,
            search_selected_ids: HashSet::new(),
            search_queued_notice: None,
            show_exit_confirm: false,
            confirmed_exit: false,
            pending_font_apply: false,
            inspect_seq: 0,
            search_seq: 0,
        };

        app.startup_engine_check(cc.egui_ctx.clone());
        app
    }

    fn startup_engine_check(&mut self, ctx: egui::Context) {
        self.checking_engines = true;
        let tx = self.tx.clone();
        let auto_check = self.settings.auto_check_ytdlp;
        self.rt.spawn(async move {
            let status = engines::check_status().await;
            let needs_install =
                !status.installed || !status.ffmpeg_installed || !status.js_runtime_installed;
            if auto_check || needs_install {
                let _ = tx.send(UiEvent::EngineStatus(status));
                ctx.request_repaint();
            }
            if needs_install {
                match engines::ensure_engines().await {
                    Ok(updated) => {
                        let _ = tx.send(UiEvent::EngineStatus(updated));
                    }
                    Err(e) => {
                        let _ = tx.send(UiEvent::EngineActionError(e));
                    }
                }
                ctx.request_repaint();
            }
        });
    }

    fn check_engines(&mut self, ctx: egui::Context) {
        self.checking_engines = true;
        self.engine_error = None;
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let status = engines::check_status().await;
            let _ = tx.send(UiEvent::EngineStatus(status));
            ctx.request_repaint();
        });
    }

    fn update_ytdlp(&mut self, ctx: egui::Context) {
        self.updating_ytdlp = true;
        self.engine_error = None;
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match engines::update_or_install_ytdlp().await {
                Ok(status) => {
                    let _ = tx.send(UiEvent::EngineStatus(status));
                }
                Err(e) => {
                    let _ = tx.send(UiEvent::EngineActionError(e));
                }
            }
            ctx.request_repaint();
        });
    }

    fn update_ffmpeg(&mut self, ctx: egui::Context) {
        self.updating_ffmpeg = true;
        self.engine_error = None;
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match engines::update_or_install_ffmpeg().await {
                Ok(status) => {
                    let _ = tx.send(UiEvent::EngineStatus(status));
                }
                Err(e) => {
                    let _ = tx.send(UiEvent::EngineActionError(e));
                }
            }
            ctx.request_repaint();
        });
    }

    fn update_deno(&mut self, ctx: egui::Context) {
        self.updating_deno = true;
        self.engine_error = None;
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match engines::update_or_install_deno().await {
                Ok(status) => {
                    let _ = tx.send(UiEvent::EngineStatus(status));
                }
                Err(e) => {
                    let _ = tx.send(UiEvent::EngineActionError(e));
                }
            }
            ctx.request_repaint();
        });
    }

    fn update_all_engines(&mut self, ctx: egui::Context) {
        let need_yt = self
            .engine_status
            .as_ref()
            .map(|s| !s.installed || s.update_available)
            .unwrap_or(true);
        let need_ff = self
            .engine_status
            .as_ref()
            .map(|s| !s.ffmpeg_installed || s.ffmpeg_update_available)
            .unwrap_or(true);
        let need_deno = self
            .engine_status
            .as_ref()
            .map(|s| !s.js_runtime_installed || s.js_runtime_update_available)
            .unwrap_or(false);

        if need_yt {
            self.updating_ytdlp = true;
        }
        if need_ff {
            self.updating_ffmpeg = true;
        }
        if need_deno {
            self.updating_deno = true;
        }
        self.engine_error = None;
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            if need_yt {
                match engines::update_or_install_ytdlp().await {
                    Ok(status) => {
                        let _ = tx.send(UiEvent::EngineStatus(status));
                    }
                    Err(e) => {
                        let _ = tx.send(UiEvent::EngineActionError(e));
                        ctx.request_repaint();
                        return;
                    }
                }
            }
            if need_ff {
                match engines::update_or_install_ffmpeg().await {
                    Ok(status) => {
                        let _ = tx.send(UiEvent::EngineStatus(status));
                    }
                    Err(e) => {
                        let _ = tx.send(UiEvent::EngineActionError(e));
                        ctx.request_repaint();
                        return;
                    }
                }
            }
            if need_deno {
                match engines::update_or_install_deno().await {
                    Ok(status) => {
                        let _ = tx.send(UiEvent::EngineStatus(status));
                    }
                    Err(e) => {
                        let _ = tx.send(UiEvent::EngineActionError(e));
                    }
                }
            }
            ctx.request_repaint();
        });
    }

    fn inspect_url(&mut self, target: String, ctx: egui::Context) {
        let candidate = target.trim().to_string();
        if !looks_like_url(&candidate) {
            self.studio_error = Some(i18n::strings(self.settings.language).studio_invalid_url.to_string());
            self.studio_is_auth_error = false;
            return;
        }
        self.inspect_seq = self.inspect_seq.wrapping_add(1);
        let seq = self.inspect_seq;
        self.studio_error = None;
        self.studio_is_auth_error = false;
        self.inspecting = true;
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match downloader::inspect_media(candidate).await {
                Ok(info) => {
                    let _ = tx.send(UiEvent::InspectSuccess(seq, info));
                }
                Err(e) => {
                    let _ = tx.send(UiEvent::InspectError(seq, e));
                }
            }
            ctx.request_repaint();
        });
    }

    fn request_thumbnail(&mut self, url: &str, ctx: &egui::Context) {
        if self.thumbnails.contains_key(url) {
            return;
        }
        if self.thumbnails.len() > 250 {
            self.thumbnails.clear();
        }
        self.thumbnails
            .insert(url.to_string(), ThumbnailState::Loading);
        let url_owned = url.to_string();
        let tx = self.tx.clone();
        let ctx_clone = ctx.clone();
        self.rt.spawn(async move {
            let client = reqwest::Client::builder()
                .user_agent(concat!("VideoDownloader/", env!("CARGO_PKG_VERSION")))
                .timeout(std::time::Duration::from_secs(10))
                .build();
            let ok = match client {
                Ok(c) => match c.get(&url_owned).send().await {
                    Ok(resp) if resp.status().is_success() => match resp.bytes().await {
                        Ok(bytes) => {
                            let _ = tx.send(UiEvent::ThumbnailLoaded(url_owned.clone(), bytes.to_vec()));
                            true
                        }
                        Err(_) => false,
                    },
                    _ => false,
                },
                Err(_) => false,
            };
            if !ok {
                let _ = tx.send(UiEvent::ThumbnailFailed(url_owned));
            }
            ctx_clone.request_repaint();
        });
    }

    fn read_clipboard_text() -> Result<String, arboard::Error> {
        #[cfg(not(target_os = "linux"))]
        {
            arboard::Clipboard::new().and_then(|mut cb| cb.get_text())
        }
        #[cfg(target_os = "linux")]
        {
            if let Ok(text) = arboard::Clipboard::new().and_then(|mut cb| cb.get_text()) {
                if !text.trim().is_empty() {
                    return Ok(text);
                }
            }
            let mut candidates: Vec<(&str, &[&str])> = Vec::new();
            if std::env::var_os("WAYLAND_DISPLAY").is_some() {
                candidates.push(("wl-paste", &["--no-newline"]));
            }
            candidates.push(("xclip", &["-o", "-selection", "clipboard"]));
            candidates.push(("xsel", &["-ob"]));

            for (cmd, args) in candidates {
                if let Ok(out) = std::process::Command::new(cmd).args(args).output() {
                    if out.status.success() {
                        let text = String::from_utf8_lossy(&out.stdout).to_string();
                        if !text.trim().is_empty() {
                            return Ok(text);
                        }
                    }
                }
            }
            Err(arboard::Error::ContentNotAvailable)
        }
    }

    fn handle_pasted_text(&mut self, text: String, action: PendingClipboardAction, ctx: egui::Context) {
        let s = i18n::strings(self.settings.language);
        let trimmed = text.trim().to_string();
        if trimmed.is_empty() {
            self.studio_error = Some(s.studio_clipboard_empty.to_string());
            return;
        }
        match action {
            PendingClipboardAction::PasteAndInspect => {
                self.url = trimmed.clone();
                if looks_like_url(&trimmed) {
                    self.download_playlist = detect_playlist_intent(&trimmed);
                    if is_yt_music_url(&trimmed) || is_bandcamp_url(&trimmed) {
                        self.mode = "audio".to_string();
                    }
                    self.clipboard_notice = Some(s.studio_clipboard_ok.to_string());
                    self.inspect_url(trimmed, ctx);
                } else {
                    self.studio_error = Some(s.studio_clipboard_not_url.to_string());
                }
            }
        }
    }

    fn process_pending_clipboard(&mut self, ctx: &egui::Context) {
        let Some(action) = self.pending_clipboard else {
            return;
        };
        let pasted = ctx.input(|i| {
            i.events.iter().find_map(|ev| match ev {
                egui::Event::Paste(text) => Some(text.clone()),
                _ => None,
            })
        });
        if let Some(text) = pasted {
            self.pending_clipboard = None;
            self.pending_clipboard_frames = 0;
            self.handle_pasted_text(text, action, ctx.clone());
        } else if self.pending_clipboard_frames > 0 {
            self.pending_clipboard_frames -= 1;
            ctx.request_repaint();
        } else {
            self.pending_clipboard = None;
            let s = i18n::strings(self.settings.language);
            self.studio_error = Some(s.studio_clipboard_empty.to_string());
        }
    }

    fn paste_from_clipboard(&mut self, ctx: egui::Context) {
        self.studio_error = None;
        self.studio_is_auth_error = false;
        self.clipboard_notice = None;
        match Self::read_clipboard_text() {
            Ok(text) => {
                self.handle_pasted_text(text, PendingClipboardAction::PasteAndInspect, ctx);
            }
            Err(_) => {
                self.pending_clipboard = Some(PendingClipboardAction::PasteAndInspect);
                self.pending_clipboard_frames = 6;
                ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                ctx.request_repaint();
            }
        }
    }

    fn start_download(&mut self, force_playlist: Option<bool>, ctx: egui::Context) {
        let target_url = self.url.trim().to_string();

        if !looks_like_url(&target_url) {
            self.studio_error = Some(i18n::strings(self.settings.language).studio_invalid_url.to_string());
            self.studio_is_auth_error = false;
            return;
        }

        let effective_playlist = force_playlist.unwrap_or(self.download_playlist);

        let (dl_mode, dl_format, dl_quality) = if self.mode == "audio" || is_bandcamp_url(&target_url) {
            self.mode = "audio".to_string();
            ("audio".to_string(), self.audio_format.clone(), self.audio_quality.clone())
        } else {
            ("video".to_string(), self.video_format.clone(), self.video_quality.clone())
        };

        let matching_info = self
            .media_info
            .as_ref()
            .filter(|m| urls_match(&m.url, &target_url));

        let final_url = if !effective_playlist && has_single_video_in_url(&target_url) {
            strip_youtube_playlist_params(&target_url)
        } else {
            target_url.clone()
        };
        let single_meta_info = matching_info.filter(|m| effective_playlist || !m.is_playlist);

        let is_audio = dl_mode == "audio";
        let req = DownloadRequest {
            url: final_url,
            title: single_meta_info.map(|m| m.title.clone()),
            uploader: single_meta_info.map(|m| m.uploader.clone()),
            album: single_meta_info.and_then(|m| m.album.clone()),
            album_artist: single_meta_info.and_then(|m| m.album_artist.clone()),
            thumbnail: matching_info.and_then(|m| m.thumbnail.clone()),
            extractor: Some(
                matching_info
                    .map(|m| m.extractor.clone())
                    .unwrap_or_else(|| extract_domain_label(&target_url)),
            ),
            mode: dl_mode,
            format: dl_format,
            quality: dl_quality,
            subtitles: self.subtitles,
            embed_thumbnail: is_audio && self.embed_thumbnail,
            embed_metadata: self.embed_metadata,
            download_playlist: effective_playlist,
            output_dir: Some(self.settings.download_dir.clone()),
        };

        self.studio_error = None;
        self.studio_is_auth_error = false;
        self.url.clear();
        if self.media_info.is_some() {
            self.video_format = self.settings.default_video_format.clone();
            self.video_quality = self.settings.default_video_quality.clone();
            self.audio_format = self.settings.default_audio_format.clone();
            self.audio_quality = self.settings.default_audio_quality.clone();
        }
        self.media_info = None;
        self.clipboard_notice = None;

        let manager = self.manager.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let _ = downloader::enqueue_download(manager, tx, req).await;
            ctx.request_repaint();
        });
    }

    fn run_main_search(&mut self, ctx: egui::Context) {
        let query = self.url.trim().to_string();
        let s = i18n::strings(self.settings.language);
        if query.is_empty() {
            self.main_search_error = Some(s.search_need_query.to_string());
            return;
        }
        if looks_like_url(&query) {
            self.inspect_url(query, ctx);
            return;
        }
        self.main_search_seq = self.main_search_seq.wrapping_add(1);
        let seq = self.main_search_seq;
        self.main_searching = true;
        self.main_search_error = None;
        self.main_search_queued_notice = None;
        self.studio_error = None;
        self.studio_is_auth_error = false;
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match search::search_all_supported(query.clone()).await {
                Ok(results) => {
                    let _ = tx.send(UiEvent::MainSearchSuccess(seq, query, results));
                }
                Err(e) => {
                    let _ = tx.send(UiEvent::MainSearchError(seq, e));
                }
            }
            ctx.request_repaint();
        });
    }

    fn run_music_search(&mut self, ctx: egui::Context) {
        let query = self.search_query.trim().to_string();
        let s = i18n::strings(self.settings.language);
        if query.is_empty() {
            self.search_error = Some(s.search_need_query.to_string());
            return;
        }
        if !self.search_include_ytm && !self.search_include_bandcamp {
            self.search_error = Some(s.search_need_source.to_string());
            return;
        }
        self.search_seq = self.search_seq.wrapping_add(1);
        let seq = self.search_seq;
        self.searching = true;
        self.search_error = None;
        self.search_queued_notice = None;
        let include_ytm = self.search_include_ytm;
        let include_bc = self.search_include_bandcamp;
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match search::search_music(query.clone(), include_ytm, include_bc).await {
                Ok(results) => {
                    let _ = tx.send(UiEvent::SearchSuccess(seq, query, results));
                }
                Err(e) => {
                    let _ = tx.send(UiEvent::SearchError(seq, e));
                }
            }
            ctx.request_repaint();
        });
    }

    fn build_search_download_req(&self, item: &MusicSearchResult) -> DownloadRequest {
        match item.kind {
            MusicSearchItemKind::Video => DownloadRequest {
                url: item.url.clone(),
                title: Some(item.title.clone()),
                uploader: Some(item.artist.clone()),
                album: None,
                album_artist: None,
                thumbnail: item.thumbnail.clone(),
                extractor: Some(item.source.label().to_string()),
                mode: "video".to_string(),
                format: self.video_format.clone(),
                quality: self.video_quality.clone(),
                subtitles: self.subtitles,
                embed_thumbnail: false,
                embed_metadata: self.embed_metadata,
                download_playlist: false,
                output_dir: Some(self.settings.download_dir.clone()),
            },
            MusicSearchItemKind::Album => DownloadRequest {
                url: item.url.clone(),
                title: Some(item.title.clone()),
                uploader: Some(item.artist.clone()),
                album: Some(item.title.clone()),
                album_artist: Some(item.artist.clone()),
                thumbnail: item.thumbnail.clone(),
                extractor: Some(item.source.label().to_string()),
                mode: "audio".to_string(),
                format: self.audio_format.clone(),
                quality: self.audio_quality.clone(),
                subtitles: false,
                embed_thumbnail: self.embed_thumbnail,
                embed_metadata: self.embed_metadata,
                download_playlist: true,
                output_dir: Some(self.settings.download_dir.clone()),
            },
            MusicSearchItemKind::Song => DownloadRequest {
                url: item.url.clone(),
                title: Some(item.title.clone()),
                uploader: Some(item.artist.clone()),
                album: item.album.clone(),
                album_artist: Some(item.artist.clone()),
                thumbnail: item.thumbnail.clone(),
                extractor: Some(item.source.label().to_string()),
                mode: "audio".to_string(),
                format: self.audio_format.clone(),
                quality: self.audio_quality.clone(),
                subtitles: false,
                embed_thumbnail: self.embed_thumbnail,
                embed_metadata: self.embed_metadata,
                download_playlist: false,
                output_dir: Some(self.settings.download_dir.clone()),
            },
        }
    }

    fn download_search_result(
        &mut self,
        item: &MusicSearchResult,
        to_main_notice: bool,
        ctx: egui::Context,
    ) {
        let req = self.build_search_download_req(item);
        let s = i18n::strings(self.settings.language);
        let kind_msg = match item.kind {
            MusicSearchItemKind::Video => s.search_queued_video,
            MusicSearchItemKind::Album => s.search_queued_album,
            MusicSearchItemKind::Song => s.search_queued_song,
        };
        let notice = format!("✓ {}: {} — {}", kind_msg, item.artist, item.title);
        if to_main_notice {
            self.main_search_queued_notice = Some(notice);
        } else {
            self.search_queued_notice = Some(notice);
        }

        let manager = self.manager.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let _ = downloader::enqueue_download(manager, tx, req).await;
            ctx.request_repaint();
        });
    }

    fn download_selected_search_results(
        &mut self,
        items: Vec<MusicSearchResult>,
        to_main_notice: bool,
        ctx: egui::Context,
    ) {
        if items.is_empty() {
            return;
        }
        let count = items.len();
        let reqs: Vec<DownloadRequest> = items
            .iter()
            .map(|item| self.build_search_download_req(item))
            .collect();

        let s = i18n::strings(self.settings.language);
        let notice = format!("✓ {} {}", count, s.search_queued_multi);
        if to_main_notice {
            self.main_search_queued_notice = Some(notice);
        } else {
            self.search_queued_notice = Some(notice);
        }

        let manager = self.manager.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            for req in reqs {
                let _ = downloader::enqueue_download(manager.clone(), tx.clone(), req).await;
            }
            ctx.request_repaint();
        });
    }

    fn inspect_search_result(&mut self, item: &MusicSearchResult, ctx: egui::Context) {
        self.active_tab = ActiveTab::Download;
        self.url = item.url.clone();
        self.mode = if item.kind == MusicSearchItemKind::Video {
            "video".to_string()
        } else {
            "audio".to_string()
        };
        self.download_playlist = item.kind == MusicSearchItemKind::Album;
        self.clipboard_notice = None;
        self.studio_error = None;
        self.studio_is_auth_error = false;
        self.inspect_url(item.url.clone(), ctx);
    }

    fn process_events(&mut self, ctx: &egui::Context) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                UiEvent::EngineStatus(status) => {
                    self.checking_engines = false;
                    self.updating_ytdlp = false;
                    self.updating_ffmpeg = false;
                    self.updating_deno = false;
                    if !status.installed
                        || !status.ffmpeg_installed
                        || !status.js_runtime_installed
                        || status.update_available
                        || status.ffmpeg_update_available
                        || status.js_runtime_update_available
                    {
                        self.banner_dismissed = false;
                    }
                    self.engine_status = Some(status);
                }
                UiEvent::EngineActionError(err) => {
                    self.checking_engines = false;
                    self.updating_ytdlp = false;
                    self.updating_ffmpeg = false;
                    self.updating_deno = false;
                    self.engine_error = Some(err);
                }
                UiEvent::InspectSuccess(seq, info) => {
                    if seq != self.inspect_seq {
                        continue;
                    }
                    self.inspecting = false;
                    if info.is_playlist {
                        if !is_youtube_mix_or_radio_url(&info.url)
                            && !is_youtube_mix_or_radio_url(&self.url)
                        {
                            self.download_playlist = true;
                        }
                        if info.is_yt_music
                            || info.extractor.to_ascii_lowercase().contains("bandcamp")
                            || (info.available_heights.is_empty()
                                && !info.available_audio_formats.is_empty())
                        {
                            self.mode = "audio".to_string();
                        }
                    } else if info.available_heights.is_empty() && !info.available_audio_formats.is_empty() {
                        self.mode = "audio".to_string();
                    }

                    if !info.available_video_formats.is_empty()
                        && !info.available_video_formats.contains(&self.video_format)
                    {
                        self.video_format = info.available_video_formats[0].clone();
                    }
                    if !info.available_heights.is_empty()
                        && (self.video_quality == "best"
                            || !info
                                .available_heights
                                .iter()
                                .any(|h| h.to_string() == self.video_quality))
                    {
                        self.video_quality = info.available_heights[0].to_string();
                    }
                    if !info.available_audio_formats.is_empty()
                        && !info.available_audio_formats.contains(&self.audio_format)
                    {
                        self.audio_format = info.available_audio_formats[0].clone();
                    }
                    if !info.available_audio_qualities.is_empty()
                        && !info.available_audio_qualities.contains(&self.audio_quality)
                    {
                        self.audio_quality = info.available_audio_qualities[0].clone();
                    }

                    if let Some(thumb) = info.thumbnail.as_deref() {
                        self.request_thumbnail(thumb, ctx);
                    }
                    self.media_info = Some(info);
                }
                UiEvent::InspectError(seq, err) => {
                    if seq != self.inspect_seq {
                        continue;
                    }
                    self.inspecting = false;
                    self.media_info = None;
                    self.studio_is_auth_error = err.contains("[BOT_CHECK]")
                        || err.contains("[NO_BROWSER_DB]")
                        || err.contains("Sign in to confirm")
                        || err.contains("cookies");
                    self.studio_error =
                        Some(i18n::translate_backend_error(&err, self.settings.language));
                }
                UiEvent::SearchSuccess(seq, q, results) => {
                    if seq != self.search_seq {
                        continue;
                    }
                    self.searching = false;
                    self.search_last_query = q;
                    self.search_error = None;
                    self.search_selected_ids.clear();
                    for item in results.iter().take(16) {
                        if let Some(thumb) = item.thumbnail.as_deref() {
                            self.request_thumbnail(thumb, ctx);
                        }
                    }
                    self.search_results = results;
                }
                UiEvent::SearchError(seq, err) => {
                    if seq != self.search_seq {
                        continue;
                    }
                    self.searching = false;
                    self.search_error = Some(err);
                }
                UiEvent::MainSearchSuccess(seq, q, results) => {
                    if seq != self.main_search_seq {
                        continue;
                    }
                    self.main_searching = false;
                    self.main_search_last_query = q;
                    self.main_search_error = None;
                    self.main_search_selected_ids.clear();
                    self.main_search_type_filter = SearchTypeFilter::All;
                    self.main_search_source_filter = None;
                    for item in results.iter().take(16) {
                        if let Some(thumb) = item.thumbnail.as_deref() {
                            self.request_thumbnail(thumb, ctx);
                        }
                    }
                    self.main_search_results = results;
                }
                UiEvent::MainSearchError(seq, err) => {
                    if seq != self.main_search_seq {
                        continue;
                    }
                    self.main_searching = false;
                    self.main_search_error = Some(err);
                }
                UiEvent::DownloadUpdated(updated) => {
                    if let Some(pos) = self.downloads.iter().position(|d| d.id == updated.id) {
                        self.downloads[pos] = updated;
                    } else {
                        self.downloads.insert(0, updated);
                    }
                }
                UiEvent::ThumbnailLoaded(url, bytes) => {
                    if let Ok(img) = image::load_from_memory(&bytes) {
                        let rgba = img.into_rgba8();
                        let size = [rgba.width() as usize, rgba.height() as usize];
                        let color_image =
                            egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                        let tex = ctx.load_texture(
                            format!("thumb_{}", url),
                            color_image,
                            egui::TextureOptions::LINEAR,
                        );
                        self.thumbnails.insert(url, ThumbnailState::Ready(tex));
                    } else {
                        self.thumbnails.insert(url, ThumbnailState::Failed);
                    }
                }
                UiEvent::ThumbnailFailed(url) => {
                    self.thumbnails.insert(url, ThumbnailState::Failed);
                }
            }
        }
    }

    fn toggle_theme(&mut self, ctx: &egui::Context) {
        let next = match self.settings.theme {
            ThemeMode::Dark => ThemeMode::Light,
            ThemeMode::Light => ThemeMode::Dark,
        };
        self.settings.theme = next;
        self.settings_draft.theme = next;
        apply_visuals(ctx, next);
        apply_font_styles(ctx, self.settings.font_size, self.settings.small_font_size);
        let _ = config::save_settings(&self.settings);
    }

    fn set_language(&mut self, lang: AppLanguage) {
        self.settings.language = lang;
        self.settings_draft.language = lang;
        let _ = config::save_settings(&self.settings);
    }

    fn font_scale(&self) -> f32 {
        (self.settings.font_size / 14.0).clamp(0.75, 1.8)
    }
}

fn apply_font_styles(ctx: &egui::Context, base_size: f32, small_size: f32) {
    let base = base_size.clamp(11.0, 22.0);
    let small = small_size.clamp(9.0, 18.0);
    ctx.style_mut(|style| {
        style.text_styles.insert(
            egui::TextStyle::Small,
            egui::FontId::new(small, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Body,
            egui::FontId::new(base, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Button,
            egui::FontId::new(base, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Monospace,
            egui::FontId::new((base - 0.5).max(10.0), egui::FontFamily::Monospace),
        );
        style.text_styles.insert(
            egui::TextStyle::Heading,
            egui::FontId::new(base * 1.45, egui::FontFamily::Proportional),
        );
    });
}

fn apply_visuals(ctx: &egui::Context, theme: ThemeMode) {
    let mut dark_v = egui::Visuals::dark();
    dark_v.panel_fill = egui::Color32::from_rgb(15, 23, 42); // slate-900
    dark_v.window_fill = egui::Color32::from_rgb(15, 23, 42);
    dark_v.extreme_bg_color = egui::Color32::from_rgb(2, 6, 23); // slate-950
    dark_v.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(30, 41, 59);

    let mut light_v = egui::Visuals::light();
    light_v.panel_fill = egui::Color32::from_rgb(248, 250, 252);
    light_v.window_fill = egui::Color32::WHITE;

    ctx.set_visuals_of(egui::Theme::Dark, dark_v.clone());
    ctx.set_visuals_of(egui::Theme::Light, light_v.clone());

    match theme {
        ThemeMode::Dark => {
            ctx.set_theme(egui::ThemePreference::Dark);
            ctx.set_visuals(dark_v);
        }
        ThemeMode::Light => {
            ctx.set_theme(egui::ThemePreference::Light);
            ctx.set_visuals(light_v);
        }
    }
}

impl eframe::App for VideoDownloaderApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.pending_font_apply {
            self.pending_font_apply = false;
            self.settings.font_size = self.settings_draft.font_size.clamp(11.0, 22.0);
            self.settings.small_font_size = self.settings_draft.small_font_size.clamp(9.0, 18.0);
            self.settings_draft.font_size = self.settings.font_size;
            self.settings_draft.small_font_size = self.settings.small_font_size;
            apply_font_styles(ctx, self.settings.font_size, self.settings.small_font_size);
            let _ = config::save_settings(&self.settings);
        }

        let want_dark = self.settings.theme == ThemeMode::Dark;
        if ctx.style().visuals.dark_mode != want_dark {
            apply_visuals(ctx, self.settings.theme);
            apply_font_styles(ctx, self.settings.font_size, self.settings.small_font_size);
        } else {
            let cur_body = ctx
                .style()
                .text_styles
                .get(&egui::TextStyle::Body)
                .map(|f| f.size);
            let cur_small = ctx
                .style()
                .text_styles
                .get(&egui::TextStyle::Small)
                .map(|f| f.size);
            if cur_body != Some(self.settings.font_size)
                || cur_small != Some(self.settings.small_font_size)
            {
                apply_font_styles(ctx, self.settings.font_size, self.settings.small_font_size);
            }
        }

        self.process_events(ctx);
        self.process_pending_clipboard(ctx);

        let close_requested = ctx.input(|i| i.viewport().close_requested());
        if close_requested && !self.confirmed_exit {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.show_exit_confirm = true;
        }

        let popup_open = ctx.memory(|m| m.any_popup_open());
        if !popup_open && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.show_exit_confirm {
                self.show_exit_confirm = false;
            } else {
                self.show_exit_confirm = true;
            }
        }

        let has_active = self.downloads.iter().any(|d| {
            d.status == "downloading" || d.status == "queued" || d.status == "processing"
        });
        if has_active
            || self.inspecting
            || self.searching
            || self.main_searching
            || self.checking_engines
            || self.updating_ytdlp
            || self.updating_ffmpeg
            || self.updating_deno
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        let s = i18n::strings(self.settings.language);
        let scale = self.font_scale();
        let is_fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));

        // 8-direction edge/corner resize support for borderless window
        let resize_dir = if !is_fullscreen && !self.show_exit_confirm && !popup_open {
            let screen = ctx.screen_rect();
            let margin = 6.0_f32;
            ctx.input(|i| {
                i.pointer.hover_pos().and_then(|pos| {
                    let left = pos.x <= screen.left() + margin;
                    let right = pos.x >= screen.right() - margin;
                    let top = pos.y <= screen.top() + margin;
                    let bottom = pos.y >= screen.bottom() - margin;
                    match (left, right, top, bottom) {
                        (true, _, true, _) => Some(egui::ResizeDirection::NorthWest),
                        (_, true, true, _) => Some(egui::ResizeDirection::NorthEast),
                        (true, _, _, true) => Some(egui::ResizeDirection::SouthWest),
                        (_, true, _, true) => Some(egui::ResizeDirection::SouthEast),
                        (true, _, _, _) => Some(egui::ResizeDirection::West),
                        (_, true, _, _) => Some(egui::ResizeDirection::East),
                        (_, _, true, _) => Some(egui::ResizeDirection::North),
                        (_, _, _, true) => Some(egui::ResizeDirection::South),
                        _ => None,
                    }
                })
            })
        } else {
            None
        };

        if let Some(dir) = resize_dir {
            let icon = match dir {
                egui::ResizeDirection::North | egui::ResizeDirection::South => {
                    egui::CursorIcon::ResizeVertical
                }
                egui::ResizeDirection::East | egui::ResizeDirection::West => {
                    egui::CursorIcon::ResizeHorizontal
                }
                egui::ResizeDirection::NorthWest | egui::ResizeDirection::SouthEast => {
                    egui::CursorIcon::ResizeNwSe
                }
                egui::ResizeDirection::NorthEast | egui::ResizeDirection::SouthWest => {
                    egui::CursorIcon::ResizeNeSw
                }
            };
            ctx.set_cursor_icon(icon);
            if ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary)) {
                ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
            }
        }

        let active_count = self
            .downloads
            .iter()
            .filter(|d| d.status == "downloading" || d.status == "queued" || d.status == "processing")
            .count();

        // Sidebar
        let sidebar_width = (256.0 + (self.settings.font_size - 14.0).max(0.0) * 10.0).clamp(256.0, 355.0);
        egui::SidePanel::left("sidebar")
            .exact_width(sidebar_width)
            .resizable(false)
            .show(ctx, |ui| {
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    let badge_bg = egui::Color32::from_rgb(37, 99, 235);
                    egui::Frame::none()
                        .fill(badge_bg)
                        .rounding(8.0)
                        .inner_margin(egui::Margin::symmetric(8.0, 6.0))
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new("VD")
                                    .strong()
                                    .color(egui::Color32::WHITE)
                                    .size(13.0 * scale),
                            );
                        });
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new("VideoDownloader").strong().size(16.0 * scale));
                    ui.label(
                        egui::RichText::new(concat!("v", env!("CARGO_PKG_VERSION")))
                            .small()
                            .monospace()
                            .color(egui::Color32::from_rgb(96, 165, 250)),
                    );
                });

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(8.0);

                let nav_items = [
                    (ActiveTab::Download, s.nav_download, None),
                    (
                        ActiveTab::Search,
                        s.nav_search,
                        if !self.search_results.is_empty() {
                            Some(self.search_results.len())
                        } else {
                            None
                        },
                    ),
                    (
                        ActiveTab::Manager,
                        s.nav_manager,
                        if active_count > 0 {
                            Some(active_count)
                        } else {
                            None
                        },
                    ),
                    (ActiveTab::Settings, s.nav_settings, None),
                ];

                for (tab, label, count) in nav_items {
                    let selected = self.active_tab == tab;
                    let text = if let Some(c) = count {
                        format!("{} ({})", label, c)
                    } else {
                        label.to_string()
                    };
                    if ui
                        .add_sized(
                            [ui.available_width(), (36.0 * scale.max(1.0)).round()],
                            egui::SelectableLabel::new(
                                selected,
                                egui::RichText::new(text).size(14.0 * scale),
                            ),
                        )
                        .clicked()
                    {
                        self.active_tab = tab;
                        if tab != ActiveTab::Settings {
                            self.settings_saved_notice = false;
                        }
                    }
                    ui.add_space(4.0);
                }

                // Bottom engine status panel
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.add_space(8.0);
                    egui::Frame::group(ui.style())
                        .inner_margin(12.0)
                        .rounding(8.0)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            let is_busy = self.checking_engines
                                || self.updating_ytdlp
                                || self.updating_ffmpeg
                                || self.updating_deno;

                            let st_snapshot = self.engine_status.clone();
                            let need_yt = st_snapshot
                                .as_ref()
                                .map(|st| !st.installed || st.update_available)
                                .unwrap_or(false);
                            let need_ff = st_snapshot
                                .as_ref()
                                .map(|st| !st.ffmpeg_installed || st.ffmpeg_update_available)
                                .unwrap_or(false);
                            let need_deno = st_snapshot
                                .as_ref()
                                .map(|st| !st.js_runtime_installed || st.js_runtime_update_available)
                                .unwrap_or(false);
                            let update_count =
                                usize::from(need_yt) + usize::from(need_ff) + usize::from(need_deno);

                            // Action buttons (bottom-up layout: put bottom buttons first or use top-down inside frame)
                            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(s.engine_title).small().weak());
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui
                                                .add_enabled(
                                                    !is_busy,
                                                    egui::Button::new(s.engine_check).small(),
                                                )
                                                .clicked()
                                            {
                                                self.check_engines(ctx.clone());
                                            }
                                        },
                                    );
                                });
                                ui.add_space(6.0);

                                let yt_ver = st_snapshot
                                    .as_ref()
                                    .and_then(|st| st.current_version.clone())
                                    .unwrap_or_else(|| s.engine_not_installed.to_string());
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("yt-dlp").small().weak());
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(egui::RichText::new(yt_ver).small().monospace());
                                        },
                                    );
                                });

                                if let Some(st) = &st_snapshot {
                                    if st.update_available {
                                        if let Some(latest) = &st.latest_version {
                                            ui.horizontal(|ui| {
                                                ui.label(
                                                    egui::RichText::new(s.engine_latest)
                                                        .small()
                                                        .weak(),
                                                );
                                                ui.with_layout(
                                                    egui::Layout::right_to_left(egui::Align::Center),
                                                    |ui| {
                                                        ui.label(
                                                            egui::RichText::new(latest)
                                                                .small()
                                                                .monospace()
                                                                .color(egui::Color32::from_rgb(
                                                                    245, 158, 11,
                                                                )),
                                                        );
                                                    },
                                                );
                                            });
                                        }
                                    }
                                }

                                let ff_display = match &st_snapshot {
                                    None => "—".to_string(),
                                    Some(st) if !st.ffmpeg_installed => s.engine_missing.to_string(),
                                    Some(st) => match &st.ffmpeg_version {
                                        Some(v) if st.ffmpeg_source == "system" => {
                                            format!("{} ({})", v, s.engine_system)
                                        }
                                        Some(v) => v.clone(),
                                        None => s.engine_available.to_string(),
                                    },
                                };
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("ffmpeg").small().weak());
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(
                                                egui::RichText::new(ff_display).small().monospace(),
                                            );
                                        },
                                    );
                                });

                                let js_display = match &st_snapshot {
                                    None => "—".to_string(),
                                    Some(st) if !st.js_runtime_installed => {
                                        s.engine_missing.to_string()
                                    }
                                    Some(st) => {
                                        let name = st.js_runtime_name.as_deref().unwrap_or("JS");
                                        match (&st.js_runtime_version, st.js_runtime_source.as_str()) {
                                            (Some(v), "system") => {
                                                format!("{} {} ({})", name, v, s.engine_system)
                                            }
                                            (None, "system") => {
                                                format!("{} ({})", name, s.engine_system)
                                            }
                                            (Some(v), _) => format!("{} {}", name, v),
                                            (None, _) => name.to_string(),
                                        }
                                    }
                                };
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("JS (EJS)").small().weak());
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(
                                                egui::RichText::new(js_display).small().monospace(),
                                            );
                                        },
                                    );
                                });

                                ui.add_space(6.0);

                                if update_count >= 2 {
                                    let btn_lbl = if self.updating_ytdlp {
                                        s.engine_updating_ytdlp
                                    } else if self.updating_ffmpeg {
                                        s.engine_updating_ffmpeg
                                    } else if self.updating_deno {
                                        s.engine_updating_deno
                                    } else {
                                        s.engine_update_all
                                    };
                                    if ui
                                        .add_enabled(
                                            !is_busy,
                                            egui::Button::new(
                                                egui::RichText::new(btn_lbl)
                                                    .color(egui::Color32::WHITE)
                                                    .small(),
                                            )
                                            .fill(egui::Color32::from_rgb(217, 119, 6))
                                            .min_size(egui::vec2(ui.available_width(), 28.0)),
                                        )
                                        .clicked()
                                    {
                                        self.update_all_engines(ctx.clone());
                                    }
                                } else if let Some(st) = &st_snapshot {
                                    if !st.installed || st.update_available {
                                        let lbl = if !st.installed {
                                            if self.updating_ytdlp {
                                                s.engine_installing_ytdlp.to_string()
                                            } else {
                                                s.engine_install_ytdlp.to_string()
                                            }
                                        } else if self.updating_ytdlp {
                                            s.engine_updating_ytdlp.to_string()
                                        } else {
                                            format!(
                                                "{} ({})",
                                                s.engine_update_ytdlp,
                                                st.latest_version.as_deref().unwrap_or("")
                                            )
                                        };
                                        if ui
                                            .add_enabled(
                                                !self.updating_ytdlp,
                                                egui::Button::new(
                                                    egui::RichText::new(lbl)
                                                        .color(egui::Color32::WHITE)
                                                        .small(),
                                                )
                                                .fill(egui::Color32::from_rgb(37, 99, 235))
                                                .min_size(egui::vec2(
                                                    ui.available_width(),
                                                    28.0,
                                                )),
                                            )
                                            .clicked()
                                        {
                                            self.update_ytdlp(ctx.clone());
                                        }
                                        ui.add_space(4.0);
                                    }

                                    // Always allow updating/installing ffmpeg from the app!
                                    let ff_lbl = if !st.ffmpeg_installed {
                                        if self.updating_ffmpeg {
                                            s.engine_installing_ffmpeg.to_string()
                                        } else {
                                            s.engine_install_ffmpeg.to_string()
                                        }
                                    } else if st.ffmpeg_update_available {
                                        if self.updating_ffmpeg {
                                            s.engine_updating_ffmpeg.to_string()
                                        } else {
                                            format!(
                                                "{} ({})",
                                                s.engine_update_ffmpeg,
                                                st.ffmpeg_latest_version
                                                    .as_deref()
                                                    .unwrap_or("")
                                            )
                                        }
                                    } else if self.updating_ffmpeg {
                                        s.engine_updating_ffmpeg.to_string()
                                    } else {
                                        s.engine_reinstall_ffmpeg.to_string()
                                    };
                                    let ff_fill = if !st.ffmpeg_installed
                                        || st.ffmpeg_update_available
                                    {
                                        egui::Color32::from_rgb(217, 119, 6)
                                    } else {
                                        egui::Color32::from_rgb(51, 65, 85)
                                    };
                                    if ui
                                        .add_enabled(
                                            !self.updating_ffmpeg,
                                            egui::Button::new(
                                                egui::RichText::new(ff_lbl)
                                                    .color(egui::Color32::WHITE)
                                                    .small(),
                                            )
                                            .fill(ff_fill)
                                            .min_size(egui::vec2(ui.available_width(), 26.0)),
                                        )
                                        .clicked()
                                    {
                                        self.update_ffmpeg(ctx.clone());
                                    }

                                    if !st.js_runtime_installed || st.js_runtime_update_available {
                                        ui.add_space(4.0);
                                        let deno_lbl = if !st.js_runtime_installed {
                                            if self.updating_deno {
                                                s.engine_installing_deno.to_string()
                                            } else {
                                                s.engine_install_deno.to_string()
                                            }
                                        } else if self.updating_deno {
                                            s.engine_updating_deno.to_string()
                                        } else {
                                            format!(
                                                "{} ({})",
                                                s.engine_update_deno,
                                                st.js_runtime_latest_version
                                                    .as_deref()
                                                    .unwrap_or("")
                                            )
                                        };
                                        if ui
                                            .add_enabled(
                                                !self.updating_deno,
                                                egui::Button::new(
                                                    egui::RichText::new(deno_lbl)
                                                        .color(egui::Color32::WHITE)
                                                        .small(),
                                                )
                                                .fill(egui::Color32::from_rgb(217, 119, 6))
                                                .min_size(egui::vec2(ui.available_width(), 26.0)),
                                            )
                                            .clicked()
                                        {
                                            self.update_deno(ctx.clone());
                                        }
                                    }
                                }

                                if let Some(st) = &st_snapshot {
                                    if st.installed
                                        && st.ffmpeg_installed
                                        && st.js_runtime_installed
                                        && !st.update_available
                                        && !st.ffmpeg_update_available
                                        && !st.js_runtime_update_available
                                        && st.error.is_none()
                                    {
                                        ui.add_space(4.0);
                                        ui.label(
                                            egui::RichText::new(format!("✓ {}", s.engine_up_to_date))
                                                .small()
                                                .color(egui::Color32::from_rgb(16, 185, 129)),
                                        );
                                    }
                                }
                            });
                        });
                });
            });

        // Borderless custom header bar with drag support & top-right Fullscreen / Window controls
        egui::TopBottomPanel::top("header_bar").show(ctx, |ui| {
            ui.add_space(6.0);
            let mut controls_hovered = false;
            let row_resp = ui.horizontal(|ui| {
                let tab_name = match self.active_tab {
                    ActiveTab::Download => s.nav_download,
                    ActiveTab::Search => s.nav_search,
                    ActiveTab::Manager => s.nav_manager,
                    ActiveTab::Settings => s.nav_settings,
                };
                ui.label(
                    egui::RichText::new(concat!("VideoDownloader v", env!("CARGO_PKG_VERSION"), " /"))
                        .weak(),
                );
                ui.label(egui::RichText::new(tab_name).strong());

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let close_resp = draw_close_button(ui, s.win_close);
                    controls_hovered |= close_resp.hovered();
                    if close_resp.clicked() {
                        self.show_exit_confirm = true;
                    }

                    ui.add_space(2.0);

                    let fs_tooltip = if is_fullscreen {
                        s.win_fullscreen_exit
                    } else {
                        s.win_fullscreen_enter
                    };
                    let fs_resp = draw_fullscreen_button(ui, is_fullscreen, fs_tooltip);
                    controls_hovered |= fs_resp.hovered();
                    if fs_resp.clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!is_fullscreen));
                    }

                    ui.add_space(2.0);

                    let min_resp = draw_minimize_button(ui, s.win_minimize);
                    controls_hovered |= min_resp.hovered();
                    if min_resp.clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                    }

                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(4.0);

                    let mut chosen_lang = self.settings.language;
                    let lang_resp = egui::ComboBox::from_id_salt("header_lang_combo")
                        .width(96.0)
                        .selected_text(chosen_lang.code().to_uppercase())
                        .show_ui(ui, |ui| {
                            for lang in [AppLanguage::En, AppLanguage::De, AppLanguage::Hu] {
                                ui.selectable_value(
                                    &mut chosen_lang,
                                    lang,
                                    format!("{} · {}", lang.code().to_uppercase(), lang.display_name()),
                                );
                            }
                        });
                    controls_hovered |= lang_resp.response.hovered();
                    if chosen_lang != self.settings.language {
                        self.set_language(chosen_lang);
                    }

                    let theme_btn_label = match self.settings.theme {
                        ThemeMode::Dark => format!("☀ {}", s.light_mode),
                        ThemeMode::Light => format!("🌙 {}", s.dark_mode),
                    };
                    let theme_resp = ui.button(theme_btn_label);
                    controls_hovered |= theme_resp.hovered();
                    if theme_resp.clicked() {
                        self.toggle_theme(ctx);
                    }
                });
            });
            ui.add_space(6.0);

            let header_rect = row_resp.response.rect.expand2(egui::vec2(8.0, 6.0));
            let ptr_in_header = ctx.input(|i| {
                i.pointer
                    .hover_pos()
                    .is_some_and(|p| header_rect.contains(p))
            });
            let drag_start =
                ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
            let dbl_click =
                ctx.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary));
            if !controls_hovered
                && resize_dir.is_none()
                && !self.show_exit_confirm
                && !popup_open
                && ptr_in_header
                && !ctx.is_using_pointer()
            {
                if dbl_click {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!is_fullscreen));
                } else if drag_start && !is_fullscreen {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
            }
        });

        // Update / missing engine banner
        let show_banner = !self.banner_dismissed
            && self.engine_status.as_ref().is_some_and(|st| {
                !st.installed
                    || !st.ffmpeg_installed
                    || !st.js_runtime_installed
                    || st.update_available
                    || st.ffmpeg_update_available
                    || st.js_runtime_update_available
            });

        if show_banner {
            if let Some(st) = self.engine_status.clone() {
                egui::TopBottomPanel::top("update_banner").show(ctx, |ui| {
                    egui::Frame::none()
                        .fill(egui::Color32::from_rgb(120, 53, 15))
                        .inner_margin(egui::Margin::symmetric(12.0, 8.0))
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                let msg = if !st.installed && !st.ffmpeg_installed {
                                    s.banner_missing_both.to_string()
                                } else if !st.installed {
                                    s.banner_missing_ytdlp.to_string()
                                } else if !st.ffmpeg_installed {
                                    s.banner_missing_ffmpeg.to_string()
                                } else if !st.js_runtime_installed {
                                    s.banner_missing_deno.to_string()
                                } else if st.update_available {
                                    format!(
                                        "{} {} -> {}",
                                        s.banner_update_ytdlp,
                                        st.current_version.as_deref().unwrap_or("?"),
                                        st.latest_version.as_deref().unwrap_or("?")
                                    )
                                } else if st.ffmpeg_update_available {
                                    format!(
                                        "{} {} -> {}",
                                        s.banner_update_ffmpeg,
                                        st.ffmpeg_version.as_deref().unwrap_or("?"),
                                        st.ffmpeg_latest_version.as_deref().unwrap_or("?")
                                    )
                                } else {
                                    format!(
                                        "{} {} -> {}",
                                        s.banner_update_deno,
                                        st.js_runtime_version.as_deref().unwrap_or("?"),
                                        st.js_runtime_latest_version.as_deref().unwrap_or("?")
                                    )
                                };
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(msg).color(egui::Color32::WHITE),
                                    )
                                    .wrap(),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.button("✕").clicked() {
                                            self.banner_dismissed = true;
                                        }
                                        let is_busy = self.updating_ytdlp
                                            || self.updating_ffmpeg
                                            || self.updating_deno;
                                        let cta = if is_busy {
                                            s.banner_downloading
                                        } else if !st.installed
                                            || !st.ffmpeg_installed
                                            || !st.js_runtime_installed
                                        {
                                            s.banner_install_cta
                                        } else {
                                            s.banner_update_cta
                                        };
                                        if ui
                                            .add_enabled(
                                                !is_busy,
                                                egui::Button::new(
                                                    egui::RichText::new(cta)
                                                        .color(egui::Color32::WHITE),
                                                )
                                                .fill(egui::Color32::from_rgb(217, 119, 6)),
                                            )
                                            .clicked()
                                        {
                                            self.update_all_engines(ctx.clone());
                                        }
                                    },
                                );
                            });
                        });
                });
            }
        }

        if let Some(err) = self.engine_error.clone() {
            egui::TopBottomPanel::top("engine_err_banner").show(ctx, |ui| {
                egui::Frame::none()
                    .fill(egui::Color32::from_rgb(127, 29, 29))
                    .inner_margin(egui::Margin::symmetric(12.0, 6.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(err).color(egui::Color32::WHITE).small());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.button("✕").clicked() {
                                    self.engine_error = None;
                                }
                            });
                        });
                    });
            });
        }

        // Main panel
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(8.0);
                match self.active_tab {
                    ActiveTab::Download => self.render_download_studio(ui, ctx),
                    ActiveTab::Search => self.render_search(ui, ctx),
                    ActiveTab::Manager => self.render_manager(ui, ctx),
                    ActiveTab::Settings => self.render_settings(ui, ctx),
                }
                ui.add_space(16.0);
            });
        });

        // Subtle border around borderless window when not in fullscreen
        if !is_fullscreen {
            let border_color = if self.settings.theme == ThemeMode::Dark {
                egui::Color32::from_rgb(51, 65, 85)
            } else {
                egui::Color32::from_rgb(203, 213, 225)
            };
            ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("borderless_window_outline"),
            ))
            .rect_stroke(
                ctx.screen_rect().shrink(0.5),
                0.0,
                egui::Stroke::new(1.0_f32, border_color),
            );
        }

        // Exit Confirmation Modal (Esc or Close button)
        if self.show_exit_confirm {
            egui::Area::new(egui::Id::new("exit_confirm_backdrop"))
                .fixed_pos(egui::Pos2::ZERO)
                .order(egui::Order::Middle)
                .show(ctx, |ui| {
                    let screen = ctx.screen_rect();
                    let resp = ui.allocate_rect(screen, egui::Sense::click());
                    ui.painter().rect_filled(
                        screen,
                        0.0,
                        egui::Color32::from_rgba_unmultiplied(0, 0, 0, 150),
                    );
                    if resp.clicked() {
                        self.show_exit_confirm = false;
                    }
                });

            let modal_w = (430.0 * scale.max(1.0)).min((ctx.screen_rect().width() - 32.0).max(320.0));
            egui::Window::new(s.exit_confirm_title)
                .id(egui::Id::new("exit_confirm_modal"))
                .order(egui::Order::Foreground)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .fixed_size([modal_w, 0.0])
                .show(ctx, |ui| {
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(s.exit_confirm_msg).strong());
                    if active_count > 0 {
                        ui.add_space(8.0);
                        egui::Frame::none()
                            .fill(egui::Color32::from_rgb(120, 53, 15))
                            .rounding(6.0)
                            .inner_margin(8.0)
                            .show(ui, |ui| {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "⚠ {} ({})",
                                        s.exit_confirm_active_warn, active_count
                                    ))
                                    .small()
                                    .color(egui::Color32::WHITE),
                                );
                            });
                    }
                    ui.add_space(16.0);
                    let btn_h = (32.0 * scale.max(1.0)).round();
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), btn_h),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new(s.exit_confirm_yes)
                                            .strong()
                                            .color(egui::Color32::WHITE),
                                    )
                                    .fill(egui::Color32::from_rgb(220, 38, 38))
                                    .min_size(egui::vec2((105.0 * scale).round(), btn_h)),
                                )
                                .clicked()
                            {
                                self.rt.block_on(downloader::cancel_all_active(&self.manager));
                                self.confirmed_exit = true;
                                self.show_exit_confirm = false;
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }

                            if ui
                                .add(
                                    egui::Button::new(s.exit_confirm_no)
                                        .min_size(egui::vec2((90.0 * scale).round(), btn_h)),
                                )
                                .clicked()
                            {
                                self.show_exit_confirm = false;
                            }

                            if is_fullscreen
                                && ui
                                    .add(
                                        egui::Button::new(s.exit_confirm_leave_fs)
                                            .min_size(egui::vec2((130.0 * scale).round(), btn_h)),
                                    )
                                    .clicked()
                            {
                                self.show_exit_confirm = false;
                                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                            }
                        },
                    );
                    ui.add_space(4.0);
                });
        }
    }
}

impl VideoDownloaderApp {
    fn render_download_studio(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let s = i18n::strings(self.settings.language);
        let scale = self.font_scale();
        let max_w = ui.available_width().min(960.0);

        ui.vertical(|ui| {
            ui.set_max_width(max_w);

            ui.label(egui::RichText::new(s.studio_heading).size(22.0 * scale).strong());
            ui.add_space(2.0);
            ui.label(egui::RichText::new(s.studio_subtitle).weak());
            ui.add_space(12.0);

            egui::Frame::group(ui.style())
                .inner_margin(16.0)
                .rounding(10.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());

                    let has_text = !self.url.trim().is_empty();
                    let is_url_input = looks_like_url(&self.url);
                    let is_search_input = has_text && !is_url_input;

                    // URL / Search Input + Action Buttons (explicit row height prevents vertical center bloat)
                    let row_h = (34.0 * scale.max(1.0)).round();
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), row_h),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if is_search_input {
                                let srch_lbl = if self.main_searching {
                                    "..."
                                } else {
                                    s.studio_search_query_cta
                                };
                                if ui
                                    .add_enabled(
                                        !self.main_searching,
                                        egui::Button::new(
                                            egui::RichText::new(format!("🔍 {}", srch_lbl))
                                                .strong()
                                                .color(egui::Color32::WHITE),
                                        )
                                        .fill(egui::Color32::from_rgb(13, 148, 136))
                                        .min_size(egui::vec2((115.0 * scale).round(), row_h)),
                                    )
                                    .clicked()
                                {
                                    self.run_main_search(ctx.clone());
                                }
                            } else if ui
                                .add_enabled(
                                    has_text,
                                    egui::Button::new(
                                        egui::RichText::new(s.studio_download)
                                            .strong()
                                            .color(egui::Color32::WHITE),
                                    )
                                    .fill(egui::Color32::from_rgb(37, 99, 235))
                                    .min_size(egui::vec2((105.0 * scale).round(), row_h)),
                                )
                                .clicked()
                            {
                                self.start_download(None, ctx.clone());
                            }

                            let inspect_label = if self.inspecting {
                                "..."
                            } else {
                                s.studio_inspect
                            };
                            if ui
                                .add_enabled(
                                    !self.inspecting && is_url_input,
                                    egui::Button::new(inspect_label)
                                        .min_size(egui::vec2((86.0 * scale).round(), row_h)),
                                )
                                .clicked()
                            {
                                let target = self.url.clone();
                                self.inspect_url(target, ctx.clone());
                            }

                            if ui
                                .add(
                                    egui::Button::new(s.studio_paste)
                                        .min_size(egui::vec2((76.0 * scale).round(), row_h)),
                                )
                                .clicked()
                            {
                                self.paste_from_clipboard(ctx.clone());
                            }

                            let edit_w = ui.available_width().max(180.0);
                            ui.allocate_ui_with_layout(
                                egui::vec2(edit_w, row_h),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    let resp = ui.add_sized(
                                        [edit_w, row_h],
                                        egui::TextEdit::singleline(&mut self.url)
                                            .hint_text(s.studio_url_hint),
                                    );
                                    if resp.changed() {
                                        self.studio_error = None;
                                        self.studio_is_auth_error = false;
                                        if self.url.trim().is_empty() {
                                            self.media_info = None;
                                            self.download_playlist = false;
                                            self.video_format =
                                                self.settings.default_video_format.clone();
                                            self.video_quality =
                                                self.settings.default_video_quality.clone();
                                            self.audio_format =
                                                self.settings.default_audio_format.clone();
                                            self.audio_quality =
                                                self.settings.default_audio_quality.clone();
                                        } else {
                                            self.download_playlist =
                                                detect_playlist_intent(&self.url);
                                            if is_yt_music_url(&self.url)
                                                || is_bandcamp_url(&self.url)
                                            {
                                                self.mode = "audio".to_string();
                                            }
                                        }
                                    }
                                    if resp.lost_focus()
                                        && ui.input(|i| i.key_pressed(egui::Key::Enter))
                                    {
                                        if looks_like_url(&self.url) && !self.inspecting {
                                            let target = self.url.clone();
                                            self.inspect_url(target, ctx.clone());
                                        } else if !self.url.trim().is_empty()
                                            && !self.main_searching
                                        {
                                            self.run_main_search(ctx.clone());
                                        }
                                    }
                                },
                            );
                        },
                    );

                    ui.add_space(8.0);

                    // Music Search shortcut + Supported platforms strip
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new(format!("🔍 {}", s.studio_search_shortcut))
                                        .strong()
                                        .color(egui::Color32::WHITE)
                                        .small(),
                                )
                                .fill(egui::Color32::from_rgb(13, 148, 136))
                                .min_size(egui::vec2(185.0, 28.0)),
                            )
                            .clicked()
                        {
                            self.active_tab = ActiveTab::Search;
                        }
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new(s.studio_supported_sites).small().weak());
                    });

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);

                    // Media Type, Format, Quality controls
                    ui.columns(3, |cols| {
                        cols[0].label(egui::RichText::new(s.studio_media_type).small().weak());
                        cols[0].add_space(4.0);
                        cols[0].horizontal(|ui| {
                            if ui
                                .selectable_label(self.mode == "video", format!("🎬 {}", s.studio_video))
                                .clicked()
                            {
                                self.mode = "video".to_string();
                            }
                            if ui
                                .selectable_label(self.mode == "audio", format!("🎵 {}", s.studio_audio))
                                .clicked()
                            {
                                self.mode = "audio".to_string();
                            }
                        });

                        cols[1].label(egui::RichText::new(s.studio_format).small().weak());
                        cols[1].add_space(4.0);
                        if self.mode == "video" {
                            let video_formats: Vec<String> = self
                                .media_info
                                .as_ref()
                                .filter(|m| !m.available_video_formats.is_empty())
                                .map(|m| m.available_video_formats.clone())
                                .unwrap_or_else(|| {
                                    vec!["mp4".to_string(), "mkv".to_string(), "webm".to_string()]
                                });
                            egui::ComboBox::from_id_salt("studio_vfmt")
                                .width(cols[1].available_width() - 8.0)
                                .selected_text(self.video_format.to_uppercase())
                                .show_ui(&mut cols[1], |ui| {
                                    for fmt in &video_formats {
                                        ui.selectable_value(
                                            &mut self.video_format,
                                            fmt.clone(),
                                            fmt.to_uppercase(),
                                        );
                                    }
                                });
                        } else {
                            let audio_formats: Vec<String> = self
                                .media_info
                                .as_ref()
                                .filter(|m| !m.available_audio_formats.is_empty())
                                .map(|m| m.available_audio_formats.clone())
                                .unwrap_or_else(|| {
                                    vec![
                                        "mp3".to_string(),
                                        "flac".to_string(),
                                        "m4a".to_string(),
                                        "opus".to_string(),
                                        "wav".to_string(),
                                    ]
                                });
                            egui::ComboBox::from_id_salt("studio_afmt")
                                .width(cols[1].available_width() - 8.0)
                                .selected_text(self.audio_format.to_uppercase())
                                .show_ui(&mut cols[1], |ui| {
                                    for fmt in &audio_formats {
                                        ui.selectable_value(
                                            &mut self.audio_format,
                                            fmt.clone(),
                                            fmt.to_uppercase(),
                                        );
                                    }
                                });
                        }

                        cols[2].label(egui::RichText::new(s.studio_quality).small().weak());
                        cols[2].add_space(4.0);
                        if self.mode == "video" {
                            let video_qualities: Vec<String> = self
                                .media_info
                                .as_ref()
                                .filter(|m| !m.available_heights.is_empty())
                                .map(|m| m.available_heights.iter().map(|h| h.to_string()).collect())
                                .unwrap_or_else(|| {
                                    vec![
                                        "best".to_string(),
                                        "2160".to_string(),
                                        "1440".to_string(),
                                        "1080".to_string(),
                                        "720".to_string(),
                                        "480".to_string(),
                                    ]
                                });
                            let q_label = video_quality_label(&self.video_quality, s);
                            egui::ComboBox::from_id_salt("studio_vq")
                                .width(cols[2].available_width() - 8.0)
                                .selected_text(q_label)
                                .show_ui(&mut cols[2], |ui| {
                                    for q in &video_qualities {
                                        ui.selectable_value(
                                            &mut self.video_quality,
                                            q.clone(),
                                            video_quality_label(q, s),
                                        );
                                    }
                                });
                        } else if self.audio_format == "flac" || self.audio_format == "wav" {
                            cols[2].add_enabled(
                                false,
                                egui::Button::new(s.studio_lossless)
                                    .min_size(egui::vec2(cols[2].available_width() - 8.0, 22.0)),
                            );
                        } else {
                            let audio_qualities: Vec<String> = self
                                .media_info
                                .as_ref()
                                .filter(|m| !m.available_audio_qualities.is_empty())
                                .map(|m| m.available_audio_qualities.clone())
                                .unwrap_or_else(|| {
                                    vec![
                                        "320".to_string(),
                                        "256".to_string(),
                                        "192".to_string(),
                                        "128".to_string(),
                                    ]
                                });
                            let q_label = audio_quality_label(&self.audio_quality, s);
                            egui::ComboBox::from_id_salt("studio_aq")
                                .width(cols[2].available_width() - 8.0)
                                .selected_text(q_label)
                                .show_ui(&mut cols[2], |ui| {
                                    for q in &audio_qualities {
                                        ui.selectable_value(
                                            &mut self.audio_quality,
                                            q.clone(),
                                            audio_quality_label(q, s),
                                        );
                                    }
                                });
                        }
                    });

                    ui.add_space(10.0);

                    // Options checkboxes
                    ui.horizontal_wrapped(|ui| {
                        ui.checkbox(&mut self.download_playlist, s.studio_opt_playlist);
                        if self.mode == "audio" {
                            ui.add_space(8.0);
                            ui.checkbox(&mut self.embed_thumbnail, s.studio_opt_thumb);
                        }
                        ui.add_space(8.0);
                        ui.checkbox(&mut self.embed_metadata, s.studio_opt_meta);
                        if self.mode == "video" {
                            ui.add_space(8.0);
                            ui.checkbox(&mut self.subtitles, s.studio_opt_subs);
                        }
                    });

                    if self.inspecting {
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new(format!("⏳ {}", s.studio_inspect_loading))
                                .small()
                                .color(egui::Color32::from_rgb(59, 130, 246)),
                        );
                    }

                    if let Some(notice) = &self.clipboard_notice {
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new(notice)
                                .small()
                                .color(egui::Color32::from_rgb(59, 130, 246)),
                        );
                    }

                    if let Some(err) = &self.studio_error {
                        ui.add_space(8.0);
                        egui::Frame::none()
                            .fill(egui::Color32::from_rgb(127, 29, 29))
                            .rounding(6.0)
                            .inner_margin(10.0)
                            .show(ui, |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(
                                        egui::RichText::new(err)
                                            .color(egui::Color32::WHITE)
                                            .small(),
                                    );
                                    if self.studio_is_auth_error {
                                        if ui.button(s.studio_setup_auth).clicked() {
                                            self.active_tab = ActiveTab::Settings;
                                        }
                                    }
                                });
                            });
                    }
                });

            // Inspected Media Card
            if let Some(info) = self.media_info.clone() {
                ui.add_space(16.0);
                egui::Frame::group(ui.style())
                    .inner_margin(16.0)
                    .rounding(10.0)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal_top(|ui| {
                            if let Some(thumb_url) = &info.thumbnail {
                                if let Some(ThumbnailState::Ready(tex)) =
                                    self.thumbnails.get(thumb_url)
                                {
                                    ui.add(
                                        egui::Image::new(tex)
                                            .max_size(egui::vec2(148.0, 148.0))
                                            .rounding(8.0),
                                    );
                                    ui.add_space(12.0);
                                }
                            }

                            ui.vertical(|ui| {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(
                                        egui::RichText::new(&info.extractor)
                                            .small()
                                            .color(egui::Color32::from_rgb(59, 130, 246)),
                                    );
                                    ui.label(egui::RichText::new("·").weak().small());
                                    ui.label(egui::RichText::new(&info.uploader).weak().small());
                                    if let Some(dur) = info.duration {
                                        ui.label(egui::RichText::new("·").weak().small());
                                        ui.label(
                                            egui::RichText::new(format_duration(Some(dur)))
                                                .small()
                                                .monospace()
                                                .weak(),
                                        );
                                    }
                                    if info.has_flac {
                                        ui.label(egui::RichText::new("·").weak().small());
                                        ui.label(
                                            egui::RichText::new(format!(
                                                "★ {}",
                                                s.studio_flac_detected
                                            ))
                                            .small()
                                            .color(egui::Color32::from_rgb(16, 185, 129)),
                                        );
                                    }
                                });

                                ui.add_space(4.0);
                                ui.label(egui::RichText::new(&info.title).size(16.0 * scale).strong());
                                ui.add_space(4.0);

                                if info.is_playlist {
                                    ui.label(egui::RichText::new(s.studio_playlist_desc).small().weak());
                                    if let Some(folder) = build_album_folder_name(
                                        info.album_artist.as_deref().unwrap_or(&info.uploader),
                                        info.album.as_deref().unwrap_or(&info.title),
                                    ) {
                                        ui.add_space(2.0);
                                        ui.label(
                                            egui::RichText::new(format!(
                                                "{}: {}",
                                                s.studio_album_folder_label, folder
                                            ))
                                            .small()
                                            .monospace()
                                            .color(egui::Color32::from_rgb(96, 165, 250)),
                                        );
                                    }
                                } else if !info.available_heights.is_empty() {
                                    let res_list: Vec<String> = info
                                        .available_heights
                                        .iter()
                                        .take(6)
                                        .map(|h| format!("{}p", h))
                                        .collect();
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "{} {}",
                                            s.studio_available_res,
                                            res_list.join(" · ")
                                        ))
                                        .small()
                                        .monospace()
                                        .weak(),
                                    );
                                }

                                ui.add_space(10.0);
                                let can_download_single = info.is_playlist
                                    && (has_single_video_in_url(&info.url)
                                        || has_single_video_in_url(&self.url));
                                let full_playlist_text = format!(
                                    "{} ({} {})",
                                    s.studio_download_full_playlist,
                                    info.playlist_count.unwrap_or(0),
                                    s.studio_tracks_unit
                                );

                                if can_download_single {
                                    let single_fill = if !self.download_playlist {
                                        egui::Color32::from_rgb(37, 99, 235)
                                    } else {
                                        egui::Color32::from_rgb(51, 65, 85)
                                    };
                                    let playlist_fill = if self.download_playlist {
                                        egui::Color32::from_rgb(37, 99, 235)
                                    } else {
                                        egui::Color32::from_rgb(51, 65, 85)
                                    };
                                    ui.horizontal_wrapped(|ui| {
                                        if ui
                                            .add(
                                                egui::Button::new(
                                                    egui::RichText::new(s.studio_download_single)
                                                        .strong()
                                                        .color(egui::Color32::WHITE),
                                                )
                                                .fill(single_fill)
                                                .min_size(egui::vec2(190.0, 32.0)),
                                            )
                                            .clicked()
                                        {
                                            self.start_download(Some(false), ctx.clone());
                                        }
                                        if ui
                                            .add(
                                                egui::Button::new(
                                                    egui::RichText::new(full_playlist_text)
                                                        .strong()
                                                        .color(egui::Color32::WHITE),
                                                )
                                                .fill(playlist_fill)
                                                .min_size(egui::vec2(190.0, 32.0)),
                                            )
                                            .clicked()
                                        {
                                            self.start_download(Some(true), ctx.clone());
                                        }
                                    });
                                } else {
                                    let cta_text = if info.is_playlist {
                                        full_playlist_text
                                    } else {
                                        s.studio_download.to_string()
                                    };
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                egui::RichText::new(cta_text)
                                                    .strong()
                                                    .color(egui::Color32::WHITE),
                                            )
                                            .fill(egui::Color32::from_rgb(37, 99, 235))
                                            .min_size(egui::vec2(200.0, 32.0)),
                                        )
                                        .clicked()
                                    {
                                        self.start_download(
                                            Some(info.is_playlist || self.download_playlist),
                                            ctx.clone(),
                                        );
                                    }
                                }
                            });
                        });

                        if info.is_playlist && !info.entries.is_empty() {
                            ui.add_space(10.0);
                            ui.separator();
                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} ({} {})",
                                    s.studio_tracklist,
                                    info.entries.len(),
                                    s.studio_tracks_unit
                                ))
                                .small()
                                .strong(),
                            );
                            ui.add_space(4.0);
                            egui::ScrollArea::vertical()
                                .id_salt("tracklist_scroll")
                                .max_height(200.0)
                                .show(ui, |ui| {
                                    for entry in &info.entries {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                egui::RichText::new(format!("{:02}.", entry.index))
                                                    .monospace()
                                                    .small()
                                                    .weak(),
                                            );
                                            ui.label(egui::RichText::new(&entry.title).small());
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    ui.label(
                                                        egui::RichText::new(format_duration(
                                                            entry.duration,
                                                        ))
                                                        .monospace()
                                                        .small()
                                                        .weak(),
                                                    );
                                                },
                                            );
                                        });
                                    }
                                });
                        }
                    });
            }

            self.render_main_search_section(ui, ctx);

            // Recent Downloads
            ui.add_space(20.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(s.studio_recent_title).size(16.0 * scale).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.link(s.studio_open_manager).clicked() {
                        self.active_tab = ActiveTab::Manager;
                    }
                });
            });
            ui.add_space(8.0);

            if self.downloads.is_empty() {
                egui::Frame::group(ui.style())
                    .inner_margin(24.0)
                    .rounding(10.0)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.vertical_centered(|ui| {
                            ui.label(egui::RichText::new(s.studio_empty_title).strong());
                            ui.add_space(4.0);
                            ui.label(egui::RichText::new(s.studio_empty_sub).small().weak());
                        });
                    });
            } else {
                egui::Frame::group(ui.style())
                    .inner_margin(12.0)
                    .rounding(10.0)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        for (idx, item) in self.downloads.iter().take(5).enumerate() {
                            if idx > 0 {
                                ui.separator();
                            }
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.set_max_width(
                                        (ui.available_width() - 210.0 * scale.max(1.0)).max(160.0),
                                    );
                                    ui.label(egui::RichText::new(&item.title).strong());
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(
                                            egui::RichText::new(&item.extractor).small().weak(),
                                        );
                                        ui.label(egui::RichText::new("·").small().weak());
                                        ui.label(
                                            egui::RichText::new(item.format.to_uppercase())
                                                .small()
                                                .monospace()
                                                .weak(),
                                        );
                                        if let Some(pl) = &item.playlist_progress {
                                            ui.label(egui::RichText::new("·").small().weak());
                                            ui.label(
                                                egui::RichText::new(pl)
                                                    .small()
                                                    .color(egui::Color32::from_rgb(59, 130, 246)),
                                            );
                                        }
                                    });
                                });

                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if item.status == "completed" {
                                            ui.label(
                                                egui::RichText::new(format!(
                                                    "✓ {}",
                                                    s.status_completed
                                                ))
                                                .small()
                                                .color(egui::Color32::from_rgb(16, 185, 129)),
                                            );
                                        } else if item.status == "failed" || item.status == "cancelled" {
                                            let lbl = if item.status == "cancelled" {
                                                s.status_cancelled
                                            } else {
                                                s.status_failed
                                            };
                                            ui.label(
                                                egui::RichText::new(format!("✕ {}", lbl))
                                                    .small()
                                                    .color(egui::Color32::from_rgb(239, 68, 68)),
                                            );
                                        } else {
                                            ui.vertical(|ui| {
                                                ui.set_width(165.0);
                                                ui.horizontal(|ui| {
                                                    ui.label(
                                                        egui::RichText::new(format!(
                                                            "{:.1}%",
                                                            item.progress
                                                        ))
                                                        .small()
                                                        .monospace(),
                                                    );
                                                    ui.with_layout(
                                                        egui::Layout::right_to_left(
                                                            egui::Align::Center,
                                                        ),
                                                        |ui| {
                                                            ui.label(
                                                                egui::RichText::new(&item.speed)
                                                                    .small()
                                                                    .monospace()
                                                                    .weak(),
                                                            );
                                                        },
                                                    );
                                                });
                                                ui.add_sized(
                                                    [165.0, 8.0],
                                                    egui::ProgressBar::new(
                                                        (item.progress / 100.0) as f32,
                                                    ),
                                                );
                                            });
                                        }
                                    },
                                );
                            });
                        }
                    });
            }
        });
    }

    fn render_main_search_section(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let show = self.main_searching
            || self.main_search_error.is_some()
            || !self.main_search_results.is_empty()
            || !self.main_search_last_query.is_empty()
            || self.main_search_queued_notice.is_some();
        if !show {
            return;
        }

        let s = i18n::strings(self.settings.language);
        let scale = self.font_scale();
        let is_dark = self.settings.theme == ThemeMode::Dark;

        let mut close_search = false;
        let mut download_item: Option<MusicSearchResult> = None;
        let mut inspect_item: Option<MusicSearchResult> = None;
        let mut download_selected: Option<Vec<MusicSearchResult>> = None;
        let mut open_manager = false;
        let mut thumbs_to_fetch: Vec<String> = Vec::new();

        ui.add_space(16.0);
        egui::Frame::group(ui.style())
            .inner_margin(14.0)
            .rounding(10.0)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());

                ui.horizontal(|ui| {
                    let heading = if self.main_search_last_query.is_empty() {
                        s.studio_search_results_title.to_string()
                    } else {
                        format!(
                            "{}: \"{}\"",
                            s.studio_search_results_title, self.main_search_last_query
                        )
                    };
                    ui.label(egui::RichText::new(heading).size(16.0 * scale).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button(egui::RichText::new(format!("✕ {}", s.studio_clear_search)).small())
                            .clicked()
                        {
                            close_search = true;
                        }
                    });
                });

                if self.main_searching {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(format!("⏳ {}", s.studio_searching_all))
                            .small()
                            .color(egui::Color32::from_rgb(59, 130, 246)),
                    );
                }

                if let Some(err) = &self.main_search_error {
                    ui.add_space(8.0);
                    egui::Frame::none()
                        .fill(egui::Color32::from_rgb(127, 29, 29))
                        .rounding(6.0)
                        .inner_margin(10.0)
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new(err).color(egui::Color32::WHITE).small());
                        });
                }

                if let Some(notice) = &self.main_search_queued_notice {
                    ui.add_space(8.0);
                    egui::Frame::none()
                        .fill(egui::Color32::from_rgb(6, 78, 59))
                        .rounding(6.0)
                        .inner_margin(10.0)
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(
                                    egui::RichText::new(notice)
                                        .color(egui::Color32::WHITE)
                                        .small(),
                                );
                                if ui
                                    .button(egui::RichText::new(s.studio_open_manager).small())
                                    .clicked()
                                {
                                    open_manager = true;
                                }
                            });
                        });
                }

                if !self.main_search_results.is_empty() {
                    let mut total_videos = 0usize;
                    let mut total_albums = 0usize;
                    let mut total_songs = 0usize;
                    for r in &self.main_search_results {
                        match r.kind {
                            MusicSearchItemKind::Video => total_videos += 1,
                            MusicSearchItemKind::Album => total_albums += 1,
                            MusicSearchItemKind::Song => total_songs += 1,
                        }
                    }

                    ui.add_space(8.0);
                    // Kind filter pills: All / Video / Album / Song
                    ui.horizontal_wrapped(|ui| {
                        for (tf, lbl) in [
                            (
                                SearchTypeFilter::All,
                                format!("{} ({})", s.search_filter_all, self.main_search_results.len()),
                            ),
                            (
                                SearchTypeFilter::Videos,
                                format!("🎬 {} ({})", s.search_filter_videos, total_videos),
                            ),
                            (
                                SearchTypeFilter::Albums,
                                format!("💿 {} ({})", s.search_filter_albums, total_albums),
                            ),
                            (
                                SearchTypeFilter::Songs,
                                format!("🎵 {} ({})", s.search_filter_songs, total_songs),
                            ),
                        ] {
                            if ui
                                .selectable_label(self.main_search_type_filter == tf, lbl)
                                .clicked()
                            {
                                self.main_search_type_filter = tf;
                            }
                        }

                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(8.0);

                        if ui
                            .selectable_label(
                                self.main_search_source_filter.is_none(),
                                s.search_filter_all,
                            )
                            .clicked()
                        {
                            self.main_search_source_filter = None;
                        }
                        for src in [
                            MusicSearchSource::Youtube,
                            MusicSearchSource::YoutubeMusic,
                            MusicSearchSource::Bandcamp,
                            MusicSearchSource::Dailymotion,
                            MusicSearchSource::Videa,
                            MusicSearchSource::OkRu,
                        ] {
                            let count = self
                                .main_search_results
                                .iter()
                                .filter(|r| r.source == src)
                                .count();
                            if count > 0
                                && ui
                                    .selectable_label(
                                        self.main_search_source_filter == Some(src),
                                        format!("{} ({})", src.label(), count),
                                    )
                                    .clicked()
                            {
                                self.main_search_source_filter = Some(src);
                            }
                        }
                    });

                    let filtered: Vec<MusicSearchResult> = self
                        .main_search_results
                        .iter()
                        .filter(|r| {
                            let type_ok = match self.main_search_type_filter {
                                SearchTypeFilter::All => true,
                                SearchTypeFilter::Videos => r.kind == MusicSearchItemKind::Video,
                                SearchTypeFilter::Albums => r.kind == MusicSearchItemKind::Album,
                                SearchTypeFilter::Songs => r.kind == MusicSearchItemKind::Song,
                            };
                            let src_ok = self
                                .main_search_source_filter
                                .map_or(true, |src| r.source == src);
                            type_ok && src_ok
                        })
                        .cloned()
                        .collect();

                    ui.add_space(8.0);
                    // Multi-select action bar
                    let selected_in_filtered: Vec<MusicSearchResult> = filtered
                        .iter()
                        .filter(|r| self.main_search_selected_ids.contains(&r.id))
                        .cloned()
                        .collect();
                    let sel_count = selected_in_filtered.len();
                    ui.horizontal_wrapped(|ui| {
                        let dl_sel_lbl = format!(
                            "⬇ {} ({})",
                            s.search_download_selected,
                            self.main_search_selected_ids.len()
                        );
                        if ui
                            .add_enabled(
                                !self.main_search_selected_ids.is_empty(),
                                egui::Button::new(
                                    egui::RichText::new(dl_sel_lbl)
                                        .small()
                                        .strong()
                                        .color(egui::Color32::WHITE),
                                )
                                .fill(egui::Color32::from_rgb(37, 99, 235)),
                            )
                            .clicked()
                        {
                            let to_dl: Vec<MusicSearchResult> = self
                                .main_search_results
                                .iter()
                                .filter(|r| self.main_search_selected_ids.contains(&r.id))
                                .cloned()
                                .collect();
                            download_selected = Some(to_dl);
                        }
                        if ui
                            .add_enabled(
                                !filtered.is_empty() && sel_count < filtered.len(),
                                egui::Button::new(egui::RichText::new(s.search_select_all).small()),
                            )
                            .clicked()
                        {
                            for r in &filtered {
                                self.main_search_selected_ids.insert(r.id.clone());
                            }
                        }
                        if ui
                            .add_enabled(
                                !self.main_search_selected_ids.is_empty(),
                                egui::Button::new(
                                    egui::RichText::new(s.search_clear_selection).small(),
                                ),
                            )
                            .clicked()
                        {
                            self.main_search_selected_ids.clear();
                        }
                    });

                    ui.add_space(8.0);
                    if filtered.is_empty() {
                        ui.vertical_centered(|ui| {
                            ui.add_space(8.0);
                            ui.label(egui::RichText::new(s.search_no_results).weak());
                            ui.add_space(8.0);
                        });
                    } else {
                        render_search_results_scroll(
                            ui,
                            "main_multi_search_results",
                            380.0,
                            &filtered,
                            &mut self.main_search_selected_ids,
                            &self.thumbnails,
                            is_dark,
                            scale,
                            s,
                            &mut thumbs_to_fetch,
                            &mut download_item,
                            &mut inspect_item,
                        );
                    }
                } else if !self.main_searching && !self.main_search_last_query.is_empty() {
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new(s.search_no_results).weak());
                }
            });

        for turl in thumbs_to_fetch {
            self.request_thumbnail(&turl, ctx);
        }
        if let Some(items) = download_selected {
            self.main_search_selected_ids.clear();
            self.download_selected_search_results(items, true, ctx.clone());
        }
        if let Some(item) = download_item {
            self.download_search_result(&item, true, ctx.clone());
        }
        if let Some(item) = inspect_item {
            self.inspect_search_result(&item, ctx.clone());
        }
        if open_manager {
            self.active_tab = ActiveTab::Manager;
        }
        if close_search {
            self.main_search_results.clear();
            self.main_search_last_query.clear();
            self.main_search_error = None;
            self.main_search_selected_ids.clear();
            self.main_search_queued_notice = None;
        }
    }

    fn render_search(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let s = i18n::strings(self.settings.language);
        let scale = self.font_scale();
        let max_w = ui.available_width().min(1040.0);
        let is_dark = self.settings.theme == ThemeMode::Dark;

        let mut trigger_search = false;
        let mut download_item: Option<MusicSearchResult> = None;
        let mut inspect_item: Option<MusicSearchResult> = None;
        let mut download_selected: Option<Vec<MusicSearchResult>> = None;
        let mut open_manager = false;
        let mut thumbs_to_fetch: Vec<String> = Vec::new();

        ui.vertical(|ui| {
            ui.set_max_width(max_w);

            ui.label(egui::RichText::new(s.search_title).size(22.0 * scale).strong());
            ui.add_space(2.0);
            ui.label(egui::RichText::new(s.search_subtitle).weak());
            ui.add_space(12.0);

            // Search input + sources + audio output options
            egui::Frame::group(ui.style())
                .inner_margin(16.0)
                .rounding(10.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());

                    let search_row_h = (34.0 * scale.max(1.0)).round();
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), search_row_h),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            let btn_text = if self.searching {
                                "..."
                            } else {
                                s.search_button
                            };
                            if ui
                                .add_enabled(
                                    !self.searching && !self.search_query.trim().is_empty(),
                                    egui::Button::new(
                                        egui::RichText::new(format!("🔍 {}", btn_text))
                                            .strong()
                                            .color(egui::Color32::WHITE),
                                    )
                                    .fill(egui::Color32::from_rgb(13, 148, 136))
                                    .min_size(egui::vec2((120.0 * scale).round(), search_row_h)),
                                )
                                .clicked()
                            {
                                trigger_search = true;
                            }

                            let edit_w = ui.available_width().max(180.0);
                            ui.allocate_ui_with_layout(
                                egui::vec2(edit_w, search_row_h),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    let resp = ui.add_sized(
                                        [edit_w, search_row_h],
                                        egui::TextEdit::singleline(&mut self.search_query)
                                            .hint_text(s.search_hint),
                                    );
                                    if resp.lost_focus()
                                        && ui.input(|i| i.key_pressed(egui::Key::Enter))
                                    {
                                        trigger_search = true;
                                    }
                                },
                            );
                        },
                    );

                    ui.add_space(10.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(egui::RichText::new(s.search_sources_label).small().weak());
                        ui.checkbox(&mut self.search_include_ytm, "YouTube Music");
                        ui.checkbox(&mut self.search_include_bandcamp, "Bandcamp");

                        ui.add_space(12.0);
                        ui.label(egui::RichText::new(format!("{}:", s.studio_format)).small().weak());
                        egui::ComboBox::from_id_salt("search_afmt")
                            .width(78.0)
                            .selected_text(self.audio_format.to_uppercase())
                            .show_ui(ui, |ui| {
                                for fmt in ["mp3", "flac", "m4a", "opus", "wav"] {
                                    ui.selectable_value(
                                        &mut self.audio_format,
                                        fmt.to_string(),
                                        fmt.to_uppercase(),
                                    );
                                }
                            });

                        ui.label(egui::RichText::new(format!("{}:", s.studio_quality)).small().weak());
                        if self.audio_format == "flac" || self.audio_format == "wav" {
                            ui.add_enabled(false, egui::Button::new(s.studio_lossless).small());
                        } else {
                            let q_label = audio_quality_label(&self.audio_quality, s);
                            egui::ComboBox::from_id_salt("search_aq")
                                .width(102.0)
                                .selected_text(q_label)
                                .show_ui(ui, |ui| {
                                    for q in ["320", "256", "192", "128"] {
                                        ui.selectable_value(
                                            &mut self.audio_quality,
                                            q.to_string(),
                                            audio_quality_label(q, s),
                                        );
                                    }
                                });
                        }

                        ui.add_space(8.0);
                        ui.checkbox(&mut self.embed_thumbnail, s.studio_opt_thumb);
                        ui.checkbox(&mut self.embed_metadata, s.studio_opt_meta);
                    });

                    if self.searching {
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new(format!("⏳ {}", s.search_searching))
                                .small()
                                .color(egui::Color32::from_rgb(59, 130, 246)),
                        );
                    }

                    if let Some(err) = &self.search_error {
                        ui.add_space(8.0);
                        egui::Frame::none()
                            .fill(egui::Color32::from_rgb(127, 29, 29))
                            .rounding(6.0)
                            .inner_margin(10.0)
                            .show(ui, |ui| {
                                ui.label(
                                    egui::RichText::new(err)
                                        .color(egui::Color32::WHITE)
                                        .small(),
                                );
                            });
                    }

                    if let Some(notice) = &self.search_queued_notice {
                        ui.add_space(8.0);
                        egui::Frame::none()
                            .fill(egui::Color32::from_rgb(6, 78, 59))
                            .rounding(6.0)
                            .inner_margin(10.0)
                            .show(ui, |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(
                                        egui::RichText::new(notice)
                                            .color(egui::Color32::WHITE)
                                            .small(),
                                    );
                                    if ui
                                        .button(
                                            egui::RichText::new(s.studio_open_manager).small(),
                                        )
                                        .clicked()
                                    {
                                        open_manager = true;
                                    }
                                });
                            });
                    }
                });

            ui.add_space(14.0);

            if self.search_results.is_empty() && !self.searching {
                egui::Frame::group(ui.style())
                    .inner_margin(28.0)
                    .rounding(10.0)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.vertical_centered(|ui| {
                            let title = if self.search_last_query.is_empty() {
                                s.search_empty_title
                            } else {
                                s.search_no_results
                            };
                            ui.label(egui::RichText::new(title).strong().size(15.0 * scale));
                            ui.add_space(4.0);
                            ui.label(egui::RichText::new(s.search_empty_sub).small().weak());
                        });
                    });
            } else if !self.search_results.is_empty() {
                let mut total_albums = 0usize;
                let mut total_songs = 0usize;
                for r in &self.search_results {
                    match r.kind {
                        MusicSearchItemKind::Album => total_albums += 1,
                        MusicSearchItemKind::Song => total_songs += 1,
                        MusicSearchItemKind::Video => {}
                    }
                }

                // Filter bar
                ui.horizontal_wrapped(|ui| {
                    for (tf, lbl) in [
                        (
                            SearchTypeFilter::All,
                            format!("{} ({})", s.search_filter_all, self.search_results.len()),
                        ),
                        (
                            SearchTypeFilter::Albums,
                            format!("💿 {} ({})", s.search_filter_albums, total_albums),
                        ),
                        (
                            SearchTypeFilter::Songs,
                            format!("🎵 {} ({})", s.search_filter_songs, total_songs),
                        ),
                    ] {
                        if ui
                            .selectable_label(self.search_type_filter == tf, lbl)
                            .clicked()
                        {
                            self.search_type_filter = tf;
                        }
                    }

                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(12.0);

                    for (sf, lbl) in [
                        (SearchSourceFilter::All, s.search_filter_all),
                        (SearchSourceFilter::YoutubeMusic, "YouTube Music"),
                        (SearchSourceFilter::Bandcamp, "Bandcamp"),
                    ] {
                        if ui
                            .selectable_label(self.search_source_filter == sf, lbl)
                            .clicked()
                        {
                            self.search_source_filter = sf;
                        }
                    }
                });

                let filtered: Vec<MusicSearchResult> = self
                    .search_results
                    .iter()
                    .filter(|r| {
                        let type_ok = match self.search_type_filter {
                            SearchTypeFilter::All | SearchTypeFilter::Videos => true,
                            SearchTypeFilter::Albums => r.kind == MusicSearchItemKind::Album,
                            SearchTypeFilter::Songs => r.kind == MusicSearchItemKind::Song,
                        };
                        let source_ok = match self.search_source_filter {
                            SearchSourceFilter::All => true,
                            SearchSourceFilter::YoutubeMusic => {
                                r.source == MusicSearchSource::YoutubeMusic
                            }
                            SearchSourceFilter::Bandcamp => {
                                r.source == MusicSearchSource::Bandcamp
                            }
                        };
                        type_ok && source_ok
                    })
                    .cloned()
                    .collect();

                ui.add_space(8.0);
                // Multi-select action bar (checkboxes + Ctrl+Click)
                let selected_in_filtered: Vec<MusicSearchResult> = filtered
                    .iter()
                    .filter(|r| self.search_selected_ids.contains(&r.id))
                    .cloned()
                    .collect();
                let sel_count = selected_in_filtered.len();
                ui.horizontal_wrapped(|ui| {
                    let dl_sel_lbl = format!(
                        "⬇ {} ({})",
                        s.search_download_selected,
                        self.search_selected_ids.len()
                    );
                    if ui
                        .add_enabled(
                            !self.search_selected_ids.is_empty(),
                            egui::Button::new(
                                egui::RichText::new(dl_sel_lbl)
                                    .strong()
                                    .color(egui::Color32::WHITE),
                            )
                            .fill(egui::Color32::from_rgb(37, 99, 235)),
                        )
                        .clicked()
                    {
                        let to_dl: Vec<MusicSearchResult> = self
                            .search_results
                            .iter()
                            .filter(|r| self.search_selected_ids.contains(&r.id))
                            .cloned()
                            .collect();
                        download_selected = Some(to_dl);
                    }
                    if ui
                        .add_enabled(
                            !filtered.is_empty() && sel_count < filtered.len(),
                            egui::Button::new(egui::RichText::new(s.search_select_all).small()),
                        )
                        .clicked()
                    {
                        for r in &filtered {
                            self.search_selected_ids.insert(r.id.clone());
                        }
                    }
                    if ui
                        .add_enabled(
                            !self.search_selected_ids.is_empty(),
                            egui::Button::new(egui::RichText::new(s.search_clear_selection).small()),
                        )
                        .clicked()
                    {
                        self.search_selected_ids.clear();
                    }
                });

                ui.add_space(10.0);

                // Uniform Results list (no enlarged card at top)
                if filtered.is_empty() {
                    egui::Frame::group(ui.style())
                        .inner_margin(24.0)
                        .rounding(10.0)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.vertical_centered(|ui| {
                                ui.label(egui::RichText::new(s.search_no_results).weak());
                            });
                        });
                } else {
                    egui::Frame::group(ui.style())
                        .inner_margin(10.0)
                        .rounding(10.0)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            render_search_results_scroll(
                                ui,
                                "music_search_results",
                                460.0,
                                &filtered,
                                &mut self.search_selected_ids,
                                &self.thumbnails,
                                is_dark,
                                scale,
                                s,
                                &mut thumbs_to_fetch,
                                &mut download_item,
                                &mut inspect_item,
                            );
                        });
                }
            }
        });

        for turl in thumbs_to_fetch {
            self.request_thumbnail(&turl, ctx);
        }
        if trigger_search {
            self.run_music_search(ctx.clone());
        }
        if let Some(items) = download_selected {
            self.search_selected_ids.clear();
            self.download_selected_search_results(items, false, ctx.clone());
        }
        if let Some(item) = download_item {
            self.download_search_result(&item, false, ctx.clone());
        }
        if let Some(item) = inspect_item {
            self.inspect_search_result(&item, ctx.clone());
        }
        if open_manager {
            self.active_tab = ActiveTab::Manager;
        }
    }

    fn render_manager(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let s = i18n::strings(self.settings.language);
        let scale = self.font_scale();
        let max_w = ui.available_width().min(1080.0);

        let mut cancel_id: Option<String> = None;
        let mut retry_req: Option<DownloadRequest> = None;
        let mut remove_id: Option<String> = None;
        let mut open_path: Option<String> = None;
        let mut clear_finished = false;

        ui.vertical(|ui| {
            ui.set_max_width(max_w);

            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(s.mgr_title).size(22.0 * scale).strong());
                    ui.label(egui::RichText::new(s.mgr_subtitle).weak());
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let has_finished = self
                        .downloads
                        .iter()
                        .any(|d| d.status == "completed" || d.status == "failed" || d.status == "cancelled");
                    if ui
                        .add_enabled(has_finished, egui::Button::new(s.mgr_clear))
                        .clicked()
                    {
                        clear_finished = true;
                    }
                });
            });

            ui.add_space(12.0);

            let mut cnt_active = 0usize;
            let mut cnt_completed = 0usize;
            let mut cnt_failed = 0usize;
            for d in &self.downloads {
                match d.status.as_str() {
                    "downloading" | "queued" | "processing" => cnt_active += 1,
                    "completed" => cnt_completed += 1,
                    _ => cnt_failed += 1,
                }
            }

            ui.horizontal(|ui| {
                for (f, label) in [
                    (ManagerFilter::All, format!("{} ({})", s.mgr_all, self.downloads.len())),
                    (ManagerFilter::Active, format!("{} ({})", s.mgr_active, cnt_active)),
                    (
                        ManagerFilter::Completed,
                        format!("{} ({})", s.mgr_completed, cnt_completed),
                    ),
                    (ManagerFilter::Failed, format!("{} ({})", s.mgr_failed, cnt_failed)),
                ] {
                    if ui.selectable_label(self.mgr_filter == f, label).clicked() {
                        self.mgr_filter = f;
                    }
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.mgr_search)
                            .hint_text(s.mgr_search_hint)
                            .desired_width(240.0),
                    );
                });
            });

            ui.add_space(12.0);

            let q = self.mgr_search.trim().to_ascii_lowercase();
            let filtered: Vec<DownloadItem> = self
                .downloads
                .iter()
                .filter(|d| {
                    let match_status = match self.mgr_filter {
                        ManagerFilter::All => true,
                        ManagerFilter::Active => {
                            d.status == "downloading"
                                || d.status == "queued"
                                || d.status == "processing"
                        }
                        ManagerFilter::Completed => d.status == "completed",
                        ManagerFilter::Failed => {
                            d.status == "failed" || d.status == "cancelled"
                        }
                    };
                    if !match_status {
                        return false;
                    }
                    if q.is_empty() {
                        return true;
                    }
                    d.title.to_ascii_lowercase().contains(&q)
                        || d.url.to_ascii_lowercase().contains(&q)
                        || d.extractor.to_ascii_lowercase().contains(&q)
                })
                .cloned()
                .collect();

            if filtered.is_empty() {
                egui::Frame::group(ui.style())
                    .inner_margin(28.0)
                    .rounding(10.0)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.vertical_centered(|ui| {
                            let msg = if self.downloads.is_empty() {
                                s.mgr_empty
                            } else {
                                s.mgr_empty_filter
                            };
                            ui.label(egui::RichText::new(msg).weak());
                        });
                    });
            } else {
                egui::Frame::group(ui.style())
                    .inner_margin(12.0)
                    .rounding(10.0)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        for (idx, item) in filtered.iter().enumerate() {
                            if idx > 0 {
                                ui.separator();
                            }
                            let is_running = item.status == "downloading"
                                || item.status == "queued"
                                || item.status == "processing";
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.set_max_width(
                                        (ui.available_width() - 360.0 * scale.max(1.0)).max(180.0),
                                    );
                                    ui.label(egui::RichText::new(&item.title).strong());
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(
                                            egui::RichText::new(&item.extractor).small().weak(),
                                        );
                                        ui.label(egui::RichText::new("·").small().weak());
                                        ui.label(
                                            egui::RichText::new(item.format.to_uppercase())
                                                .small()
                                                .monospace()
                                                .weak(),
                                        );
                                        ui.label(egui::RichText::new("·").small().weak());
                                        ui.label(
                                            egui::RichText::new(&item.quality)
                                                .small()
                                                .monospace()
                                                .weak(),
                                        );
                                    });
                                    if let Some(pl) = &item.playlist_progress {
                                        ui.label(
                                            egui::RichText::new(pl)
                                                .small()
                                                .color(egui::Color32::from_rgb(59, 130, 246)),
                                        );
                                    }
                                    if let Some(err) = &item.error {
                                        ui.label(
                                            egui::RichText::new(i18n::translate_backend_error(
                                                err,
                                                self.settings.language,
                                            ))
                                            .small()
                                            .color(egui::Color32::from_rgb(239, 68, 68)),
                                        );
                                    }
                                });

                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.button(s.mgr_remove).clicked() {
                                            remove_id = Some(item.id.clone());
                                        }
                                        if item.status == "completed" || item.output_path.is_some() {
                                            if ui.button(s.mgr_open_folder).clicked() {
                                                open_path = Some(
                                                    item.output_path
                                                        .clone()
                                                        .unwrap_or_else(|| item.output_dir.clone()),
                                                );
                                            }
                                        }
                                        if item.status == "failed" || item.status == "cancelled" {
                                            if ui.button(s.mgr_retry).clicked() {
                                                retry_req = Some(item.request.clone());
                                            }
                                        }
                                        if is_running {
                                            if ui.button(s.mgr_cancel).clicked() {
                                                cancel_id = Some(item.id.clone());
                                            }
                                        }

                                        ui.add_space(8.0);
                                        ui.vertical(|ui| {
                                            ui.set_width(170.0);
                                            ui.horizontal(|ui| {
                                                let st_lbl = match item.status.as_str() {
                                                    "completed" => s.status_completed,
                                                    "downloading" => s.status_downloading,
                                                    "processing" => s.status_processing,
                                                    "queued" => s.status_queued,
                                                    "cancelled" => s.status_cancelled,
                                                    _ => s.status_failed,
                                                };
                                                ui.label(
                                                    egui::RichText::new(format!(
                                                        "{} · {:.0}%",
                                                        st_lbl, item.progress
                                                    ))
                                                    .small()
                                                    .monospace(),
                                                );
                                            });
                                            ui.add_sized(
                                                [170.0, 8.0],
                                                egui::ProgressBar::new(
                                                    (item.progress / 100.0) as f32,
                                                ),
                                            );
                                            ui.label(
                                                egui::RichText::new(format!(
                                                    "{} · ETA {} · {}",
                                                    item.speed, item.eta, item.total_size
                                                ))
                                                .small()
                                                .monospace()
                                                .weak(),
                                            );
                                        });
                                    },
                                );
                            });
                        }
                    });
            }
        });

        if let Some(id) = cancel_id {
            let mgr = self.manager.clone();
            let tx = self.tx.clone();
            let c = ctx.clone();
            self.rt.spawn(async move {
                downloader::cancel_download(mgr, tx, id).await;
                c.request_repaint();
            });
        }
        if let Some(req) = retry_req {
            let mgr = self.manager.clone();
            let tx = self.tx.clone();
            let c = ctx.clone();
            self.rt.spawn(async move {
                let _ = downloader::enqueue_download(mgr, tx, req).await;
                c.request_repaint();
            });
        }
        if let Some(id) = remove_id {
            self.downloads.retain(|d| d.id != id);
            let mgr = self.manager.clone();
            let c = ctx.clone();
            self.rt.spawn(async move {
                downloader::remove_download(mgr, id).await;
                c.request_repaint();
            });
        }
        if clear_finished {
            self.downloads.retain(|d| {
                d.status == "downloading" || d.status == "queued" || d.status == "processing"
            });
            let mgr = self.manager.clone();
            self.rt.spawn(async move {
                {
                    let mut items = mgr.items.lock().await;
                    items.retain(|i| {
                        i.status == "downloading"
                            || i.status == "queued"
                            || i.status == "processing"
                    });
                }
                mgr.persist().await;
            });
        }
        if let Some(path) = open_path {
            downloader::open_folder(&path);
        }
    }

    fn render_settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let s = i18n::strings(self.settings.language);
        let scale = self.font_scale();
        let max_w = ui.available_width().min(780.0);

        ui.vertical(|ui| {
            ui.set_max_width(max_w);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(s.set_title).size(22.0 * scale).strong());
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(concat!("VideoDownloader v", env!("CARGO_PKG_VERSION")))
                        .monospace()
                        .color(egui::Color32::from_rgb(96, 165, 250)),
                );
            });
            ui.label(egui::RichText::new(s.set_subtitle).weak());
            ui.add_space(12.0);

            // Appearance & Font Sizes
            egui::Frame::group(ui.style())
                .inner_margin(16.0)
                .rounding(10.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(egui::RichText::new(s.set_appearance).size(15.0 * scale).strong());
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label(s.set_theme);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .selectable_label(self.settings.theme == ThemeMode::Dark, s.set_dark)
                                .clicked()
                                && self.settings.theme != ThemeMode::Dark
                            {
                                self.toggle_theme(ctx);
                            }
                            if ui
                                .selectable_label(
                                    self.settings.theme == ThemeMode::Light,
                                    s.set_light,
                                )
                                .clicked()
                                && self.settings.theme != ThemeMode::Light
                            {
                                self.toggle_theme(ctx);
                            }
                        });
                    });

                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(8.0);

                    // Quick Font Size Presets
                    ui.horizontal_wrapped(|ui| {
                        for (label, base, small) in [
                            (s.set_font_preset_compact, 13.0_f32, 10.5_f32),
                            (s.set_font_preset_default, 14.0_f32, 12.0_f32),
                            (s.set_font_preset_large, 16.0_f32, 13.5_f32),
                            (s.set_font_preset_xlarge, 18.0_f32, 15.0_f32),
                        ] {
                            let active = (self.settings_draft.font_size - base).abs() < 0.25
                                && (self.settings_draft.small_font_size - small).abs() < 0.25;
                            if ui
                                .selectable_label(
                                    active,
                                    format!("{} ({:.0}/{:.0})", label, base, small),
                                )
                                .clicked()
                            {
                                self.settings_draft.font_size = base;
                                self.settings_draft.small_font_size = small;
                                self.pending_font_apply = true;
                                ctx.request_repaint();
                            }
                        }
                    });

                    ui.add_space(8.0);
                    let pointer_down = ctx.input(|i| i.pointer.any_down());
                    let step_sz = [(26.0 * scale.max(1.0)).round(), (22.0 * scale.max(1.0)).round()];
                    egui::Grid::new("settings_font_grid")
                        .num_columns(2)
                        .spacing([20.0, 8.0])
                        .show(ui, |ui| {
                            ui.label(s.set_font_size);
                            ui.horizontal(|ui| {
                                if ui.add_sized(step_sz, egui::Button::new("-")).clicked() {
                                    self.settings_draft.font_size =
                                        (self.settings_draft.font_size - 1.0).clamp(11.0, 22.0);
                                    self.pending_font_apply = true;
                                    ctx.request_repaint();
                                }
                                let r = ui.add(
                                    egui::Slider::new(
                                        &mut self.settings_draft.font_size,
                                        11.0..=22.0,
                                    )
                                    .step_by(1.0)
                                    .suffix(" px"),
                                );
                                if ui.add_sized(step_sz, egui::Button::new("+")).clicked() {
                                    self.settings_draft.font_size =
                                        (self.settings_draft.font_size + 1.0).clamp(11.0, 22.0);
                                    self.pending_font_apply = true;
                                    ctx.request_repaint();
                                }
                                if r.drag_stopped()
                                    || r.lost_focus()
                                    || (r.changed() && !pointer_down)
                                {
                                    self.pending_font_apply = true;
                                    ctx.request_repaint();
                                }
                            });
                            ui.end_row();

                            ui.label(s.set_small_font_size);
                            ui.horizontal(|ui| {
                                if ui.add_sized(step_sz, egui::Button::new("-")).clicked() {
                                    self.settings_draft.small_font_size =
                                        (self.settings_draft.small_font_size - 1.0)
                                            .clamp(9.0, 18.0);
                                    self.pending_font_apply = true;
                                    ctx.request_repaint();
                                }
                                let r = ui.add(
                                    egui::Slider::new(
                                        &mut self.settings_draft.small_font_size,
                                        9.0..=18.0,
                                    )
                                    .step_by(0.5)
                                    .suffix(" px"),
                                );
                                if ui.add_sized(step_sz, egui::Button::new("+")).clicked() {
                                    self.settings_draft.small_font_size =
                                        (self.settings_draft.small_font_size + 1.0)
                                            .clamp(9.0, 18.0);
                                    self.pending_font_apply = true;
                                    ctx.request_repaint();
                                }
                                if r.drag_stopped()
                                    || r.lost_focus()
                                    || (r.changed() && !pointer_down)
                                {
                                    self.pending_font_apply = true;
                                    ctx.request_repaint();
                                }
                            });
                            ui.end_row();
                        });
                });

            ui.add_space(12.0);

            // Paths & Defaults
            egui::Frame::group(ui.style())
                .inner_margin(16.0)
                .rounding(10.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(egui::RichText::new(s.set_paths).size(15.0 * scale).strong());
                    ui.add_space(8.0);

                    ui.label(egui::RichText::new(s.set_dl_dir).small().weak());
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.settings_draft.download_dir)
                                .desired_width((ui.available_width() - 105.0).max(200.0)),
                        );
                        if ui.button(s.set_browse).clicked() {
                            if let Some(folder) =
                                rfd::FileDialog::new().set_title(s.set_dl_dir).pick_folder()
                            {
                                self.settings_draft.download_dir =
                                    folder.to_string_lossy().to_string();
                            }
                        }
                    });

                    ui.add_space(10.0);
                    egui::Grid::new("settings_defaults_grid")
                        .num_columns(2)
                        .spacing([24.0, 8.0])
                        .show(ui, |ui| {
                            ui.label(s.set_def_mode);
                            egui::ComboBox::from_id_salt("set_def_mode")
                                .selected_text(if self.settings_draft.default_mode == "audio" {
                                    s.studio_audio
                                } else {
                                    s.studio_video
                                })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut self.settings_draft.default_mode,
                                        "video".to_string(),
                                        s.studio_video,
                                    );
                                    ui.selectable_value(
                                        &mut self.settings_draft.default_mode,
                                        "audio".to_string(),
                                        s.studio_audio,
                                    );
                                });
                            ui.end_row();

                            ui.label(s.set_def_vfmt);
                            egui::ComboBox::from_id_salt("set_def_vfmt")
                                .selected_text(self.settings_draft.default_video_format.to_uppercase())
                                .show_ui(ui, |ui| {
                                    for fmt in ["mp4", "mkv", "webm"] {
                                        ui.selectable_value(
                                            &mut self.settings_draft.default_video_format,
                                            fmt.to_string(),
                                            fmt.to_uppercase(),
                                        );
                                    }
                                });
                            ui.end_row();

                            ui.label(s.set_def_vq);
                            egui::ComboBox::from_id_salt("set_def_vq")
                                .selected_text(video_quality_label(
                                    &self.settings_draft.default_video_quality,
                                    s,
                                ))
                                .show_ui(ui, |ui| {
                                    for q in ["best", "2160", "1440", "1080", "720", "480"] {
                                        ui.selectable_value(
                                            &mut self.settings_draft.default_video_quality,
                                            q.to_string(),
                                            video_quality_label(q, s),
                                        );
                                    }
                                });
                            ui.end_row();

                            ui.label(s.set_def_afmt);
                            egui::ComboBox::from_id_salt("set_def_afmt")
                                .selected_text(self.settings_draft.default_audio_format.to_uppercase())
                                .show_ui(ui, |ui| {
                                    for fmt in ["mp3", "flac", "m4a", "opus", "wav"] {
                                        ui.selectable_value(
                                            &mut self.settings_draft.default_audio_format,
                                            fmt.to_string(),
                                            fmt.to_uppercase(),
                                        );
                                    }
                                });
                            ui.end_row();

                            ui.label(s.set_def_aq);
                            egui::ComboBox::from_id_salt("set_def_aq")
                                .selected_text(audio_quality_label(
                                    &self.settings_draft.default_audio_quality,
                                    s,
                                ))
                                .show_ui(ui, |ui| {
                                    for q in ["320", "256", "192", "128"] {
                                        ui.selectable_value(
                                            &mut self.settings_draft.default_audio_quality,
                                            q.to_string(),
                                            audio_quality_label(q, s),
                                        );
                                    }
                                });
                            ui.end_row();
                        });
                });

            ui.add_space(12.0);

            // In-app Engine Updates (yt-dlp & ffmpeg)
            egui::Frame::group(ui.style())
                .inner_margin(16.0)
                .rounding(10.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        egui::RichText::new(s.set_engines_section)
                            .size(15.0 * scale)
                            .strong(),
                    );
                    ui.label(egui::RichText::new(s.set_engines_desc).small().weak());
                    ui.add_space(8.0);

                    if let Some(st) = &self.engine_status {
                        ui.label(
                            egui::RichText::new(format!(
                                "yt-dlp: {} ({})",
                                st.current_version.as_deref().unwrap_or(s.engine_not_installed),
                                st.binary_path.as_deref().unwrap_or("—")
                            ))
                            .small()
                            .monospace(),
                        );
                        ui.label(
                            egui::RichText::new(format!(
                                "ffmpeg: {} [{}] ({})",
                                st.ffmpeg_version.as_deref().unwrap_or(s.engine_missing),
                                st.ffmpeg_source,
                                st.ffmpeg_path.as_deref().unwrap_or("—")
                            ))
                            .small()
                            .monospace(),
                        );
                        ui.label(
                            egui::RichText::new(format!(
                                "JS (EJS): {} {} [{}] ({})",
                                st.js_runtime_name.as_deref().unwrap_or(s.engine_missing),
                                st.js_runtime_version.as_deref().unwrap_or("—"),
                                st.js_runtime_source,
                                st.js_runtime_path.as_deref().unwrap_or("—")
                            ))
                            .small()
                            .monospace(),
                        );
                        ui.add_space(8.0);
                    }

                    let busy = self.checking_engines
                        || self.updating_ytdlp
                        || self.updating_ffmpeg
                        || self.updating_deno;
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add_enabled(!busy, egui::Button::new(s.engine_check))
                            .clicked()
                        {
                            self.check_engines(ctx.clone());
                        }
                        if ui
                            .add_enabled(!self.updating_ytdlp, egui::Button::new(s.engine_update_ytdlp))
                            .clicked()
                        {
                            self.update_ytdlp(ctx.clone());
                        }
                        if ui
                            .add_enabled(
                                !self.updating_ffmpeg,
                                egui::Button::new(if self.updating_ffmpeg {
                                    s.engine_updating_ffmpeg
                                } else {
                                    s.engine_update_ffmpeg
                                }),
                            )
                            .clicked()
                        {
                            self.update_ffmpeg(ctx.clone());
                        }
                        let deno_btn_lbl = if self.updating_deno {
                            s.engine_updating_deno
                        } else if self
                            .engine_status
                            .as_ref()
                            .is_some_and(|st| st.js_runtime_source == "local")
                        {
                            s.engine_update_deno
                        } else {
                            s.engine_reinstall_deno
                        };
                        if ui
                            .add_enabled(!self.updating_deno, egui::Button::new(deno_btn_lbl))
                            .clicked()
                        {
                            self.update_deno(ctx.clone());
                        }
                    });
                });

            ui.add_space(12.0);

            // YouTube Authentication (Cookies)
            egui::Frame::group(ui.style())
                .inner_margin(16.0)
                .rounding(10.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(egui::RichText::new(s.set_auth_title).size(15.0 * scale).strong());
                    ui.label(egui::RichText::new(s.set_auth_desc).small().weak());
                    ui.add_space(8.0);

                    ui.horizontal(|ui| {
                        ui.label(s.set_browser_cookies);
                        egui::ComboBox::from_id_salt("cookie_browser_combo")
                            .selected_text(if self.settings_draft.cookie_browser == "none" {
                                s.set_browser_none
                            } else {
                                &self.settings_draft.cookie_browser
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.settings_draft.cookie_browser,
                                    "none".to_string(),
                                    s.set_browser_none,
                                );
                                for (val, lbl) in [
                                    ("firefox", "Mozilla Firefox"),
                                    ("chrome", "Google Chrome"),
                                    ("chromium", "Chromium"),
                                    ("brave", "Brave"),
                                    ("edge", "Microsoft Edge"),
                                    ("opera", "Opera"),
                                    ("vivaldi", "Vivaldi"),
                                ] {
                                    ui.selectable_value(
                                        &mut self.settings_draft.cookie_browser,
                                        val.to_string(),
                                        lbl,
                                    );
                                }
                            });
                    });

                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(egui::RichText::new(s.set_cookies_status).small());
                        if self.settings_draft.has_cookies_file {
                            ui.label(
                                egui::RichText::new(s.set_cookies_active)
                                    .small()
                                    .color(egui::Color32::from_rgb(16, 185, 129)),
                            );
                        } else {
                            ui.label(egui::RichText::new(s.set_cookies_empty).small().weak());
                        }
                        if ui.button(s.set_cookies_load).clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("Cookies", &["txt"])
                                .pick_file()
                            {
                                match std::fs::read_to_string(&path) {
                                    Ok(content) => match config::save_cookies_txt(&content) {
                                        Ok(()) => {
                                            self.settings.has_cookies_file = true;
                                            self.settings_draft.has_cookies_file = true;
                                            self.cookie_status_msg =
                                                Some(s.set_cookies_saved.to_string());
                                            self.cookie_error_msg = None;
                                        }
                                        Err(e) => self.cookie_error_msg = Some(e),
                                    },
                                    Err(e) => self.cookie_error_msg = Some(e.to_string()),
                                }
                            }
                        }
                        if self.settings_draft.has_cookies_file
                            && ui.button(s.set_cookies_clear).clicked()
                        {
                            if let Err(e) = config::clear_cookies_txt() {
                                self.cookie_error_msg = Some(e);
                            } else {
                                self.settings.has_cookies_file = false;
                                self.settings_draft.has_cookies_file = false;
                                self.cookie_status_msg = Some(s.set_cookies_cleared.to_string());
                                self.cookie_error_msg = None;
                            }
                        }
                    });

                    ui.add_space(6.0);
                    ui.add(
                        egui::TextEdit::multiline(&mut self.cookies_text)
                            .desired_rows(3)
                            .desired_width(f32::INFINITY)
                            .hint_text("# Netscape HTTP Cookie File..."),
                    );
                    ui.add_space(4.0);
                    if ui
                        .add_enabled(
                            !self.cookies_text.trim().is_empty(),
                            egui::Button::new(s.set_cookies_save),
                        )
                        .clicked()
                    {
                        match config::save_cookies_txt(&self.cookies_text) {
                            Ok(()) => {
                                self.cookies_text.clear();
                                self.settings.has_cookies_file = true;
                                self.settings_draft.has_cookies_file = true;
                                self.cookie_status_msg = Some(s.set_cookies_saved.to_string());
                                self.cookie_error_msg = None;
                            }
                            Err(e) => self.cookie_error_msg = Some(e),
                        }
                    }

                    if let Some(msg) = &self.cookie_status_msg {
                        ui.label(
                            egui::RichText::new(msg)
                                .small()
                                .color(egui::Color32::from_rgb(16, 185, 129)),
                        );
                    }
                    if let Some(err) = &self.cookie_error_msg {
                        ui.label(
                            egui::RichText::new(err)
                                .small()
                                .color(egui::Color32::from_rgb(239, 68, 68)),
                        );
                    }
                });

            ui.add_space(12.0);

            // Behavior toggles
            egui::Frame::group(ui.style())
                .inner_margin(16.0)
                .rounding(10.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(egui::RichText::new(s.set_behavior).size(15.0 * scale).strong());
                    ui.add_space(6.0);
                    ui.checkbox(&mut self.settings_draft.auto_check_ytdlp, s.set_auto_check);
                    ui.checkbox(&mut self.settings_draft.embed_thumbnail, s.set_embed_thumb);
                    ui.checkbox(&mut self.settings_draft.embed_metadata, s.set_embed_meta);
                    ui.checkbox(
                        &mut self.settings_draft.include_id_in_filename,
                        s.set_include_id_filename,
                    );
                });

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if self.settings_saved_notice {
                    ui.label(
                        egui::RichText::new(format!("✓ {}", s.set_saved))
                            .color(egui::Color32::from_rgb(16, 185, 129)),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new(s.set_save)
                                    .strong()
                                    .color(egui::Color32::WHITE),
                            )
                            .fill(egui::Color32::from_rgb(37, 99, 235))
                            .min_size(egui::vec2(140.0, 32.0)),
                        )
                        .clicked()
                    {
                        self.settings_draft.theme = self.settings.theme;
                        self.settings_draft.language = self.settings.language;
                        if self.settings_draft.download_dir.trim().is_empty() {
                            self.settings_draft.download_dir = self.settings.download_dir.clone();
                        }
                        self.settings_draft.font_size =
                            self.settings_draft.font_size.clamp(11.0, 22.0);
                        self.settings_draft.small_font_size =
                            self.settings_draft.small_font_size.clamp(9.0, 18.0);
                        if config::save_settings(&self.settings_draft).is_ok() {
                            self.settings = self.settings_draft.clone();
                            self.mode = self.settings.default_mode.clone();
                            self.video_format = self.settings.default_video_format.clone();
                            self.video_quality = self.settings.default_video_quality.clone();
                            self.audio_format = self.settings.default_audio_format.clone();
                            self.audio_quality = self.settings.default_audio_quality.clone();
                            self.embed_thumbnail = self.settings.embed_thumbnail;
                            self.embed_metadata = self.settings.embed_metadata;
                            apply_font_styles(
                                ctx,
                                self.settings.font_size,
                                self.settings.small_font_size,
                            );
                            self.settings_saved_notice = true;
                        }
                    }
                });
            });
        });
    }
}

fn video_quality_label(q: &str, s: &i18n::Strings) -> String {
    match q {
        "best" => s.studio_video_best.to_string(),
        other if other.chars().all(|c| c.is_ascii_digit()) && !other.is_empty() => {
            format!("{}p", other)
        }
        other => other.to_string(),
    }
}

fn audio_quality_label(q: &str, s: &i18n::Strings) -> String {
    match q {
        "best" => s.studio_audio_best_q.to_string(),
        other if other.chars().all(|c| c.is_ascii_digit()) && !other.is_empty() => {
            format!("{} kbps", other)
        }
        other => other.to_string(),
    }
}

fn normalize_url_for_match(u: &str) -> String {
    u.trim()
        .trim_end_matches('/')
        .replace("://music.youtube.com/", "://www.youtube.com/")
        .replace("://m.youtube.com/", "://www.youtube.com/")
}

fn extract_query_param<'a>(url: &'a str, key: &str) -> Option<&'a str> {
    let query = url.split('?').nth(1)?;
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            if k == key && !v.is_empty() {
                return Some(v);
            }
        }
    }
    None
}

fn urls_match(a: &str, b: &str) -> bool {
    let na = normalize_url_for_match(a);
    let nb = normalize_url_for_match(b);
    if na == nb || nb.contains(&na) || na.contains(&nb) {
        return true;
    }
    if let (Some(la), Some(lb)) = (extract_query_param(&na, "list"), extract_query_param(&nb, "list")) {
        if la == lb {
            return true;
        }
    }
    if let (Some(va), Some(vb)) = (extract_query_param(&na, "v"), extract_query_param(&nb, "v")) {
        if va == vb {
            return true;
        }
    }
    false
}

#[allow(clippy::too_many_arguments)]
fn render_search_results_scroll(
    ui: &mut egui::Ui,
    scroll_id: &str,
    max_height: f32,
    filtered: &[MusicSearchResult],
    selected_ids: &mut HashSet<String>,
    thumbnails: &HashMap<String, ThumbnailState>,
    is_dark: bool,
    scale: f32,
    s: &i18n::Strings,
    thumbs_to_fetch: &mut Vec<String>,
    download_item: &mut Option<MusicSearchResult>,
    inspect_item: &mut Option<MusicSearchResult>,
) {
    let ctrl_held = ui.input(|i| i.modifiers.ctrl || i.modifiers.command);

    egui::ScrollArea::vertical()
        .id_salt(scroll_id)
        .max_height(max_height)
        .show(ui, |ui| {
            for (idx, item) in filtered.iter().enumerate() {
                if idx > 0 {
                    ui.add_space(6.0);
                }
                let is_selected = selected_ids.contains(&item.id);
                let is_album = item.kind == MusicSearchItemKind::Album;
                let is_video = item.kind == MusicSearchItemKind::Video;

                let row_fill = if is_selected {
                    if is_dark {
                        egui::Color32::from_rgb(30, 41, 59)
                    } else {
                        egui::Color32::from_rgb(239, 246, 255)
                    }
                } else if is_album {
                    if is_dark {
                        egui::Color32::from_rgb(24, 31, 46)
                    } else {
                        egui::Color32::from_rgb(255, 251, 235)
                    }
                } else {
                    egui::Color32::TRANSPARENT
                };

                let row_stroke = if is_selected {
                    egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(59, 130, 246))
                } else if is_album {
                    egui::Stroke::new(
                        1.0_f32,
                        egui::Color32::from_rgba_unmultiplied(217, 119, 6, 110),
                    )
                } else {
                    ui.visuals().widgets.noninteractive.bg_stroke
                };

                egui::Frame::none()
                    .fill(row_fill)
                    .stroke(row_stroke)
                    .rounding(8.0)
                    .inner_margin(10.0)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            let mut checked = is_selected;
                            if ui.checkbox(&mut checked, "").changed() {
                                if checked {
                                    selected_ids.insert(item.id.clone());
                                } else {
                                    selected_ids.remove(&item.id);
                                }
                            }
                            ui.add_space(4.0);

                            let body_w = (ui.available_width() - 255.0 * scale.max(1.0)).max(160.0);
                            let body_resp = ui
                                .scope(|ui| {
                                    ui.set_min_width(body_w);
                                    ui.set_max_width(body_w);
                                    ui.horizontal(|ui| {
                                        if let Some(turl) = &item.thumbnail {
                                            match thumbnails.get(turl) {
                                                Some(ThumbnailState::Ready(tex)) => {
                                                    ui.add(
                                                        egui::Image::new(tex)
                                                            .max_size(egui::vec2(48.0, 48.0))
                                                            .rounding(6.0),
                                                    );
                                                    ui.add_space(8.0);
                                                }
                                                None => {
                                                    thumbs_to_fetch.push(turl.clone());
                                                }
                                                _ => {}
                                            }
                                        }

                                        ui.vertical(|ui| {
                                            ui.set_max_width(
                                                (ui.available_width() - 8.0).max(120.0),
                                            );
                                            ui.horizontal_wrapped(|ui| {
                                                render_kind_badge(ui, item, s);
                                                render_source_badge(ui, item.source);
                                                ui.label(
                                                    egui::RichText::new(&item.title).strong(),
                                                );
                                            });
                                            ui.label(
                                                egui::RichText::new(format_search_meta(item))
                                                    .small()
                                                    .weak(),
                                            );
                                        });
                                    });
                                })
                                .response
                                .interact(egui::Sense::click())
                                .on_hover_cursor(egui::CursorIcon::PointingHand);

                            if body_resp.clicked() {
                                if ctrl_held {
                                    if !selected_ids.remove(&item.id) {
                                        selected_ids.insert(item.id.clone());
                                    }
                                } else {
                                    selected_ids.clear();
                                    selected_ids.insert(item.id.clone());
                                }
                            }

                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let (btn_lbl, btn_fill) = if is_video {
                                        (
                                            format!("⬇ {}", s.search_download_video),
                                            egui::Color32::from_rgb(13, 148, 136),
                                        )
                                    } else if is_album {
                                        (
                                            format!("⬇ {}", s.search_download_album),
                                            egui::Color32::from_rgb(217, 119, 6),
                                        )
                                    } else {
                                        (
                                            format!("⬇ {}", s.search_download_song),
                                            egui::Color32::from_rgb(37, 99, 235),
                                        )
                                    };
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                egui::RichText::new(btn_lbl)
                                                    .small()
                                                    .strong()
                                                    .color(egui::Color32::WHITE),
                                            )
                                            .fill(btn_fill),
                                        )
                                        .clicked()
                                    {
                                        *download_item = Some(item.clone());
                                    }

                                    if ui
                                        .add(egui::Button::new(
                                            egui::RichText::new(s.search_inspect).small(),
                                        ))
                                        .clicked()
                                    {
                                        *inspect_item = Some(item.clone());
                                    }
                                },
                            );
                        });
                    });
            }
        });
}

fn render_kind_badge(ui: &mut egui::Ui, item: &MusicSearchResult, s: &i18n::Strings) {
    let (bg, label) = match item.kind {
        MusicSearchItemKind::Video => (
            egui::Color32::from_rgb(13, 148, 136),
            format!("🎬 {}", s.search_badge_video),
        ),
        MusicSearchItemKind::Album => {
            let lbl = match item.release_type.as_deref() {
                Some(rt) if !rt.eq_ignore_ascii_case("album") => {
                    format!("💿 {} ({})", s.search_badge_album, rt.to_uppercase())
                }
                _ => format!("💿 {}", s.search_badge_album),
            };
            (egui::Color32::from_rgb(217, 119, 6), lbl)
        }
        MusicSearchItemKind::Song => (
            egui::Color32::from_rgb(37, 99, 235),
            format!("🎵 {}", s.search_badge_song),
        ),
    };

    egui::Frame::none()
        .fill(bg)
        .rounding(4.0)
        .inner_margin(egui::Margin::symmetric(6.0, 2.0))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(label)
                    .small()
                    .strong()
                    .color(egui::Color32::WHITE),
            );
        });
}

fn render_source_badge(ui: &mut egui::Ui, source: MusicSearchSource) {
    let bg = match source {
        MusicSearchSource::Youtube => egui::Color32::from_rgb(185, 28, 28),
        MusicSearchSource::YoutubeMusic => egui::Color32::from_rgb(190, 18, 60),
        MusicSearchSource::Bandcamp => egui::Color32::from_rgb(15, 118, 110),
        MusicSearchSource::Dailymotion => egui::Color32::from_rgb(0, 102, 204),
        MusicSearchSource::Videa => egui::Color32::from_rgb(126, 34, 206),
        MusicSearchSource::OkRu => egui::Color32::from_rgb(234, 88, 12),
    };
    egui::Frame::none()
        .fill(bg)
        .rounding(4.0)
        .inner_margin(egui::Margin::symmetric(6.0, 2.0))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(source.label())
                    .small()
                    .color(egui::Color32::WHITE),
            );
        });
}

fn format_search_meta(item: &MusicSearchResult) -> String {
    let mut parts = vec![item.artist.clone()];
    if item.kind == MusicSearchItemKind::Song {
        if let Some(alb) = &item.album {
            if !alb.trim().is_empty() {
                parts.push(alb.clone());
            }
        }
    }
    if let Some(year) = &item.year {
        if !year.trim().is_empty() {
            parts.push(year.clone());
        }
    }
    if let Some(dur) = &item.duration_text {
        if !dur.trim().is_empty() {
            parts.push(dur.clone());
        }
    }
    parts.join(" · ")
}

fn draw_fullscreen_button(
    ui: &mut egui::Ui,
    is_fullscreen: bool,
    tooltip: &str,
) -> egui::Response {
    let size = egui::vec2(28.0, 24.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let bg = if is_fullscreen {
            if resp.hovered() {
                egui::Color32::from_rgb(59, 130, 246)
            } else {
                egui::Color32::from_rgb(37, 99, 235)
            }
        } else if resp.hovered() {
            ui.visuals().widgets.hovered.bg_fill
        } else {
            ui.visuals().widgets.inactive.weak_bg_fill
        };
        let fg = if is_fullscreen {
            egui::Color32::WHITE
        } else if resp.hovered() {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().text_color()
        };

        ui.painter().rect_filled(rect, 5.0, bg);
        let stroke = egui::Stroke::new(1.7_f32, fg);
        let b = rect.shrink2(egui::vec2(8.0, 6.0));
        let arm = 4.0_f32;

        if !is_fullscreen {
            // Outward corner brackets
            ui.painter().line_segment([b.left_top(), egui::pos2(b.left() + arm, b.top())], stroke);
            ui.painter().line_segment([b.left_top(), egui::pos2(b.left(), b.top() + arm)], stroke);

            ui.painter().line_segment([b.right_top(), egui::pos2(b.right() - arm, b.top())], stroke);
            ui.painter().line_segment([b.right_top(), egui::pos2(b.right(), b.top() + arm)], stroke);

            ui.painter().line_segment([b.left_bottom(), egui::pos2(b.left() + arm, b.bottom())], stroke);
            ui.painter().line_segment([b.left_bottom(), egui::pos2(b.left(), b.bottom() - arm)], stroke);

            ui.painter().line_segment([b.right_bottom(), egui::pos2(b.right() - arm, b.bottom())], stroke);
            ui.painter().line_segment([b.right_bottom(), egui::pos2(b.right(), b.bottom() - arm)], stroke);
        } else {
            // Inward corner brackets (exit fullscreen)
            let mid_x = b.center().x;
            let mid_y = b.center().y;
            let gap = 1.5_f32;

            let tl = egui::pos2(mid_x - gap, mid_y - gap);
            ui.painter().line_segment([tl, egui::pos2(tl.x - arm, tl.y)], stroke);
            ui.painter().line_segment([tl, egui::pos2(tl.x, tl.y - arm)], stroke);

            let tr = egui::pos2(mid_x + gap, mid_y - gap);
            ui.painter().line_segment([tr, egui::pos2(tr.x + arm, tr.y)], stroke);
            ui.painter().line_segment([tr, egui::pos2(tr.x, tr.y - arm)], stroke);

            let bl = egui::pos2(mid_x - gap, mid_y + gap);
            ui.painter().line_segment([bl, egui::pos2(bl.x - arm, bl.y)], stroke);
            ui.painter().line_segment([bl, egui::pos2(bl.x, bl.y + arm)], stroke);

            let br = egui::pos2(mid_x + gap, mid_y + gap);
            ui.painter().line_segment([br, egui::pos2(br.x + arm, br.y)], stroke);
            ui.painter().line_segment([br, egui::pos2(br.x, br.y + arm)], stroke);
        }
    }
    resp.on_hover_text(tooltip)
}

fn draw_minimize_button(ui: &mut egui::Ui, tooltip: &str) -> egui::Response {
    let size = egui::vec2(26.0, 24.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let bg = if resp.hovered() {
            ui.visuals().widgets.hovered.bg_fill
        } else {
            ui.visuals().widgets.inactive.weak_bg_fill
        };
        let fg = if resp.hovered() {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().text_color()
        };
        ui.painter().rect_filled(rect, 5.0, bg);
        let y = rect.center().y + 2.0;
        ui.painter().line_segment(
            [egui::pos2(rect.left() + 8.0, y), egui::pos2(rect.right() - 8.0, y)],
            egui::Stroke::new(1.7_f32, fg),
        );
    }
    resp.on_hover_text(tooltip)
}

fn draw_close_button(ui: &mut egui::Ui, tooltip: &str) -> egui::Response {
    let size = egui::vec2(26.0, 24.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let bg = if resp.hovered() {
            egui::Color32::from_rgb(220, 38, 38)
        } else {
            ui.visuals().widgets.inactive.weak_bg_fill
        };
        let fg = if resp.hovered() {
            egui::Color32::WHITE
        } else {
            ui.visuals().text_color()
        };
        ui.painter().rect_filled(rect, 5.0, bg);
        let b = rect.shrink2(egui::vec2(9.0, 7.0));
        let stroke = egui::Stroke::new(1.7_f32, fg);
        ui.painter().line_segment([b.left_top(), b.right_bottom()], stroke);
        ui.painter().line_segment([b.right_top(), b.left_bottom()], stroke);
    }
    resp.on_hover_text(tooltip)
}
