use crate::config;
use crate::types::EngineStatus;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::Mutex;

static YTDLP_INSTALL_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static FFMPEG_INSTALL_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static DENO_INSTALL_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn ytdlp_lock() -> &'static Mutex<()> {
    YTDLP_INSTALL_LOCK.get_or_init(|| Mutex::new(()))
}

fn ffmpeg_lock() -> &'static Mutex<()> {
    FFMPEG_INSTALL_LOCK.get_or_init(|| Mutex::new(()))
}

fn deno_lock() -> &'static Mutex<()> {
    DENO_INSTALL_LOCK.get_or_init(|| Mutex::new(()))
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: Option<String>,
    published_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FfmpegVersionMeta {
    version: String,
    published_at: String,
}

fn ytdlp_bin_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    }
}

fn ffmpeg_bin_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    }
}

fn ffprobe_bin_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "ffprobe.exe"
    } else {
        "ffprobe"
    }
}

fn deno_bin_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "deno.exe"
    } else {
        "deno"
    }
}

fn ffmpeg_meta_path() -> PathBuf {
    config::bin_dir().join("ffmpeg.version.json")
}

pub fn resolve_ytdlp_binary() -> Option<PathBuf> {
    let bin_name = ytdlp_bin_name();
    let local_bin = config::bin_dir().join(bin_name);
    if local_bin.is_file() {
        return Some(local_bin);
    }

    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(bin_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

pub fn resolve_ffmpeg_binary() -> Option<(PathBuf, &'static str)> {
    let bin_name = ffmpeg_bin_name();
    let local_bin = config::bin_dir().join(bin_name);
    if local_bin.is_file() {
        return Some((local_bin, "local"));
    }

    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(bin_name);
            if candidate.is_file() {
                return Some((candidate, "system"));
            }
        }
    }
    None
}

pub fn resolve_ffmpeg_location() -> Option<PathBuf> {
    resolve_ffmpeg_binary().and_then(|(path, _)| path.parent().map(|p| p.to_path_buf()))
}

