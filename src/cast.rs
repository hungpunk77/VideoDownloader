use crate::downloader::{append_cookie_args, decode_ytdlp_bytes, format_ytdlp_error, UiEvent};
use crate::engines;
use crate::types::{
    is_bandcamp_url, CastDevice, CastMediaItem, CastPlayerStatus, CastSessionState,
};
use eframe::egui;
use mdns_sd::{ServiceDaemon, ServiceEvent};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::WebPkiSupportedAlgorithms;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, Error as RustlsError, SignatureScheme};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::Command;
use tokio::sync::{mpsc, watch};
use tokio_rustls::TlsConnector;
use uuid::Uuid;

const GOOGLE_CAST_SERVICE: &str = "_googlecast._tcp.local.";
const DEFAULT_MEDIA_RECEIVER_APP_ID: &str = "CC1AD845";

const NS_CONNECTION: &str = "urn:x-cast:com.google.cast.tp.connection";
const NS_HEARTBEAT: &str = "urn:x-cast:com.google.cast.tp.heartbeat";
const NS_RECEIVER: &str = "urn:x-cast:com.google.cast.receiver";
const NS_MEDIA: &str = "urn:x-cast:com.google.cast.media";

#[derive(Debug, Clone)]
pub enum CastCommand {
    Play,
    Pause,
    Seek(f64),
    SetVolume(f32),
    SetMuted(bool),
    Stop,
}

#[derive(Clone)]
enum CastStreamPlan {
    Proxy {
        url: String,
        content_type: String,
        headers: HashMap<String, String>,
        seekable: bool,
    },
    FfmpegMux {
        ffmpeg_bin: PathBuf,
        video_url: String,
        audio_url: String,
        needs_aac_transcode: bool,
        headers: HashMap<String, String>,
    },
    FfmpegAudio {
        ffmpeg_bin: PathBuf,
        audio_url: String,
        headers: HashMap<String, String>,
    },
}

impl CastStreamPlan {
    fn content_type(&self) -> &str {
        match self {
            Self::Proxy { content_type, .. } => content_type,
            Self::FfmpegMux { .. } => "video/mp4",
            Self::FfmpegAudio { .. } => "audio/mpeg",
        }
    }

    fn seekable(&self) -> bool {
        match self {
            Self::Proxy { seekable, .. } => *seekable,
            Self::FfmpegMux { .. } | Self::FfmpegAudio { .. } => false,
        }
    }
}

struct NoCertVerifier {
    algs: WebPkiSupportedAlgorithms,
}

impl std::fmt::Debug for NoCertVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NoCertVerifier").finish()
    }
}

impl ServerCertVerifier for NoCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algs)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

fn build_cast_tls_connector() -> Result<TlsConnector, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let algs = provider.signature_verification_algorithms;
    let cfg = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("TLS config error: {}", e))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoCertVerifier { algs }))
        .with_no_client_auth();
    Ok(TlsConnector::from(Arc::new(cfg)))
}

fn unescape_mdns_instance(fullname: &str) -> String {
    let base = fullname
        .strip_suffix(GOOGLE_CAST_SERVICE)
        .map(|s| s.trim_end_matches('.'))
        .unwrap_or(fullname);
    base.replace("\\.", ".").replace("\\\\", "\\")
}

pub async fn discover_devices(tx: mpsc::UnboundedSender<UiEvent>, ctx: egui::Context) {
    let daemon = match ServiceDaemon::new() {
        Ok(d) => d,
        Err(e) => {
            let _ = tx.send(UiEvent::CastDiscoveryDone(Some(format!(
                "mDNS discovery error: {}",
                e
            ))));
            ctx.request_repaint();
            return;
        }
    };

    let receiver = match daemon.browse(GOOGLE_CAST_SERVICE) {
        Ok(r) => r,
        Err(e) => {
            let _ = daemon.shutdown();
            let _ = tx.send(UiEvent::CastDiscoveryDone(Some(format!(
                "mDNS browse error: {}",
                e
            ))));
            ctx.request_repaint();
            return;
        }
    };

    let mut devices: BTreeMap<String, CastDevice> = BTreeMap::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(4);

    loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            break;
        }
        let rem = deadline - now;
        match tokio::time::timeout(rem, receiver.recv_async()).await {
            Ok(Ok(ServiceEvent::ServiceResolved(info))) => {
                let addrs = info.get_addresses_v4();
                let picked_ip: Option<Ipv4Addr> = addrs
                    .iter()
                    .copied()
                    .find(|ip| !ip.is_loopback() && !ip.is_link_local())
                    .or_else(|| addrs.iter().copied().next());
                let Some(ip) = picked_ip else {
                    continue;
                };

                let fullname = info.get_fullname().to_string();
                let name = info
                    .get_property_val_str("fn")
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| {
                        let fallback = unescape_mdns_instance(&fullname);
                        if fallback.is_empty() {
                            ip.to_string()
                        } else {
                            fallback
                        }
                    });
                let model = info
                    .get_property_val_str("md")
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("Chromecast / Android TV")
                    .to_string();
                let port = if info.get_port() == 0 {
                    8009
                } else {
                    info.get_port()
                };

                let dev = CastDevice {
                    id: fullname.clone(),
                    name,
                    model,
                    host: ip.to_string(),
                    port,
                };
                devices.insert(fullname, dev);

                let mut list: Vec<CastDevice> = devices.values().cloned().collect();
                list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                let _ = tx.send(UiEvent::CastDevicesUpdated(list));
                ctx.request_repaint();
            }
            Ok(Ok(ServiceEvent::ServiceRemoved(_, fullname))) => {
                if devices.remove(&fullname).is_some() {
                    let mut list: Vec<CastDevice> = devices.values().cloned().collect();
                    list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                    let _ = tx.send(UiEvent::CastDevicesUpdated(list));
                    ctx.request_repaint();
                }
            }
            Ok(Ok(_)) => {}
            Ok(Err(_)) | Err(_) => break,
        }
    }

    let _ = daemon.stop_browse(GOOGLE_CAST_SERVICE);
    let _ = daemon.shutdown();
    let _ = tx.send(UiEvent::CastDiscoveryDone(None));
    ctx.request_repaint();
}

