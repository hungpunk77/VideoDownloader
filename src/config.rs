use crate::types::{AppLanguage, AppSettings, ThemeMode};
use std::fs;
use std::path::PathBuf;

impl Default for AppSettings {
    fn default() -> Self {
        let default_dl = dirs::download_dir()
            .or_else(dirs::home_dir)
            .map(|p| p.join("VideoDownloader"))
            .unwrap_or_else(|| PathBuf::from("./downloads"))
            .to_string_lossy()
            .to_string();

        Self {
            download_dir: default_dl,
            default_mode: "video".to_string(),
            default_video_format: "mp4".to_string(),
            default_video_quality: "best".to_string(),
            default_audio_format: "mp3".to_string(),
            default_audio_quality: "320".to_string(),
            embed_thumbnail: true,
            embed_metadata: true,
            download_subtitles: false,
            auto_check_ytdlp: true,
            auto_paste_clipboard: false,
            max_concurrent: 2,
            cookie_browser: "none".to_string(),
            has_cookies_file: false,
            language: AppLanguage::En,
            theme: ThemeMode::Dark,
            include_id_in_filename: true,
            font_size: 14.0,
            small_font_size: 12.0,
        }
    }
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("videodownloader")
}

pub fn data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("videodownloader")
}

pub fn bin_dir() -> PathBuf {
    data_dir().join("bin")
}

pub fn config_file_path() -> PathBuf {
    config_dir().join("config.json")
}

pub fn cookies_file_path() -> PathBuf {
    config_dir().join("cookies.txt")
}

pub fn history_file_path() -> PathBuf {
    data_dir().join("history.json")
}

pub fn load_settings() -> AppSettings {
    let path = config_file_path();
    let has_cookies = cookies_file_path().exists();
    if let Ok(content) = fs::read_to_string(&path) {
        if let Ok(mut parsed) = serde_json::from_str::<AppSettings>(&content) {
            parsed.has_cookies_file = has_cookies;
            if parsed.default_video_quality.trim().is_empty() {
                parsed.default_video_quality = "best".to_string();
            }
            if parsed.default_audio_format.trim().is_empty() || parsed.default_audio_format == "best" {
                parsed.default_audio_format = "mp3".to_string();
            }
            if parsed.default_audio_quality.trim().is_empty() || parsed.default_audio_quality == "best" {
                parsed.default_audio_quality = "320".to_string();
            }
            if parsed.max_concurrent == 0 {
                parsed.max_concurrent = 2;
            }
            if !parsed.font_size.is_finite() || parsed.font_size <= 0.0 {
                parsed.font_size = 14.0;
            } else {
                parsed.font_size = parsed.font_size.clamp(11.0, 22.0);
            }
            if !parsed.small_font_size.is_finite() || parsed.small_font_size <= 0.0 {
                parsed.small_font_size = 12.0;
            } else {
                parsed.small_font_size = parsed.small_font_size.clamp(9.0, 18.0);
            }
            return parsed;
        }
    }
    let mut defaults = AppSettings::default();
    defaults.has_cookies_file = has_cookies;
    let _ = save_settings(&defaults);
    defaults
}

pub fn save_settings(settings: &AppSettings) -> Result<(), String> {
    let dir = config_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create config dir: {}", e))?;
    let mut to_save = settings.clone();
    to_save.has_cookies_file = cookies_file_path().exists();
    let json = serde_json::to_string_pretty(&to_save)
        .map_err(|e| format!("Serialization error: {}", e))?;
    fs::write(config_file_path(), json).map_err(|e| format!("Failed to save config: {}", e))?;

    let dl_path = PathBuf::from(&settings.download_dir);
    let _ = fs::create_dir_all(dl_path);
    Ok(())
}

pub fn save_cookies_txt(content: &str) -> Result<(), String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err("Cookies content cannot be empty.".to_string());
    }
    let dir = config_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create config dir: {}", e))?;
    fs::write(cookies_file_path(), trimmed)
        .map_err(|e| format!("Failed to save cookies.txt: {}", e))
}

pub fn clear_cookies_txt() -> Result<(), String> {
    let path = cookies_file_path();
    if path.exists() {
        fs::remove_file(path).map_err(|e| format!("Failed to remove cookies.txt: {}", e))?;
    }
    Ok(())
}