pub fn resolve_ffprobe_binary() -> Option<PathBuf> {
    let bin_name = ffprobe_bin_name();
    let local_bin = config::bin_dir().join(bin_name);
    if local_bin.is_file() {
        return Some(local_bin);
    }

    if let Some((ffmpeg_path, _)) = resolve_ffmpeg_binary() {
        if let Some(dir) = ffmpeg_path.parent() {
            let sibling = dir.join(bin_name);
            if sibling.is_file() {
                return Some(sibling);
            }
        }
    }

    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(bin_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn find_in_path(names: &[&str]) -> Option<PathBuf> {
    let path_var = std::env::var("PATH").ok()?;
    for dir in std::env::split_paths(&path_var) {
        for name in names {
            let exe = if cfg!(target_os = "windows") {
                format!("{}.exe", name)
            } else {
                (*name).to_string()
            };
            let candidate = dir.join(exe);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

pub fn resolve_deno_binary() -> Option<PathBuf> {
    let local_bin = config::bin_dir().join(deno_bin_name());
    if local_bin.is_file() {
        return Some(local_bin);
    }
    find_in_path(&["deno"])
}

pub fn resolve_any_js_runtime() -> Option<(&'static str, PathBuf, &'static str)> {
    let local_deno = config::bin_dir().join(deno_bin_name());
    if local_deno.is_file() {
        return Some(("deno", local_deno, "local"));
    }
    if let Some(p) = find_in_path(&["deno"]) {
        return Some(("deno", p, "system"));
    }
    if let Some(p) = find_in_path(&["node", "nodejs"]) {
        return Some(("node", p, "system"));
    }
    if let Some(p) = find_in_path(&["bun"]) {
        return Some(("bun", p, "system"));
    }
    if let Some(p) = find_in_path(&["qjs", "quickjs"]) {
        return Some(("quickjs", p, "system"));
    }
    None
}

pub fn configure_ytdlp_command(cmd: &mut Command) {
    let bin_dir = config::bin_dir();
    let mut paths: Vec<PathBuf> = vec![bin_dir.clone()];
    if let Some(existing) = std::env::var_os("PATH") {
        for p in std::env::split_paths(&existing) {
            if p != bin_dir {
                paths.push(p);
            }
        }
    }
    if let Ok(joined) = std::env::join_paths(paths) {
        cmd.env("PATH", joined);
    }
    // Ensure Python/PyInstaller (yt-dlp.exe on Windows) outputs UTF-8 on piped stdout/stderr
    // instead of the Windows legacy codepage (e.g. cp1250), keeping accented paths intact.
    cmd.env("PYTHONUTF8", "1");
    cmd.env("PYTHONIOENCODING", "utf-8");
}

// 0 = unknown, 1 = unsupported, 2 = supported
static YTDLP_HAS_JS_RUNTIMES: std::sync::atomic::AtomicU8 =
    std::sync::atomic::AtomicU8::new(0);
static YTDLP_HAS_REMOTE_COMPONENTS: std::sync::atomic::AtomicU8 =
    std::sync::atomic::AtomicU8::new(0);
static YTDLP_HAS_IMPERSONATE: std::sync::atomic::AtomicU8 =
    std::sync::atomic::AtomicU8::new(0);
static YTDLP_IMPERSONATE_UPGRADE_TRIED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

async fn probe_ytdlp_ejs_capabilities(ytdlp_bin: &Path) -> (bool, bool) {
    let cached_rt = YTDLP_HAS_JS_RUNTIMES.load(std::sync::atomic::Ordering::Relaxed);
    let cached_rc = YTDLP_HAS_REMOTE_COMPONENTS.load(std::sync::atomic::Ordering::Relaxed);
    if cached_rt != 0 && cached_rc != 0 {
        return (cached_rt == 2, cached_rc == 2);
    }

    let mut cmd = Command::new(ytdlp_bin);
    cmd.arg("--help").stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000);
    let help_text = cmd
        .output()
        .await
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();

    let has_rt = help_text.contains("--js-runtimes");
    let has_rc = help_text.contains("--remote-components");
    YTDLP_HAS_JS_RUNTIMES.store(
        if has_rt { 2 } else { 1 },
        std::sync::atomic::Ordering::Relaxed,
    );
    YTDLP_HAS_REMOTE_COMPONENTS.store(
        if has_rc { 2 } else { 1 },
        std::sync::atomic::Ordering::Relaxed,
    );
    (has_rt, has_rc)
}

pub async fn ytdlp_supports_impersonation(ytdlp_bin: &Path) -> bool {
    let cached = YTDLP_HAS_IMPERSONATE.load(std::sync::atomic::Ordering::Relaxed);
    if cached != 0 {
        return cached == 2;
    }

    let mut cmd = Command::new(ytdlp_bin);
    configure_ytdlp_command(&mut cmd);
    cmd.arg("--list-impersonate-targets")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000);

    let has_target = match cmd.output().await {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            stdout.lines().any(|line| {
                let lower = line.to_ascii_lowercase();
                let trimmed = lower.trim_start();
                if trimmed.starts_with("client") && trimmed.contains("source") {
                    return false;
                }
                (lower.contains("curl_cffi")
                    || lower.contains("chrome")
                    || lower.contains("firefox")
                    || lower.contains("edge")
                    || lower.contains("safari"))
                    && !lower.contains("unavailable")
            })
        }
        Err(_) => false,
    };

    YTDLP_HAS_IMPERSONATE.store(
        if has_target { 2 } else { 1 },
        std::sync::atomic::Ordering::Relaxed,
    );
    has_target
}

pub async fn ytdlp_ejs_args(ytdlp_bin: &Path) -> Vec<String> {
    let (has_rt, has_rc) = probe_ytdlp_ejs_capabilities(ytdlp_bin).await;
    let mut args = Vec::new();

    if has_rt {
        let mut found_runtimes: Vec<String> = Vec::new();
        if let Some(path) = resolve_deno_binary() {
            let p = path.to_string_lossy();
            if p.contains(',') {
                found_runtimes.push("deno".to_string());
            } else {
                found_runtimes.push(format!("deno:{}", p));
            }
        }
        let other_candidates: &[(&str, &[&str])] = &[
            ("node", &["node", "nodejs"]),
            ("bun", &["bun"]),
            ("quickjs", &["qjs", "quickjs"]),
        ];
        for (runtime_name, bins) in other_candidates {
            if let Some(path) = find_in_path(bins) {
                let p = path.to_string_lossy();
                if p.contains(',') {
                    found_runtimes.push((*runtime_name).to_string());
                } else {
                    found_runtimes.push(format!("{}:{}", runtime_name, p));
                }
            }
        }
        if !found_runtimes.is_empty() {
            args.push("--js-runtimes".to_string());
            args.push(found_runtimes.join(","));
        }
    }

    if has_rc {
        args.push("--remote-components".to_string());
        args.push("ejs:github".to_string());
    }

    args
}

async fn get_installed_ytdlp_version(bin: &Path) -> Option<String> {
    let mut cmd = Command::new(bin);
    cmd.arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000);

    if let Ok(out) = cmd.output().await {
        if out.status.success() {
            let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    None
}

async fn get_installed_ffmpeg_version(bin: &Path, source: &str) -> Option<String> {
    if source == "local" {
        if let Ok(raw) = fs::read_to_string(ffmpeg_meta_path()) {
            if let Ok(meta) = serde_json::from_str::<FfmpegVersionMeta>(&raw) {
                if !meta.version.trim().is_empty() {
                    return Some(meta.version);
                }
            }
        }
    }

    let mut cmd = Command::new(bin);
    cmd.arg("-version")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000);

    if let Ok(out) = cmd.output().await {
        if out.status.success() {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if let Some(first_line) = stdout.lines().next() {
                if let Some(rest) = first_line.strip_prefix("ffmpeg version ") {
                    let token = rest.split_whitespace().next().unwrap_or("").trim();
                    if !token.is_empty() {
                        if let Some(date_part) = extract_build_date_from_ffmpeg_token(token) {
                            return Some(date_part);
                        }
                        return Some(token.to_string());
                    }
                }
            }
        }
    }
    None
}

fn extract_build_date_from_ffmpeg_token(token: &str) -> Option<String> {
    let last = token.rsplit('-').next()?;
    if last.len() == 8 && last.chars().all(|c| c.is_ascii_digit()) {
        return Some(format!("{}-{}-{}", &last[0..4], &last[4..6], &last[6..8]));
    }
    None
}

async fn get_installed_js_runtime_version(name: &str, bin: &Path) -> Option<String> {
    let mut cmd = Command::new(bin);
    cmd.arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000);

    let out = cmd.output().await.ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let first_line = stdout.lines().next()?.trim();
    if name == "deno" {
        let rest = first_line.strip_prefix("deno ").unwrap_or(first_line).trim();
        let token = rest.split_whitespace().next()?.trim();
        if !token.is_empty() {
            return Some(token.trim_start_matches('v').to_string());
        }
    } else {
        let token = first_line.split_whitespace().next()?.trim();
        let cleaned = token.trim_start_matches('v');
        if !cleaned.is_empty() && cleaned.chars().any(|c| c.is_ascii_digit()) {
            return Some(cleaned.to_string());
        }
    }
    None
}

async fn fetch_latest_ytdlp_release() -> Result<(String, String), String> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("VideoDownloader/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .get("https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest")
        .send()
        .await
        .map_err(|e| format!("Network error: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("GitHub API error: {}", resp.status()));
    }

    let release: GithubRelease = resp
        .json()
        .await
        .map_err(|e| format!("JSON parse error: {}", e))?;

    let url = release
        .html_url
        .unwrap_or_else(|| "https://github.com/yt-dlp/yt-dlp/releases/latest".to_string());
    Ok((release.tag_name, url))
}

async fn fetch_latest_ffmpeg_release() -> Result<FfmpegVersionMeta, String> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("VideoDownloader/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .get("https://api.github.com/repos/yt-dlp/FFmpeg-Builds/releases/latest")
        .send()
        .await
        .map_err(|e| format!("Network error: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("GitHub API error: {}", resp.status()));
    }

    let release: GithubRelease = resp
        .json()
        .await
        .map_err(|e| format!("JSON parse error: {}", e))?;

    let published_at = release.published_at.unwrap_or_default();
    let date_version = published_at
        .get(0..10)
        .filter(|s| s.len() == 10)
        .map(|s| s.to_string())
        .unwrap_or_else(|| release.tag_name.clone());

    Ok(FfmpegVersionMeta {
        version: date_version,
        published_at,
    })
}

async fn fetch_latest_deno_release() -> Result<String, String> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("VideoDownloader/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .get("https://api.github.com/repos/denoland/deno/releases/latest")
        .send()
        .await
        .map_err(|e| format!("Network error: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("GitHub API error: {}", resp.status()));
    }

    let release: GithubRelease = resp
        .json()
        .await
        .map_err(|e| format!("JSON parse error: {}", e))?;

    Ok(release.tag_name.trim_start_matches('v').to_string())
}

fn is_newer_version(current: &str, latest: &str) -> bool {
    let clean = |s: &str| -> Vec<u32> {
        s.trim_start_matches('v')
            .split(|c: char| !c.is_ascii_digit())
            .filter(|p| !p.is_empty())
            .filter_map(|p| p.parse::<u32>().ok())
            .collect()
    };
    let c = clean(current);
    let l = clean(latest);
    l > c
}

pub async fn check_status() -> EngineStatus {
    let bin = resolve_ytdlp_binary();
    let ffmpeg_resolved = resolve_ffmpeg_binary();
    let js_rt = resolve_any_js_runtime();

    let (current_version, ffmpeg_version, js_runtime_version) = tokio::join!(
        async {
            match &bin {
                Some(path) => get_installed_ytdlp_version(path).await,
                None => None,
            }
        },
        async {
            match &ffmpeg_resolved {
                Some((path, source)) => get_installed_ffmpeg_version(path, source).await,
                None => None,
            }
        },
        async {
            match &js_rt {
                Some((name, path, _)) => get_installed_js_runtime_version(name, path).await,
                None => None,
            }
        }
    );

    let installed = current_version.is_some();
    let ffmpeg_installed = ffmpeg_resolved.is_some();
    let ffmpeg_source = ffmpeg_resolved
        .as_ref()
        .map(|(_, s)| (*s).to_string())
        .unwrap_or_else(|| "missing".to_string());
    let ffmpeg_path = ffmpeg_resolved
        .as_ref()
        .map(|(p, _)| p.to_string_lossy().to_string());

    let (yt_res, ff_res, deno_res) = tokio::join!(
        fetch_latest_ytdlp_release(),
        fetch_latest_ffmpeg_release(),
        fetch_latest_deno_release()
    );

    let checked_at = chrono::Utc::now().to_rfc3339();
    let ffmpeg_latest_version = ff_res.as_ref().ok().map(|m| m.version.clone());
    let ffmpeg_update_available = match (&ffmpeg_source[..], &ffmpeg_version, &ffmpeg_latest_version) {
        ("missing", _, _) => true,
        ("local", Some(curr), Some(latest)) => is_newer_version(curr, latest),
        ("local", None, Some(_)) => true,
        _ => false,
    };

    let js_runtime_installed = js_rt.is_some();
    let js_runtime_name = js_rt.as_ref().map(|(n, _, _)| (*n).to_string());
    let js_runtime_path = js_rt.as_ref().map(|(_, p, _)| p.to_string_lossy().to_string());
    let js_runtime_source = js_rt
        .as_ref()
        .map(|(_, _, s)| (*s).to_string())
        .unwrap_or_else(|| "missing".to_string());
    let js_runtime_latest_version = deno_res.ok();
    let js_runtime_update_available = match (
        &js_runtime_source[..],
        &js_runtime_version,
        &js_runtime_latest_version,
    ) {
        ("missing", _, _) => true,
        ("local", Some(curr), Some(latest)) => is_newer_version(curr, latest),
        ("local", None, Some(_)) => true,
        _ => false,
    };

    match yt_res {
        Ok((latest, url)) => {
            let update_available = match &current_version {
                Some(curr) => is_newer_version(curr, &latest),
                None => true,
            };
            EngineStatus {
                installed,
                current_version,
                latest_version: Some(latest),
                update_available,
                binary_path: bin.map(|p| p.to_string_lossy().to_string()),
                ffmpeg_installed,
                ffmpeg_version,
                ffmpeg_latest_version,
                ffmpeg_update_available,
                ffmpeg_path,
                ffmpeg_source,
                js_runtime_installed,
                js_runtime_name,
                js_runtime_path,
                js_runtime_version,
                js_runtime_latest_version,
                js_runtime_update_available,
                js_runtime_source,
                release_url: url,
                checked_at,
                error: None,
            }
        }
        Err(err) => EngineStatus {
            installed,
            current_version,
            latest_version: None,
            update_available: false,
            binary_path: bin.map(|p| p.to_string_lossy().to_string()),
            ffmpeg_installed,
            ffmpeg_version,
            ffmpeg_latest_version,
            ffmpeg_update_available,
            ffmpeg_path,
            ffmpeg_source,
            js_runtime_installed,
            js_runtime_name,
            js_runtime_path,
            js_runtime_version,
            js_runtime_latest_version,
            js_runtime_update_available,
            js_runtime_source,
            release_url: "https://github.com/yt-dlp/yt-dlp/releases/latest".to_string(),
            checked_at,
            error: Some(err),
        },
    }
}

async fn download_url_to_file(url: &str, dest: &Path, timeout_secs: u64) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("VideoDownloader/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| e.to_string())?;

    let mut resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Download failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("GitHub download error: HTTP {}", resp.status()));
    }

    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| format!("Failed to create temporary file: {}", e))?;

    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| format!("Download stream error: {}", e))?
    {
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("Disk write error: {}", e))?;
    }

    file.flush()
        .await
        .map_err(|e| format!("Disk flush error: {}", e))?;
    Ok(())
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path)
        .map_err(|e| e.to_string())?
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