async fn run_ytdlp_json(
    bin: &PathBuf,
    ejs_args: &[String],
    url: &str,
    single_playlist_item: bool,
) -> Result<Value, String> {
    let mut args = vec![
        "--encoding".to_string(),
        "utf-8".to_string(),
        "--dump-single-json".to_string(),
        "--no-warnings".to_string(),
        "--skip-download".to_string(),
    ];
    if single_playlist_item {
        args.push("--playlist-items".to_string());
        args.push("1".to_string());
    } else {
        args.push("--no-playlist".to_string());
    }
    args.extend(ejs_args.iter().cloned());
    append_cookie_args(&mut args);
    args.push(url.to_string());

    let mut cmd = Command::new(bin);
    engines::configure_ytdlp_command(&mut cmd);
    cmd.args(&args).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000);

    let out = tokio::time::timeout(Duration::from_secs(35), cmd.output())
        .await
        .map_err(|_| "Timed out while extracting stream URL.".to_string())?
        .map_err(|e| format!("Failed to start yt-dlp: {}", e))?;

    if !out.status.success() {
        let err = decode_ytdlp_bytes(&out.stderr);
        return Err(format_ytdlp_error(&err));
    }

    serde_json::from_slice(&out.stdout).map_err(|e| format!("Metadata parse error: {}", e))
}

fn extract_headers_map(val: &Value, fallback: Option<&Value>) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for source in [fallback, Some(val)].into_iter().flatten() {
        if let Some(obj) = source.get("http_headers").and_then(|h| h.as_object()) {
            for (k, v) in obj {
                if let Some(s) = v.as_str() {
                    out.insert(k.clone(), s.to_string());
                }
            }
        }
    }
    out
}

fn is_hls_protocol(fmt: &Value) -> bool {
    let proto = fmt
        .get("protocol")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    proto.contains("m3u8")
}

fn is_dash_segment_protocol(fmt: &Value) -> bool {
    let proto = fmt
        .get("protocol")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    proto.contains("dash")
}

fn audio_content_type(ext: &str) -> &'static str {
    match ext {
        "mp3" => "audio/mpeg",
        "aac" => "audio/aac",
        "ogg" | "oga" => "audio/ogg",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        _ => "audio/mp4",
    }
}