pub async fn update_or_install_ytdlp() -> Result<EngineStatus, String> {
    let _guard = ytdlp_lock().lock().await;
    let bin_dir = config::bin_dir();
    fs::create_dir_all(&bin_dir)
        .map_err(|e| format!("Failed to create binary directory: {}", e))?;

    let asset_name = if cfg!(target_os = "windows") {
        if cfg!(target_arch = "aarch64") {
            "yt-dlp_arm64.exe"
        } else {
            "yt-dlp.exe"
        }
    } else if cfg!(target_os = "macos") {
        "yt-dlp_macos"
    } else if cfg!(target_arch = "aarch64") {
        "yt-dlp_linux_aarch64"
    } else {
        "yt-dlp_linux"
    };
    let target_name = ytdlp_bin_name();

    let download_url = format!(
        "https://github.com/yt-dlp/yt-dlp/releases/latest/download/{}",
        asset_name
    );

    let dest_path = bin_dir.join(target_name);
    let tmp_path = bin_dir.join(format!("{}.tmp", target_name));
    if let Err(e) = download_url_to_file(&download_url, &tmp_path, 120).await {
        let _ = fs::remove_file(&tmp_path);
        return Err(e);
    }

    if let Err(e) = make_executable(&tmp_path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e);
    }
    if let Err(e) = fs::rename(&tmp_path, &dest_path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(format!("Rename failed (close active downloads and retry): {}", e));
    }
    YTDLP_HAS_JS_RUNTIMES.store(0, std::sync::atomic::Ordering::Relaxed);
    YTDLP_HAS_REMOTE_COMPONENTS.store(0, std::sync::atomic::Ordering::Relaxed);
    YTDLP_HAS_IMPERSONATE.store(0, std::sync::atomic::Ordering::Relaxed);

    Ok(check_status().await)
}