async fn resolve_cast_stream(item: &CastMediaItem) -> Result<CastStreamPlan, String> {
    let mut bin = if is_bandcamp_url(&item.url) {
        engines::ensure_ytdlp_with_impersonation().await?
    } else {
        engines::ensure_ytdlp_binary().await?
    };
    let _ = engines::ensure_js_runtime_if_needed().await;
    let ejs_args = engines::ytdlp_ejs_args(&bin).await;

    let mut val = match run_ytdlp_json(&bin, &ejs_args, &item.url, true).await {
        Ok(v) => v,
        Err(e)
            if e.contains("no impersonate target") || e.contains("attempting impersonation") =>
        {
            if let Ok(upgraded) = engines::ensure_ytdlp_with_impersonation().await {
                bin = upgraded;
            }
            let ejs_retry = engines::ytdlp_ejs_args(&bin).await;
            run_ytdlp_json(&bin, &ejs_retry, &item.url, true).await?
        }
        Err(e) => return Err(e),
    };

    // If playlist/album, unwrap first entry
    if let Some(first_entry) = val
        .get("entries")
        .and_then(|e| e.as_array())
        .and_then(|arr| arr.first())
        .cloned()
    {
        let has_formats = first_entry
            .get("formats")
            .and_then(|f| f.as_array())
            .is_some_and(|a| !a.is_empty());
        let has_url = first_entry
            .get("url")
            .and_then(|u| u.as_str())
            .is_some_and(|s| s.starts_with("http://") || s.starts_with("https://"));

        if has_formats {
            val = first_entry;
        } else if let Some(entry_url) = first_entry
            .get("webpage_url")
            .or_else(|| first_entry.get("url"))
            .and_then(|v| v.as_str())
            .filter(|s| s.starts_with("http://") || s.starts_with("https://"))
        {
            val = run_ytdlp_json(&bin, &ejs_args, entry_url, false).await?;
        } else if has_url {
            val = first_entry;
        }
    }

    let formats = val
        .get("formats")
        .and_then(|f| f.as_array())
        .cloned()
        .unwrap_or_default();

    let ffmpeg_bin = engines::resolve_ffmpeg_binary().map(|(p, _)| p);

    if item.is_audio {
        let mut best_direct_audio: Option<(f64, String, String, HashMap<String, String>)> = None;
        let mut best_any_audio: Option<(f64, String, HashMap<String, String>)> = None;

        for fmt in &formats {
            let Some(u) = fmt
                .get("url")
                .and_then(|v| v.as_str())
                .filter(|s| s.starts_with("http"))
            else {
                continue;
            };
            if is_hls_protocol(fmt) || is_dash_segment_protocol(fmt) {
                continue;
            }
            let ext = fmt
                .get("ext")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let vcodec = fmt
                .get("vcodec")
                .and_then(|v| v.as_str())
                .unwrap_or("none")
                .to_ascii_lowercase();
            let acodec = fmt
                .get("acodec")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if vcodec != "none" && !vcodec.is_empty() {
                continue;
            }
            if acodec == "none" {
                continue;
            }
            let score = fmt
                .get("abr")
                .and_then(|v| v.as_f64())
                .or_else(|| fmt.get("tbr").and_then(|v| v.as_f64()))
                .unwrap_or(128.0);
            let headers = extract_headers_map(fmt, Some(&val));

            if matches!(ext.as_str(), "mp3" | "m4a" | "aac" | "mp4" | "ogg" | "wav") {
                let ctype = audio_content_type(&ext).to_string();
                if best_direct_audio
                    .as_ref()
                    .is_none_or(|(best_s, _, _, _)| score > *best_s)
                {
                    best_direct_audio = Some((score, u.to_string(), ctype, headers.clone()));
                }
            }
            if best_any_audio
                .as_ref()
                .is_none_or(|(best_s, _, _)| score > *best_s)
            {
                best_any_audio = Some((score, u.to_string(), headers));
            }
        }

        if let Some((_, url, content_type, headers)) = best_direct_audio {
            return Ok(CastStreamPlan::Proxy {
                url,
                content_type,
                headers,
                seekable: true,
            });
        }
        if let (Some(ff), Some((_, url, headers))) = (ffmpeg_bin.clone(), best_any_audio) {
            return Ok(CastStreamPlan::FfmpegAudio {
                ffmpeg_bin: ff,
                audio_url: url,
                headers,
            });
        }
    }

    // Video selection:
    // 1. Direct muxed progressive MP4 (prefer H.264 / >=720p)
    let mut best_muxed_mp4: Option<(u32, f64, String, HashMap<String, String>)> = None;
    let mut best_h264_video: Option<(u32, f64, String, HashMap<String, String>)> = None;
    let mut best_aac_audio: Option<(f64, String)> = None;
    let mut best_any_audio: Option<(f64, bool, String)> = None;

    for fmt in &formats {
        let Some(u) = fmt
            .get("url")
            .and_then(|v| v.as_str())
            .filter(|s| s.starts_with("http"))
        else {
            continue;
        };
        if is_hls_protocol(fmt) || is_dash_segment_protocol(fmt) {
            continue;
        }
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
        let height = fmt
            .get("height")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let tbr = fmt
            .get("tbr")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        let has_video = !vcodec.is_empty() && vcodec != "none";
        let has_audio = !acodec.is_empty() && acodec != "none";
        let is_h264 = vcodec.starts_with("avc1") || vcodec.contains("h264");

        if has_video && has_audio && ext == "mp4" {
            let bonus = if is_h264 { 10.0 } else { 0.0 };
            let headers = extract_headers_map(fmt, Some(&val));
            if best_muxed_mp4
                .as_ref()
                .is_none_or(|(bh, bt, _, _)| (height, tbr + bonus) > (*bh, *bt))
            {
                best_muxed_mp4 = Some((height, tbr + bonus, u.to_string(), headers));
            }
        } else if has_video && !has_audio && is_h264 && height <= 1080 {
            let headers = extract_headers_map(fmt, Some(&val));
            if best_h264_video
                .as_ref()
                .is_none_or(|(bh, bt, _, _)| (height, tbr) > (*bh, *bt))
            {
                best_h264_video = Some((height, tbr, u.to_string(), headers));
            }
        } else if !has_video && has_audio {
            let abr = fmt
                .get("abr")
                .and_then(|v| v.as_f64())
                .unwrap_or(tbr);
            let is_aac = acodec.starts_with("mp4a") || acodec == "aac" || ext == "m4a";
            if is_aac
                && best_aac_audio
                    .as_ref()
                    .is_none_or(|(ba, _)| abr > *ba)
            {
                best_aac_audio = Some((abr, u.to_string()));
            }
            if best_any_audio
                .as_ref()
                .is_none_or(|(ba, _, _)| abr > *ba)
            {
                best_any_audio = Some((abr, !is_aac, u.to_string()));
            }
        }
    }

    // Prefer native 720p+ progressive MP4 (instant start, full TV seekbar, zero CPU)
    if let Some((h, _, url, headers)) = &best_muxed_mp4 {
        if *h >= 720 {
            return Ok(CastStreamPlan::Proxy {
                url: url.clone(),
                content_type: "video/mp4".to_string(),
                headers: headers.clone(),
                seekable: true,
            });
        }
    }

    // Upgrade YouTube-style 360p-muxed streams to 720p/1080p via zero-reencode FFmpeg fragmented MP4 mux
    if let (Some(ff), Some((vh, _, v_url, headers))) = (ffmpeg_bin.clone(), best_h264_video) {
        let max_muxed_h = best_muxed_mp4.as_ref().map(|(h, _, _, _)| *h).unwrap_or(0);
        if vh > max_muxed_h {
            if let Some((_, a_url)) = best_aac_audio {
                return Ok(CastStreamPlan::FfmpegMux {
                    ffmpeg_bin: ff,
                    video_url: v_url,
                    audio_url: a_url,
                    needs_aac_transcode: false,
                    headers,
                });
            }
            if let Some((_, needs_aac, a_url)) = best_any_audio {
                return Ok(CastStreamPlan::FfmpegMux {
                    ffmpeg_bin: ff,
                    video_url: v_url,
                    audio_url: a_url,
                    needs_aac_transcode: needs_aac,
                    headers,
                });
            }
        }
    }

    if let Some((_, _, url, headers)) = best_muxed_mp4 {
        return Ok(CastStreamPlan::Proxy {
            url,
            content_type: "video/mp4".to_string(),
            headers,
            seekable: true,
        });
    }

    // Fallback to top-level URL (or HLS format)
    if let Some(u) = val
        .get("url")
        .and_then(|v| v.as_str())
        .filter(|s| s.starts_with("http"))
    {
        let ext = val
            .get("ext")
            .and_then(|v| v.as_str())
            .unwrap_or("mp4")
            .to_ascii_lowercase();
        let ctype = if item.is_audio {
            audio_content_type(&ext).to_string()
        } else if ext == "webm" {
            "video/webm".to_string()
        } else {
            "video/mp4".to_string()
        };
        let headers = extract_headers_map(&val, None);
        return Ok(CastStreamPlan::Proxy {
            url: u.to_string(),
            content_type: ctype,
            headers,
            seekable: true,
        });
    }

    // Last resort: HLS m3u8 stream
    for fmt in &formats {
        if let Some(u) = fmt
            .get("url")
            .and_then(|v| v.as_str())
            .filter(|s| s.starts_with("http"))
        {
            if is_hls_protocol(fmt) {
                let headers = extract_headers_map(fmt, Some(&val));
                return Ok(CastStreamPlan::Proxy {
                    url: u.to_string(),
                    content_type: "application/x-mpegURL".to_string(),
                    headers,
                    seekable: false,
                });
            }
        }
    }

    Err("No Chromecast-compatible media stream was found for this item.".to_string())
}

fn resolve_target_socket_addr(host: &str, port: u16) -> Result<SocketAddr, String> {
    let clean_host = host.trim().trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = clean_host.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, port));
    }
    let addrs: Vec<SocketAddr> = (clean_host, port)
        .to_socket_addrs()
        .map_err(|_| format!("Invalid target IP or host: {}", host))?
        .collect();
    if let Some(v4) = addrs.iter().copied().find(|a| a.is_ipv4()) {
        Ok(v4)
    } else {
        addrs
            .into_iter()
            .next()
            .ok_or_else(|| format!("Invalid target IP or host: {}", host))
    }
}

fn local_ip_for_target(target_addr: SocketAddr) -> Result<IpAddr, String> {
    let bind_addr = if target_addr.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let sock = std::net::UdpSocket::bind(bind_addr)
        .map_err(|e| format!("Failed to determine local LAN IP: {}", e))?;
    sock.connect(target_addr)
        .map_err(|e| format!("Network route to TV {} unreachable: {}", target_addr, e))?;
    let local = sock
        .local_addr()
        .map_err(|e| format!("Failed to read local LAN IP: {}", e))?;
    Ok(local.ip())
}