#[cfg(target_os = "windows")]
async fn extract_ffmpeg_archive(archive_path: &Path, bin_dir: &Path) -> Result<(), String> {
    let archive_owned = archive_path.to_path_buf();
    let bin_owned = bin_dir.to_path_buf();

    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let file = fs::File::open(&archive_owned)
            .map_err(|e| format!("Failed to open ffmpeg zip archive: {}", e))?;
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|e| format!("Invalid ffmpeg zip archive: {}", e))?;

        let mut extracted_ffmpeg = false;
        for idx in 0..archive.len() {
            let mut entry = archive
                .by_index(idx)
                .map_err(|e| format!("Zip read error: {}", e))?;
            if entry.is_dir() {
                continue;
            }
            let normalized = entry.name().replace('\\', "/");
            let target_name = if normalized.ends_with("/bin/ffmpeg.exe") || normalized == "ffmpeg.exe" {
                Some("ffmpeg.exe")
            } else if normalized.ends_with("/bin/ffprobe.exe") || normalized == "ffprobe.exe" {
                Some("ffprobe.exe")
            } else {
                None
            };

            if let Some(name) = target_name {
                let tmp_out = bin_owned.join(format!("{}.tmp", name));
                let final_out = bin_owned.join(name);
                {
                    let mut out = fs::File::create(&tmp_out)
                        .map_err(|e| format!("Failed to extract {}: {}", name, e))?;
                    std::io::copy(&mut entry, &mut out)
                        .map_err(|e| format!("Failed to copy {}: {}", name, e))?;
                }
                if let Err(e) = fs::rename(&tmp_out, &final_out) {
                    let _ = fs::remove_file(&tmp_out);
                    return Err(format!("Failed to place {}: {}", name, e));
                }
                if name == "ffmpeg.exe" {
                    extracted_ffmpeg = true;
                }
            }
        }

        if !extracted_ffmpeg {
            return Err("ffmpeg.exe was not found inside the downloaded archive.".to_string());
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("Extraction task error: {}", e))?
}