async fn handle_http_client(
    mut stream: TcpStream,
    expected_path: String,
    plan: CastStreamPlan,
    http_client: reqwest::Client,
) {
    let mut reader = BufReader::new(&mut stream);
    let mut request_line = String::new();
    if tokio::time::timeout(Duration::from_secs(5), reader.read_line(&mut request_line))
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or(0)
        == 0
    {
        return;
    }

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let raw_path = parts.next().unwrap_or("");
    let path = raw_path.split('?').next().unwrap_or("");

    let mut range_header: Option<String> = None;
    for _ in 0..64 {
        let mut line = String::new();
        match tokio::time::timeout(Duration::from_secs(2), reader.read_line(&mut line)).await {
            Ok(Ok(0)) | Err(_) | Ok(Err(_)) => break,
            Ok(Ok(_)) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    break;
                }
                if let Some((k, v)) = trimmed.split_once(':') {
                    if k.trim().eq_ignore_ascii_case("range") {
                        let val = v.trim();
                        if !val.is_empty() {
                            range_header = Some(val.to_string());
                        }
                    }
                }
            }
        }
    }

    if path != expected_path {
        let _ = stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await;
        return;
    }

    if method.eq_ignore_ascii_case("OPTIONS") {
        let _ = stream
            .write_all(
                b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, HEAD, OPTIONS\r\nAccess-Control-Allow-Headers: Range, Content-Type\r\nConnection: close\r\n\r\n",
            )
            .await;
        return;
    }

    if !method.eq_ignore_ascii_case("GET") && !method.eq_ignore_ascii_case("HEAD") {
        let _ = stream
            .write_all(
                b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .await;
        return;
    }

    let is_head = method.eq_ignore_ascii_case("HEAD");

    match plan {
        CastStreamPlan::Proxy {
            url,
            content_type,
            headers,
            ..
        } => {
            let mut req = if is_head {
                http_client.head(&url)
            } else {
                http_client.get(&url)
            };
            for (k, v) in &headers {
                if !k.eq_ignore_ascii_case("host")
                    && !k.eq_ignore_ascii_case("range")
                    && !k.eq_ignore_ascii_case("accept-encoding")
                {
                    req = req.header(k.as_str(), v.as_str());
                }
            }
            if !is_head {
                if let Some(r) = range_header {
                    req = req.header("Range", r);
                }
            }

            let Ok(mut resp) = req.send().await else {
                let _ = stream
                    .write_all(
                        b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .await;
                return;
            };

            let status = resp.status();
            let status_code = status.as_u16();
            let reason = status.canonical_reason().unwrap_or("OK");

            let upstream_ct = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .filter(|s| !s.is_empty() && *s != "application/octet-stream")
                .map(ToString::to_string)
                .unwrap_or(content_type);

            let mut head = format!(
                "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Expose-Headers: Content-Length, Content-Range, Accept-Ranges\r\nAccept-Ranges: bytes\r\nConnection: close\r\n",
                status_code, reason, upstream_ct
            );

            if let Some(cl) = resp
                .headers()
                .get(reqwest::header::CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok())
            {
                head.push_str(&format!("Content-Length: {}\r\n", cl));
            }
            if let Some(cr) = resp
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
            {
                head.push_str(&format!("Content-Range: {}\r\n", cr));
            }
            head.push_str("\r\n");

            if stream.write_all(head.as_bytes()).await.is_err() {
                return;
            }
            if is_head {
                let _ = stream.flush().await;
                return;
            }

            while let Ok(Some(chunk)) = resp.chunk().await {
                if stream.write_all(&chunk).await.is_err() {
                    break;
                }
            }
            let _ = stream.flush().await;
        }
        CastStreamPlan::FfmpegMux {
            ffmpeg_bin,
            video_url,
            audio_url,
            needs_aac_transcode,
            headers,
        } => {
            let head = "HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nAccess-Control-Allow-Origin: *\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
            if is_head {
                let _ = stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
                    )
                    .await;
                return;
            }
            if stream.write_all(head.as_bytes()).await.is_err() {
                return;
            }

            let mut cmd = Command::new(&ffmpeg_bin);
            cmd.args(["-nostdin", "-loglevel", "error"]);
            append_ffmpeg_input_headers(&mut cmd, &headers);
            cmd.args(["-i", &video_url]);
            append_ffmpeg_input_headers(&mut cmd, &headers);
            cmd.args(["-i", &audio_url, "-map", "0:v:0", "-map", "1:a:0", "-c:v", "copy"]);
            if needs_aac_transcode {
                cmd.args(["-c:a", "aac", "-b:a", "192k"]);
            } else {
                cmd.args(["-c:a", "copy"]);
            }
            cmd.args([
                "-movflags",
                "frag_keyframe+empty_moov+default_base_moof",
                "-f",
                "mp4",
                "pipe:1",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
            #[cfg(target_os = "windows")]
            cmd.creation_flags(0x08000000);

            pipe_ffmpeg_chunked(&mut stream, cmd).await;
        }
        CastStreamPlan::FfmpegAudio {
            ffmpeg_bin,
            audio_url,
            headers,
        } => {
            if is_head {
                let _ = stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
                    )
                    .await;
                return;
            }
            let head = "HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nAccess-Control-Allow-Origin: *\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
            if stream.write_all(head.as_bytes()).await.is_err() {
                return;
            }

            let mut cmd = Command::new(&ffmpeg_bin);
            cmd.args(["-nostdin", "-loglevel", "error"]);
            append_ffmpeg_input_headers(&mut cmd, &headers);
            cmd.args([
                "-i",
                &audio_url,
                "-vn",
                "-c:a",
                "libmp3lame",
                "-b:a",
                "192k",
                "-f",
                "mp3",
                "pipe:1",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
            #[cfg(target_os = "windows")]
            cmd.creation_flags(0x08000000);

            pipe_ffmpeg_chunked(&mut stream, cmd).await;
        }
    }
}

fn append_ffmpeg_input_headers(cmd: &mut Command, headers: &HashMap<String, String>) {
    let mut header_lines = String::new();
    for (k, v) in headers {
        if k.eq_ignore_ascii_case("user-agent") {
            cmd.arg("-user_agent").arg(v);
        } else if !k.eq_ignore_ascii_case("host")
            && !k.eq_ignore_ascii_case("range")
            && !k.eq_ignore_ascii_case("accept-encoding")
        {
            header_lines.push_str(k);
            header_lines.push_str(": ");
            header_lines.push_str(v);
            header_lines.push_str("\r\n");
        }
    }
    if !header_lines.is_empty() {
        cmd.arg("-headers").arg(header_lines);
    }
}

async fn pipe_ffmpeg_chunked(stream: &mut TcpStream, mut cmd: Command) {
    let Ok(mut child) = cmd.spawn() else {
        let _ = stream.write_all(b"0\r\n\r\n").await;
        return;
    };
    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill().await;
        let _ = stream.write_all(b"0\r\n\r\n").await;
        return;
    };

    let mut buf = vec![0u8; 32 * 1024];
    loop {
        match stdout.read(&mut buf).await {
            Ok(0) => {
                let _ = stream.write_all(b"0\r\n\r\n").await;
                break;
            }
            Ok(n) => {
                let hdr = format!("{:x}\r\n", n);
                if stream.write_all(hdr.as_bytes()).await.is_err()
                    || stream.write_all(&buf[..n]).await.is_err()
                    || stream.write_all(b"\r\n").await.is_err()
                {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let _ = stream.flush().await;
    let _ = child.kill().await;
    let _ = child.wait().await;
}

async fn spawn_local_stream_server(
    target_addr: SocketAddr,
    plan: CastStreamPlan,
) -> Result<(String, watch::Sender<bool>), String> {
    let lan_ip = local_ip_for_target(target_addr)?;
    let listener = TcpListener::bind("0.0.0.0:0")
        .await
        .map_err(|e| format!("Failed to start local Cast relay server: {}", e))?;
    let local_port = listener
        .local_addr()
        .map_err(|e| format!("Failed to inspect local server port: {}", e))?
        .port();

    let token = Uuid::new_v4().to_string();
    let path = format!("/media/{}", token);
    let host_str = match lan_ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => format!("[{}]", v6),
    };
    let media_url = format!("http://{}:{}{}", host_str, local_port, path);

    let http_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3600))
        .build()
        .map_err(|e| format!("Failed to build proxy HTTP client: {}", e))?;

    let (stop_tx, mut stop_rx) = watch::channel(false);

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = stop_rx.changed() => {
                    if *stop_rx.borrow() {
                        break;
                    }
                }
                accept_res = listener.accept() => {
                    let Ok((sock, _)) = accept_res else {
                        break;
                    };
                    let p = path.clone();
                    let pl = plan.clone();
                    let hc = http_client.clone();
                    let mut conn_stop = stop_rx.clone();
                    tokio::spawn(async move {
                        tokio::select! {
                            _ = handle_http_client(sock, p, pl, hc) => {}
                            _ = conn_stop.changed() => {}
                        }
                    });
                }
            }
        }
    });

    Ok((media_url, stop_tx))
}