#[cfg(target_os = "linux")]
async fn extract_ffmpeg_archive(archive_path: &Path, bin_dir: &Path) -> Result<(), String> {
    let extract_dir = bin_dir.join("ffmpeg-extract-tmp");
    let _ = fs::remove_dir_all(&extract_dir);
    fs::create_dir_all(&extract_dir)
        .map_err(|e| format!("Failed to create temp extract dir: {}", e))?;

    let status = Command::new("tar")
        .arg("-xJf")
        .arg(archive_path)
        .arg("-C")
        .arg(&extract_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .await
        .map_err(|e| format!("Failed to invoke tar for ffmpeg extraction: {}", e))?;

    if !status.success() {
        let _ = fs::remove_dir_all(&extract_dir);
        return Err("Failed to extract ffmpeg .tar.xz archive.".to_string());
    }

    let mut found_ffmpeg = false;
    fn find_binaries(dir: &Path, out: &mut Vec<(String, PathBuf)>) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    find_binaries(&p, out);
                } else if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                    if name == "ffmpeg" || name == "ffprobe" {
                        out.push((name.to_string(), p));
                    }
                }
            }
        }
    }

    let mut found = Vec::new();
    find_binaries(&extract_dir, &mut found);

    for (name, src) in found {
        let tmp_out = bin_dir.join(format!("{}.tmp", name));
        let final_out = bin_dir.join(&name);
        fs::copy(&src, &tmp_out).map_err(|e| format!("Failed to copy {}: {}", name, e))?;
        if let Err(e) = make_executable(&tmp_out) {
            let _ = fs::remove_file(&tmp_out);
            let _ = fs::remove_dir_all(&extract_dir);
            return Err(e);
        }
        if let Err(e) = fs::rename(&tmp_out, &final_out) {
            let _ = fs::remove_file(&tmp_out);
            let _ = fs::remove_dir_all(&extract_dir);
            return Err(format!("Failed to place {}: {}", name, e));
        }
        if name == "ffmpeg" {
            found_ffmpeg = true;
        }
    }

    let _ = fs::remove_dir_all(&extract_dir);

    if !found_ffmpeg {
        return Err("ffmpeg binary was not found inside the downloaded archive.".to_string());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
async fn extract_ffmpeg_archive(archive_path: &Path, bin_dir: &Path) -> Result<(), String> {
    let extract_dir = bin_dir.join("ffmpeg-extract-tmp");
    let _ = fs::remove_dir_all(&extract_dir);
    fs::create_dir_all(&extract_dir)
        .map_err(|e| format!("Failed to create temp extract dir: {}", e))?;

    let status = Command::new("unzip")
        .arg("-o")
        .arg(archive_path)
        .arg("-d")
        .arg(&extract_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map_err(|e| format!("Failed to run unzip: {}", e))?;

    if !status.success() {
        let _ = fs::remove_dir_all(&extract_dir);
        return Err("Failed to extract ffmpeg archive.".to_string());
    }

    let src = extract_dir.join("ffmpeg");
    if !src.exists() {
        let _ = fs::remove_dir_all(&extract_dir);
        return Err("ffmpeg binary not found in archive.".to_string());
    }
    let tmp_out = bin_dir.join("ffmpeg.tmp");
    let final_out = bin_dir.join("ffmpeg");
    if let Err(e) = fs::copy(&src, &tmp_out) {
        let _ = fs::remove_file(&tmp_out);
        let _ = fs::remove_dir_all(&extract_dir);
        return Err(format!("Failed to copy ffmpeg: {}", e));
    }
    if let Err(e) = make_executable(&tmp_out) {
        let _ = fs::remove_file(&tmp_out);
        let _ = fs::remove_dir_all(&extract_dir);
        return Err(e);
    }
    if let Err(e) = fs::rename(&tmp_out, &final_out) {
        let _ = fs::remove_file(&tmp_out);
        let _ = fs::remove_dir_all(&extract_dir);
        return Err(format!("Failed to place ffmpeg: {}", e));
    }
    let _ = fs::remove_dir_all(&extract_dir);
    Ok(())
}

pub async fn update_or_install_ffmpeg() -> Result<EngineStatus, String> {
    let _guard = ffmpeg_lock().lock().await;
    let bin_dir = config::bin_dir();
    fs::create_dir_all(&bin_dir)
        .map_err(|e| format!("Failed to create binary directory: {}", e))?;

    let latest_meta = fetch_latest_ffmpeg_release().await.unwrap_or_else(|_| FfmpegVersionMeta {
        version: chrono::Utc::now().format("%Y-%m-%d").to_string(),
        published_at: chrono::Utc::now().to_rfc3339(),
    });

    #[cfg(target_os = "windows")]
    let download_url = if cfg!(target_arch = "aarch64") {
        "https://github.com/yt-dlp/FFmpeg-Builds/releases/latest/download/ffmpeg-master-latest-winarm64-gpl.zip"
    } else {
        "https://github.com/yt-dlp/FFmpeg-Builds/releases/latest/download/ffmpeg-master-latest-win64-gpl.zip"
    };

    #[cfg(target_os = "linux")]
    let download_url = if cfg!(target_arch = "aarch64") {
        "https://github.com/yt-dlp/FFmpeg-Builds/releases/latest/download/ffmpeg-master-latest-linuxarm64-gpl.tar.xz"
    } else {
        "https://github.com/yt-dlp/FFmpeg-Builds/releases/latest/download/ffmpeg-master-latest-linux64-gpl.tar.xz"
    };

    #[cfg(target_os = "macos")]
    let download_url = "https://evermeet.cx/ffmpeg/getrelease/ffmpeg/zip";

    let archive_ext = if cfg!(target_os = "linux") { "tar.xz" } else { "zip" };
    let tmp_archive = bin_dir.join(format!("ffmpeg-download.{}", archive_ext));

    if let Err(e) = download_url_to_file(download_url, &tmp_archive, 300).await {
        let _ = fs::remove_file(&tmp_archive);
        return Err(e);
    }

    let extract_res = extract_ffmpeg_archive(&tmp_archive, &bin_dir).await;
    let _ = fs::remove_file(&tmp_archive);
    extract_res?;

    #[cfg(target_os = "macos")]
    {
        let tmp_probe = bin_dir.join("ffprobe-download.zip");
        if download_url_to_file("https://evermeet.cx/ffmpeg/getrelease/ffprobe/zip", &tmp_probe, 120)
            .await
            .is_ok()
        {
            let extract_dir = bin_dir.join("ffprobe-extract-tmp");
            let _ = fs::create_dir_all(&extract_dir);
            if Command::new("unzip")
                .args(["-o", &tmp_probe.to_string_lossy(), "-d", &extract_dir.to_string_lossy()])
                .status()
                .await
                .map(|s| s.success())
                .unwrap_or(false)
            {
                let src_probe = extract_dir.join(ffprobe_bin_name());
                if src_probe.exists() {
                    let dst_probe = bin_dir.join(ffprobe_bin_name());
                    let _ = fs::copy(&src_probe, &dst_probe);
                    let _ = make_executable(&dst_probe);
                }
            }
            let _ = fs::remove_dir_all(&extract_dir);
        }
        let _ = fs::remove_file(&tmp_probe);
    }

    if let Ok(json) = serde_json::to_string_pretty(&latest_meta) {
        let _ = fs::write(ffmpeg_meta_path(), json);
    }

    Ok(check_status().await)
}

async fn extract_deno_archive(archive_path: &Path, bin_dir: &Path) -> Result<(), String> {
    let archive_owned = archive_path.to_path_buf();
    let bin_owned = bin_dir.to_path_buf();
    let target_name = deno_bin_name();

    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let file = fs::File::open(&archive_owned)
            .map_err(|e| format!("Failed to open deno zip archive: {}", e))?;
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|e| format!("Invalid deno zip archive: {}", e))?;

        for idx in 0..archive.len() {
            let mut entry = archive
                .by_index(idx)
                .map_err(|e| format!("Zip read error: {}", e))?;
            if entry.is_dir() {
                continue;
            }
            let normalized = entry.name().replace('\\', "/");
            if normalized == target_name || normalized.ends_with(&format!("/{}", target_name)) {
                let tmp_out = bin_owned.join(format!("{}.tmp", target_name));
                let final_out = bin_owned.join(target_name);
                {
                    let mut out = fs::File::create(&tmp_out)
                        .map_err(|e| format!("Failed to extract {}: {}", target_name, e))?;
                    std::io::copy(&mut entry, &mut out)
                        .map_err(|e| format!("Failed to copy {}: {}", target_name, e))?;
                }
                if let Err(e) = make_executable(&tmp_out) {
                    let _ = fs::remove_file(&tmp_out);
                    return Err(e);
                }
                if let Err(e) = fs::rename(&tmp_out, &final_out) {
                    let _ = fs::remove_file(&tmp_out);
                    return Err(format!("Failed to place {}: {}", target_name, e));
                }
                return Ok(());
            }
        }

        Err(format!(
            "{} was not found inside the downloaded archive.",
            target_name
        ))
    })
    .await
    .map_err(|e| format!("Extraction task error: {}", e))?
}