// --- CastV2 Protobuf Wire Framing ---

fn encode_varint(mut v: u64, out: &mut Vec<u8>) {
    loop {
        let byte = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            break;
        } else {
            out.push(byte | 0x80);
        }
    }
}

fn encode_varint_field(field_num: u32, val: u64, out: &mut Vec<u8>) {
    encode_varint(((field_num as u64) << 3) | 0, out);
    encode_varint(val, out);
}

fn encode_bytes_field(field_num: u32, bytes: &[u8], out: &mut Vec<u8>) {
    encode_varint(((field_num as u64) << 3) | 2, out);
    encode_varint(bytes.len() as u64, out);
    out.extend_from_slice(bytes);
}

fn encode_cast_frame(source: &str, destination: &str, namespace: &str, payload: &str) -> Vec<u8> {
    let mut pb = Vec::with_capacity(64 + payload.len());
    encode_varint_field(1, 0, &mut pb); // protocol_version = CASTV2_1_0
    encode_bytes_field(2, source.as_bytes(), &mut pb);
    encode_bytes_field(3, destination.as_bytes(), &mut pb);
    encode_bytes_field(4, namespace.as_bytes(), &mut pb);
    encode_varint_field(5, 0, &mut pb); // payload_type = STRING
    encode_bytes_field(6, payload.as_bytes(), &mut pb);

    let mut frame = Vec::with_capacity(4 + pb.len());
    frame.extend_from_slice(&(pb.len() as u32).to_be_bytes());
    frame.extend_from_slice(&pb);
    frame
}

#[derive(Debug, Default)]
struct RawCastMessage {
    source_id: String,
    destination_id: String,
    namespace: String,
    payload_utf8: String,
}

fn read_varint(buf: &[u8], pos: &mut usize) -> Result<u64, String> {
    let mut result = 0u64;
    let mut shift = 0u32;
    loop {
        if *pos >= buf.len() {
            return Err("Unexpected EOF decoding CastV2 varint".to_string());
        }
        let b = buf[*pos];
        *pos += 1;
        result |= ((b & 0x7F) as u64) << shift;
        if (b & 0x80) == 0 {
            return Ok(result);
        }
        shift += 7;
        if shift >= 64 {
            return Err("CastV2 varint overflow".to_string());
        }
    }
}

fn decode_cast_message(buf: &[u8]) -> Result<RawCastMessage, String> {
    let mut msg = RawCastMessage::default();
    let mut pos = 0usize;
    while pos < buf.len() {
        let tag = read_varint(buf, &mut pos)?;
        let field_num = tag >> 3;
        let wire_type = tag & 0x07;
        match wire_type {
            0 => {
                let _ = read_varint(buf, &mut pos)?;
            }
            1 => {
                pos = pos
                    .checked_add(8)
                    .filter(|&p| p <= buf.len())
                    .ok_or_else(|| "Truncated 64-bit CastV2 field".to_string())?;
            }
            2 => {
                let len = read_varint(buf, &mut pos)? as usize;
                let end = pos
                    .checked_add(len)
                    .filter(|&e| e <= buf.len())
                    .ok_or_else(|| "Truncated length-delimited CastV2 field".to_string())?;
                let slice = &buf[pos..end];
                pos = end;
                match field_num {
                    2 => msg.source_id = String::from_utf8_lossy(slice).into_owned(),
                    3 => msg.destination_id = String::from_utf8_lossy(slice).into_owned(),
                    4 => msg.namespace = String::from_utf8_lossy(slice).into_owned(),
                    6 => msg.payload_utf8 = String::from_utf8_lossy(slice).into_owned(),
                    _ => {}
                }
            }
            5 => {
                pos = pos
                    .checked_add(4)
                    .filter(|&p| p <= buf.len())
                    .ok_or_else(|| "Truncated 32-bit CastV2 field".to_string())?;
            }
            other => {
                return Err(format!("Unsupported CastV2 protobuf wire type {}", other));
            }
        }
    }
    Ok(msg)
}

async fn read_cast_frame<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut R,
) -> Result<RawCastMessage, String> {
    let mut len_buf = [0u8; 4];
    reader
        .read_exact(&mut len_buf)
        .await
        .map_err(|e| format!("Cast connection closed: {}", e))?;
    let frame_len = u32::from_be_bytes(len_buf) as usize;
    if frame_len == 0 || frame_len > 2 * 1024 * 1024 {
        return Err(format!("Invalid CastV2 frame length: {}", frame_len));
    }
    let mut payload = vec![0u8; frame_len];
    reader
        .read_exact(&mut payload)
        .await
        .map_err(|e| format!("Cast frame read error: {}", e))?;
    decode_cast_message(&payload)
}

async fn send_cast_json<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    destination: &str,
    namespace: &str,
    body: &Value,
) -> Result<(), String> {
    let payload = body.to_string();
    let frame = encode_cast_frame("sender-0", destination, namespace, &payload);
    writer
        .write_all(&frame)
        .await
        .map_err(|e| format!("Cast send error: {}", e))?;
    writer
        .flush()
        .await
        .map_err(|e| format!("Cast flush error: {}", e))?;
    Ok(())
}

pub async fn run_cast_session(
    seq: u64,
    device: CastDevice,
    item: CastMediaItem,
    mut cmd_rx: mpsc::UnboundedReceiver<CastCommand>,
    tx: mpsc::UnboundedSender<UiEvent>,
    ctx: egui::Context,
) {
    let mut state = CastSessionState {
        device: device.clone(),
        title: item.title.clone(),
        artist: item.artist.clone(),
        thumbnail: item.thumbnail.clone(),
        status: CastPlayerStatus::Connecting,
        is_audio: item.is_audio,
        current_time: 0.0,
        duration: None,
        seekable: false,
        volume: 1.0,
        muted: false,
    };
    let _ = tx.send(UiEvent::CastStateUpdated(seq, Some(state.clone())));
    ctx.request_repaint();

    // 1. Resolve playable media URL via yt-dlp
    let plan = match resolve_cast_stream(&item).await {
        Ok(p) => p,
        Err(e) => {
            let _ = tx.send(UiEvent::CastError(seq, e));
            let _ = tx.send(UiEvent::CastStateUpdated(seq, None));
            ctx.request_repaint();
            return;
        }
    };
    let content_type = plan.content_type().to_string();
    state.seekable = plan.seekable();

    // 2. Resolve target address & start local LAN HTTP relay server
    let target_addr = match resolve_target_socket_addr(&device.host, device.port) {
        Ok(a) => a,
        Err(e) => {
            let _ = tx.send(UiEvent::CastError(seq, e));
            let _ = tx.send(UiEvent::CastStateUpdated(seq, None));
            ctx.request_repaint();
            return;
        }
    };

    let (media_url, server_stop_tx) = match spawn_local_stream_server(target_addr, plan).await {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.send(UiEvent::CastError(seq, e));
            let _ = tx.send(UiEvent::CastStateUpdated(seq, None));
            ctx.request_repaint();
            return;
        }
    };

    // 3. Connect TLS to Chromecast / Android TV (port 8009)
    let tcp_stream = match tokio::time::timeout(Duration::from_secs(7), TcpStream::connect(target_addr))
        .await
    {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            let _ = server_stop_tx.send(true);
            let _ = tx.send(UiEvent::CastError(
                seq,
                format!("Could not connect to TV ({}): {}", target_addr, e),
            ));
            let _ = tx.send(UiEvent::CastStateUpdated(seq, None));
            ctx.request_repaint();
            return;
        }
        Err(_) => {
            let _ = server_stop_tx.send(true);
            let _ = tx.send(UiEvent::CastError(
                seq,
                format!("Connection timed out to TV ({})", target_addr),
            ));
            let _ = tx.send(UiEvent::CastStateUpdated(seq, None));
            ctx.request_repaint();
            return;
        }
    };

    let connector = match build_cast_tls_connector() {
        Ok(c) => c,
        Err(e) => {
            let _ = server_stop_tx.send(true);
            let _ = tx.send(UiEvent::CastError(seq, e));
            let _ = tx.send(UiEvent::CastStateUpdated(seq, None));
            ctx.request_repaint();
            return;
        }
    };

    let clean_host = device.host.trim().trim_start_matches('[').trim_end_matches(']');
    let server_name: ServerName<'static> = match clean_host.parse::<IpAddr>() {
        Ok(ip) => ServerName::IpAddress(ip.into()),
        Err(_) => ServerName::try_from(clean_host.to_string())
            .unwrap_or_else(|_| ServerName::try_from("chromecast.local").expect("valid fallback DNS")),
    };

    let tls_stream =
        match tokio::time::timeout(Duration::from_secs(7), connector.connect(server_name, tcp_stream))
            .await
        {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => {
                let _ = server_stop_tx.send(true);
                let _ = tx.send(UiEvent::CastError(seq, format!("Cast TLS error: {}", e)));
                let _ = tx.send(UiEvent::CastStateUpdated(seq, None));
                ctx.request_repaint();
                return;
            }
            Err(_) => {
                let _ = server_stop_tx.send(true);
                let _ = tx.send(UiEvent::CastError(seq, "Cast TLS handshake timed out.".to_string()));
                let _ = tx.send(UiEvent::CastStateUpdated(seq, None));
                ctx.request_repaint();
                return;
            }
        };

    let (mut tls_reader, mut tls_writer) = tokio::io::split(tls_stream);

    // Spawn dedicated cancellation-safe frame reader
    let (msg_tx, mut msg_rx) = mpsc::unbounded_channel::<Result<RawCastMessage, String>>();
    let reader_handle = tokio::spawn(async move {
        loop {
            let res = read_cast_frame(&mut tls_reader).await;
            let is_err = res.is_err();
            if msg_tx.send(res).is_err() || is_err {
                break;
            }
        }
    });

    let mut req_id: u64 = 1;
    let mut next_req_id = || {
        let id = req_id;
        req_id += 1;
        id
    };

    // Connect to receiver-0 and launch Default Media Receiver
    if let Err(e) = send_cast_json(
        &mut tls_writer,
        "receiver-0",
        NS_CONNECTION,
        &json!({ "type": "CONNECT" }),
    )
    .await
    {
        reader_handle.abort();
        let _ = server_stop_tx.send(true);
        let _ = tx.send(UiEvent::CastError(seq, e));
        let _ = tx.send(UiEvent::CastStateUpdated(seq, None));
        ctx.request_repaint();
        return;
    }

    let launch_req_id = next_req_id();
    if let Err(e) = send_cast_json(
        &mut tls_writer,
        "receiver-0",
        NS_RECEIVER,
        &json!({
            "type": "LAUNCH",
            "appId": DEFAULT_MEDIA_RECEIVER_APP_ID,
            "requestId": launch_req_id
        }),
    )
    .await
    {
        reader_handle.abort();
        let _ = server_stop_tx.send(true);
        let _ = tx.send(UiEvent::CastError(seq, e));
        let _ = tx.send(UiEvent::CastStateUpdated(seq, None));
        ctx.request_repaint();
        return;
    }

    // Wait for receiver transportId
    let mut transport_id: Option<String> = None;
    let mut session_id: Option<String> = None;
    let launch_deadline = tokio::time::Instant::now() + Duration::from_secs(12);

    while transport_id.is_none() {
        let now = tokio::time::Instant::now();
        if now >= launch_deadline {
            break;
        }
        tokio::select! {
            cmd_opt = cmd_rx.recv() => {
                if matches!(cmd_opt, Some(CastCommand::Stop) | None) {
                    reader_handle.abort();
                    let _ = server_stop_tx.send(true);
                    let _ = tx.send(UiEvent::CastStateUpdated(seq, None));
                    ctx.request_repaint();
                    return;
                }
            }
            msg_opt = tokio::time::timeout(launch_deadline - now, msg_rx.recv()) => {
                match msg_opt {
                    Ok(Some(Ok(msg))) => {
                        if let Ok(v) = serde_json::from_str::<Value>(&msg.payload_utf8) {
                            let mtype = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
                            if msg.namespace == NS_HEARTBEAT && mtype == "PING" {
                                let _ = send_cast_json(
                                    &mut tls_writer,
                                    "receiver-0",
                                    NS_HEARTBEAT,
                                    &json!({ "type": "PONG" }),
                                ).await;
                            } else if msg.namespace == NS_RECEIVER && mtype == "RECEIVER_STATUS" {
                                update_volume_from_receiver(&v, &mut state);
                                if let Some(apps) = v
                                    .get("status")
                                    .and_then(|s| s.get("applications"))
                                    .and_then(|a| a.as_array())
                                {
                                    if let Some(app) = apps.iter().find(|a| {
                                        a.get("appId").and_then(|id| id.as_str())
                                            == Some(DEFAULT_MEDIA_RECEIVER_APP_ID)
                                    }) {
                                        transport_id = app
                                            .get("transportId")
                                            .or_else(|| app.get("sessionId"))
                                            .and_then(|s| s.as_str())
                                            .map(ToString::to_string);
                                        session_id = app
                                            .get("sessionId")
                                            .and_then(|s| s.as_str())
                                            .map(ToString::to_string);
                                    }
                                }
                            } else if mtype == "LAUNCH_ERROR" {
                                let reason = v
                                    .get("reason")
                                    .and_then(|r| r.as_str())
                                    .unwrap_or("Launch failed");
                                reader_handle.abort();
                                let _ = server_stop_tx.send(true);
                                let _ = tx.send(UiEvent::CastError(seq, format!("Cast launch error: {}", reason)));
                                let _ = tx.send(UiEvent::CastStateUpdated(seq, None));
                                ctx.request_repaint();
                                return;
                            }
                        }
                    }
                    Ok(Some(Err(e))) => {
                        reader_handle.abort();
                        let _ = server_stop_tx.send(true);
                        let _ = tx.send(UiEvent::CastError(seq, e));
                        let _ = tx.send(UiEvent::CastStateUpdated(seq, None));
                        ctx.request_repaint();
                        return;
                    }
                    Ok(None) | Err(_) => break,
                }
            }
        }
    }

    let Some(tid) = transport_id else {
        reader_handle.abort();
        let _ = server_stop_tx.send(true);
        let _ = tx.send(UiEvent::CastError(
            seq,
            "TV did not start the Cast media receiver in time.".to_string(),
        ));
        let _ = tx.send(UiEvent::CastStateUpdated(seq, None));
        ctx.request_repaint();
        return;
    };

    // Connect to the receiver app transport & send LOAD
    if let Err(e) = send_cast_json(&mut tls_writer, &tid, NS_CONNECTION, &json!({ "type": "CONNECT" })).await {
        reader_handle.abort();
        let _ = server_stop_tx.send(true);
        let _ = tx.send(UiEvent::CastError(seq, e));
        let _ = tx.send(UiEvent::CastStateUpdated(seq, None));
        ctx.request_repaint();
        return;
    }

    let images = item
        .thumbnail
        .as_deref()
        .map(|u| json!([{ "url": u }]))
        .unwrap_or_else(|| json!([]));

    let metadata = if item.is_audio {
        json!({
            "metadataType": 3,
            "title": item.title,
            "artist": item.artist,
            "images": images
        })
    } else {
        json!({
            "metadataType": 0,
            "title": item.title,
            "subtitle": item.artist,
            "images": images
        })
    };

    let load_req_id = next_req_id();
    if let Err(e) = send_cast_json(
        &mut tls_writer,
        &tid,
        NS_MEDIA,
        &json!({
            "type": "LOAD",
            "requestId": load_req_id,
            "autoplay": true,
            "currentTime": 0,
            "media": {
                "contentId": media_url,
                "streamType": "BUFFERED",
                "contentType": content_type,
                "metadata": metadata
            }
        }),
    )
    .await
    {
        reader_handle.abort();
        let _ = server_stop_tx.send(true);
        let _ = tx.send(UiEvent::CastError(seq, e));
        let _ = tx.send(UiEvent::CastStateUpdated(seq, None));
        ctx.request_repaint();
        return;
    }

    state.status = CastPlayerStatus::Buffering;
    let _ = tx.send(UiEvent::CastStateUpdated(seq, Some(state.clone())));
    ctx.request_repaint();

    // 4. Active playback control & heartbeat loop
    let mut media_session_id: Option<i64> = None;
    let mut ping_interval = tokio::time::interval(Duration::from_secs(5));
    let mut status_interval = tokio::time::interval(Duration::from_secs(1));
    let mut ever_played = false;

    loop {
        tokio::select! {
            _ = ping_interval.tick() => {
                if send_cast_json(
                    &mut tls_writer,
                    "receiver-0",
                    NS_HEARTBEAT,
                    &json!({ "type": "PING" }),
                ).await.is_err() {
                    break;
                }
            }
            _ = status_interval.tick() => {
                if media_session_id.is_some() {
                    let _ = send_cast_json(
                        &mut tls_writer,
                        &tid,
                        NS_MEDIA,
                        &json!({ "type": "GET_STATUS", "requestId": next_req_id() }),
                    ).await;
                }
            }
            cmd_opt = cmd_rx.recv() => {
                match cmd_opt {
                    Some(CastCommand::Pause) => {
                        if let Some(msid) = media_session_id {
                            let _ = send_cast_json(
                                &mut tls_writer,
                                &tid,
                                NS_MEDIA,
                                &json!({ "type": "PAUSE", "mediaSessionId": msid, "requestId": next_req_id() }),
                            ).await;
                        }
                    }
                    Some(CastCommand::Play) => {
                        if let Some(msid) = media_session_id {
                            let _ = send_cast_json(
                                &mut tls_writer,
                                &tid,
                                NS_MEDIA,
                                &json!({ "type": "PLAY", "mediaSessionId": msid, "requestId": next_req_id() }),
                            ).await;
                        }
                    }
                    Some(CastCommand::Seek(pos)) => {
                        if let Some(msid) = media_session_id {
                            state.current_time = pos.max(0.0);
                            let _ = send_cast_json(
                                &mut tls_writer,
                                &tid,
                                NS_MEDIA,
                                &json!({ "type": "SEEK", "mediaSessionId": msid, "currentTime": pos.max(0.0), "requestId": next_req_id() }),
                            ).await;
                            let _ = tx.send(UiEvent::CastStateUpdated(seq, Some(state.clone())));
                            ctx.request_repaint();
                        }
                    }
                    Some(CastCommand::SetVolume(level)) => {
                        let clamped = level.clamp(0.0, 1.0);
                        state.volume = clamped;
                        let _ = send_cast_json(
                            &mut tls_writer,
                            "receiver-0",
                            NS_RECEIVER,
                            &json!({ "type": "SET_VOLUME", "volume": { "level": clamped }, "requestId": next_req_id() }),
                        ).await;
                        let _ = tx.send(UiEvent::CastStateUpdated(seq, Some(state.clone())));
                        ctx.request_repaint();
                    }
                    Some(CastCommand::SetMuted(muted)) => {
                        state.muted = muted;
                        let _ = send_cast_json(
                            &mut tls_writer,
                            "receiver-0",
                            NS_RECEIVER,
                            &json!({ "type": "SET_VOLUME", "volume": { "muted": muted }, "requestId": next_req_id() }),
                        ).await;
                        let _ = tx.send(UiEvent::CastStateUpdated(seq, Some(state.clone())));
                        ctx.request_repaint();
                    }
                    Some(CastCommand::Stop) | None => {
                        if let Some(msid) = media_session_id {
                            let _ = send_cast_json(
                                &mut tls_writer,
                                &tid,
                                NS_MEDIA,
                                &json!({ "type": "STOP", "mediaSessionId": msid, "requestId": next_req_id() }),
                            ).await;
                        }
                        if let Some(sid) = session_id.as_deref() {
                            let _ = send_cast_json(
                                &mut tls_writer,
                                "receiver-0",
                                NS_RECEIVER,
                                &json!({ "type": "STOP", "sessionId": sid, "requestId": next_req_id() }),
                            ).await;
                        }
                        break;
                    }
                }
            }
            msg_opt = msg_rx.recv() => {
                let Some(msg_res) = msg_opt else {
                    break;
                };
                let Ok(msg) = msg_res else {
                    break;
                };
                let Ok(v) = serde_json::from_str::<Value>(&msg.payload_utf8) else {
                    continue;
                };
                let mtype = v.get("type").and_then(|t| t.as_str()).unwrap_or("");

                if msg.namespace == NS_HEARTBEAT && mtype == "PING" {
                    let _ = send_cast_json(
                        &mut tls_writer,
                        "receiver-0",
                        NS_HEARTBEAT,
                        &json!({ "type": "PONG" }),
                    ).await;
                } else if msg.namespace == NS_CONNECTION && mtype == "CLOSE" && msg.source_id == tid {
                    break;
                } else if msg.namespace == NS_RECEIVER && mtype == "RECEIVER_STATUS" {
                    update_volume_from_receiver(&v, &mut state);
                    let _ = tx.send(UiEvent::CastStateUpdated(seq, Some(state.clone())));
                    ctx.request_repaint();
                } else if msg.namespace == NS_MEDIA && mtype == "MEDIA_STATUS" {
                    if let Some(first) = v
                        .get("status")
                        .and_then(|s| s.as_array())
                        .and_then(|arr| arr.first())
                    {
                        if let Some(msid) = first.get("mediaSessionId").and_then(|m| m.as_i64()) {
                            media_session_id = Some(msid);
                        }
                        if let Some(ct) = first.get("currentTime").and_then(|c| c.as_f64()) {
                            state.current_time = ct;
                        }
                        if let Some(dur) = first
                            .get("media")
                            .and_then(|m| m.get("duration"))
                            .and_then(|d| d.as_f64())
                        {
                            if dur.is_finite() && dur > 0.0 {
                                state.duration = Some(dur);
                            }
                        }

                        let pstate = first
                            .get("playerState")
                            .and_then(|p| p.as_str())
                            .unwrap_or("");
                        match pstate {
                            "PLAYING" => {
                                ever_played = true;
                                state.status = CastPlayerStatus::Playing;
                            }
                            "PAUSED" => {
                                ever_played = true;
                                state.status = CastPlayerStatus::Paused;
                            }
                            "BUFFERING" => {
                                state.status = CastPlayerStatus::Buffering;
                            }
                            "IDLE" => {
                                state.status = CastPlayerStatus::Idle;
                                let idle_reason = first
                                    .get("idleReason")
                                    .and_then(|r| r.as_str())
                                    .unwrap_or("");
                                if idle_reason == "ERROR" {
                                    let _ = tx.send(UiEvent::CastError(
                                        seq,
                                        "TV failed to play media (check local firewall/LAN access).".to_string(),
                                    ));
                                    break;
                                }
                                if ever_played && matches!(idle_reason, "FINISHED" | "CANCELLED") {
                                    break;
                                }
                            }
                            _ => {}
                        }
                        let _ = tx.send(UiEvent::CastStateUpdated(seq, Some(state.clone())));
                        ctx.request_repaint();
                    }
                } else if msg.namespace == NS_MEDIA && mtype == "LOAD_FAILED" {
                    let _ = tx.send(UiEvent::CastError(
                        seq,
                        "TV could not load the stream (check firewall or try another item).".to_string(),
                    ));
                    break;
                }
            }
        }
    }

    reader_handle.abort();
    let _ = server_stop_tx.send(true);
    let _ = tx.send(UiEvent::CastStateUpdated(seq, None));
    ctx.request_repaint();
}

fn update_volume_from_receiver(v: &Value, state: &mut CastSessionState) {
    if let Some(vol) = v.get("status").and_then(|s| s.get("volume")) {
        if let Some(level) = vol.get("level").and_then(|l| l.as_f64()) {
            state.volume = (level as f32).clamp(0.0, 1.0);
        }
        if let Some(muted) = vol.get("muted").and_then(|m| m.as_bool()) {
            state.muted = muted;
        }
    }
}