async fn download_and_install_deno_binary() -> Result<PathBuf, String> {
    let _guard = deno_lock().lock().await;

    let bin_dir = config::bin_dir();
    fs::create_dir_all(&bin_dir)
        .map_err(|e| format!("Failed to create binary directory: {}", e))?;

    #[cfg(target_os = "windows")]
    let download_url =
        "https://github.com/denoland/deno/releases/latest/download/deno-x86_64-pc-windows-msvc.zip";

    #[cfg(target_os = "linux")]
    let download_url = if cfg!(target_arch = "aarch64") {
        "https://github.com/denoland/deno/releases/latest/download/deno-aarch64-unknown-linux-gnu.zip"
    } else {
        "https://github.com/denoland/deno/releases/latest/download/deno-x86_64-unknown-linux-gnu.zip"
    };

    #[cfg(target_os = "macos")]
    let download_url = if cfg!(target_arch = "aarch64") {
        "https://github.com/denoland/deno/releases/latest/download/deno-aarch64-apple-darwin.zip"
    } else {
        "https://github.com/denoland/deno/releases/latest/download/deno-x86_64-apple-darwin.zip"
    };

    let tmp_archive = bin_dir.join("deno-download.zip");
    if let Err(e) = download_url_to_file(download_url, &tmp_archive, 180).await {
        let _ = fs::remove_file(&tmp_archive);
        return Err(e);
    }

    let extract_res = extract_deno_archive(&tmp_archive, &bin_dir).await;
    let _ = fs::remove_file(&tmp_archive);
    extract_res?;

    resolve_deno_binary().ok_or_else(|| "Failed to install deno automatically.".to_string())
}

pub async fn update_or_install_deno() -> Result<EngineStatus, String> {
    download_and_install_deno_binary().await?;
    Ok(check_status().await)
}

async fn install_deno_if_missing() -> Result<PathBuf, String> {
    if let Some((_, existing, _)) = resolve_any_js_runtime() {
        return Ok(existing);
    }
    download_and_install_deno_binary().await
}

pub async fn ensure_js_runtime_if_needed() -> Result<PathBuf, String> {
    if let Some((_, path, _)) = resolve_any_js_runtime() {
        return Ok(path);
    }
    install_deno_if_missing().await
}

pub async fn ensure_ytdlp_binary() -> Result<PathBuf, String> {
    if let Some(bin) = resolve_ytdlp_binary() {
        return Ok(bin);
    }
    update_or_install_ytdlp().await?;
    resolve_ytdlp_binary().ok_or_else(|| "Failed to install yt-dlp automatically.".to_string())
}

pub async fn ensure_ytdlp_with_impersonation() -> Result<PathBuf, String> {
    let bin = ensure_ytdlp_binary().await?;
    if ytdlp_supports_impersonation(&bin).await {
        return Ok(bin);
    }

    let local_bin = config::bin_dir().join(ytdlp_bin_name());
    if bin != local_bin || !YTDLP_IMPERSONATE_UPGRADE_TRIED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        update_or_install_ytdlp().await?;
        let upgraded = resolve_ytdlp_binary()
            .ok_or_else(|| "Failed to install yt-dlp automatically.".to_string())?;
        let _ = ytdlp_supports_impersonation(&upgraded).await;
        return Ok(upgraded);
    }

    Ok(bin)
}

pub async fn ensure_ffmpeg_location() -> Result<Option<PathBuf>, String> {
    if let Some(dir) = resolve_ffmpeg_location() {
        return Ok(Some(dir));
    }
    update_or_install_ffmpeg().await?;
    Ok(resolve_ffmpeg_location())
}

pub async fn ensure_engines() -> Result<EngineStatus, String> {
    if resolve_ytdlp_binary().is_none() {
        update_or_install_ytdlp().await?;
    }
    if resolve_ffmpeg_binary().is_none() {
        update_or_install_ffmpeg().await?;
    }
    if resolve_any_js_runtime().is_none() {
        let _ = install_deno_if_missing().await;
    }
    Ok(check_status().await)
}
