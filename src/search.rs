use crate::types::{
    sanitize_bandcamp_url, MusicSearchItemKind, MusicSearchResult, MusicSearchSource,
};
use serde_json::{json, Value};
use std::collections::HashSet;

const YTM_SONGS_PARAMS: &str = "EgWKAQIIAWoKEAMQBBAJEAoQBQ%3D%3D";
const YTM_ALBUMS_PARAMS: &str = "EgWKAQIYAWoKEAMQBBAJEAoQBQ%3D%3D";

const BROWSER_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36";

fn flex_col_text(renderer: &Value, index: usize) -> String {
    renderer
        .get("flexColumns")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.get(index))
        .and_then(|fc| fc.get("musicResponsiveListItemFlexColumnRenderer"))
        .and_then(|inner| inner.get("text"))
        .and_then(|t| t.get("runs"))
        .and_then(|r| r.as_array())
        .map(|runs| {
            runs.iter()
                .filter_map(|run| run.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn extract_ytm_thumbnail(renderer: &Value) -> Option<String> {
    renderer
        .get("thumbnail")
        .and_then(|t| t.get("musicThumbnailRenderer"))
        .and_then(|m| m.get("thumbnail"))
        .and_then(|t| t.get("thumbnails"))
        .and_then(|arr| arr.as_array())
        .and_then(|arr| arr.last())
        .and_then(|item| item.get("url"))
        .and_then(|u| u.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn extract_ytm_video_id(renderer: &Value) -> Option<String> {
    renderer
        .get("overlay")
        .and_then(|o| o.get("musicItemThumbnailOverlayRenderer"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.get("musicPlayButtonRenderer"))
        .and_then(|p| p.get("playNavigationEndpoint"))
        .and_then(|n| n.get("watchEndpoint"))
        .and_then(|w| w.get("videoId"))
        .and_then(|v| v.as_str())
        .or_else(|| {
            renderer
                .get("navigationEndpoint")
                .and_then(|n| n.get("watchEndpoint"))
                .and_then(|w| w.get("videoId"))
                .and_then(|v| v.as_str())
        })
        .or_else(|| {
            renderer
                .get("flexColumns")
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
                .and_then(|fc| fc.get("musicResponsiveListItemFlexColumnRenderer"))
                .and_then(|inner| inner.get("text"))
                .and_then(|t| t.get("runs"))
                .and_then(|r| r.as_array())
                .and_then(|runs| runs.first())
                .and_then(|run| run.get("navigationEndpoint"))
                .and_then(|n| n.get("watchEndpoint"))
                .and_then(|w| w.get("videoId"))
                .and_then(|v| v.as_str())
        })
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn extract_ytm_album_url(renderer: &Value) -> Option<(String, String)> {
    let overlay_playlist = renderer
        .get("overlay")
        .and_then(|o| o.get("musicItemThumbnailOverlayRenderer"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.get("musicPlayButtonRenderer"))
        .and_then(|p| p.get("playNavigationEndpoint"))
        .and_then(|n| n.get("watchPlaylistEndpoint"))
        .and_then(|w| w.get("playlistId"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && !s.starts_with("RD"));

    let playlist_id = overlay_playlist.or_else(|| {
        renderer
            .get("menu")
            .and_then(|m| m.get("menuRenderer"))
            .and_then(|mr| mr.get("items"))
            .and_then(|arr| arr.as_array())
            .and_then(|items| {
                items.iter().find_map(|it| {
                    it.get("menuNavigationItemRenderer")
                        .and_then(|mn| mn.get("navigationEndpoint"))
                        .and_then(|n| n.get("watchPlaylistEndpoint"))
                        .and_then(|w| w.get("playlistId"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty() && !s.starts_with("RD"))
                })
            })
    });

    let browse_id = renderer
        .get("navigationEndpoint")
        .and_then(|n| n.get("browseEndpoint"))
        .and_then(|b| b.get("browseId"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    if let Some(pl) = playlist_id {
        let id = browse_id.unwrap_or_else(|| pl.clone());
        return Some((
            id,
            format!("https://music.youtube.com/playlist?list={}", pl),
        ));
    }

    if let Some(bid) = browse_id {
        if let Some(pl) = bid.strip_prefix("VL") {
            return Some((
                bid.clone(),
                format!("https://music.youtube.com/playlist?list={}", pl),
            ));
        }
        return Some((
            bid.clone(),
            format!("https://music.youtube.com/browse/{}", bid),
        ));
    }

    None
}

fn looks_like_duration(s: &str) -> bool {
    let t = s.trim();
    t.contains(':') && !t.is_empty() && t.chars().all(|c| c.is_ascii_digit() || c == ':')
}

fn looks_like_year(s: &str) -> bool {
    let t = s.trim();
    t.len() == 4 && t.chars().all(|c| c.is_ascii_digit())
}

fn parse_song_subtitle(sub: &str) -> (String, Option<String>, Option<String>) {
    let mut parts: Vec<String> = sub
        .split(['•', '·'])
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();

    if let Some(first) = parts.first() {
        if first.eq_ignore_ascii_case("song")
            || first.eq_ignore_ascii_case("video")
            || first.eq_ignore_ascii_case("dal")
        {
            parts.remove(0);
        }
    }

    let duration_text = if parts.last().is_some_and(|p| looks_like_duration(p)) {
        parts.pop()
    } else {
        None
    };

    let artist = parts
        .first()
        .cloned()
        .unwrap_or_else(|| "Unknown Artist".to_string());
    let album = if parts.len() >= 2 {
        Some(parts[1..].join(" · "))
    } else {
        None
    };

    (artist, album, duration_text)
}

fn parse_album_subtitle(sub: &str) -> (Option<String>, String, Option<String>) {
    let mut parts: Vec<String> = sub
        .split(['•', '·'])
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();

    let release_type = if let Some(first) = parts.first() {
        if matches!(
            first.to_ascii_lowercase().as_str(),
            "album" | "ep" | "single" | "playlist"
        ) {
            Some(parts.remove(0))
        } else {
            Some("Album".to_string())
        }
    } else {
        Some("Album".to_string())
    };

    let year = if parts.last().is_some_and(|p| looks_like_year(p)) {
        parts.pop()
    } else {
        None
    };

    let artist = if parts.is_empty() {
        "Unknown Artist".to_string()
    } else {
        parts.join(" · ")
    };

    (release_type, artist, year)
}

async fn search_ytm_kind(
    client: &reqwest::Client,
    query: &str,
    is_album: bool,
) -> Result<Vec<MusicSearchResult>, String> {
    let params = if is_album {
        YTM_ALBUMS_PARAMS
    } else {
        YTM_SONGS_PARAMS
    };

    let payload = json!({
        "context": {
            "client": {
                "clientName": "WEB_REMIX",
                "clientVersion": "1.20250101.01.00",
                "hl": "en",
                "gl": "US"
            }
        },
        "query": query,
        "params": params
    });

    let resp = client
        .post("https://music.youtube.com/youtubei/v1/search?prettyPrint=false")
        .header("User-Agent", BROWSER_UA)
        .header("Origin", "https://music.youtube.com")
        .header("Referer", "https://music.youtube.com/")
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("YouTube Music network error: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("YouTube Music HTTP {}", resp.status()));
    }

    let data: Value = resp
        .json()
        .await
        .map_err(|e| format!("YouTube Music JSON parse error: {}", e))?;

    let sections = data
        .get("contents")
        .and_then(|c| c.get("tabbedSearchResultsRenderer"))
        .and_then(|t| t.get("tabs"))
        .and_then(|t| t.as_array())
        .and_then(|arr| arr.first())
        .and_then(|t| t.get("tabRenderer"))
        .and_then(|tr| tr.get("content"))
        .and_then(|c| c.get("sectionListRenderer"))
        .and_then(|s| s.get("contents"))
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();

    let mut shelves: Vec<&Value> = Vec::new();
    for sec in &sections {
        if let Some(shelf) = sec.get("musicShelfRenderer") {
            shelves.push(shelf);
        }
        if let Some(items) = sec
            .get("itemSectionRenderer")
            .and_then(|is| is.get("contents"))
            .and_then(|c| c.as_array())
        {
            for item in items {
                if let Some(shelf) = item.get("musicShelfRenderer") {
                    shelves.push(shelf);
                }
            }
        }
    }

    let mut results = Vec::new();
    for shelf in shelves {
        let Some(contents) = shelf.get("contents").and_then(|c| c.as_array()) else {
            continue;
        };
        for item in contents {
            let Some(renderer) = item.get("musicResponsiveListItemRenderer") else {
                continue;
            };
            let title = flex_col_text(renderer, 0);
            if title.is_empty() {
                continue;
            }
            let sub = flex_col_text(renderer, 1);
            let thumbnail = extract_ytm_thumbnail(renderer);

            if is_album {
                let Some((id_part, url)) = extract_ytm_album_url(renderer) else {
                    continue;
                };
                let (release_type, artist, year) = parse_album_subtitle(&sub);
                results.push(MusicSearchResult {
                    id: format!("ytm_album_{}", id_part),
                    source: MusicSearchSource::YoutubeMusic,
                    kind: MusicSearchItemKind::Album,
                    release_type,
                    title: title.clone(),
                    artist,
                    album: Some(title),
                    year,
                    duration_text: None,
                    url,
                    thumbnail,
                });
            } else {
                let Some(video_id) = extract_ytm_video_id(renderer) else {
                    continue;
                };
                let (artist, album, duration_text) = parse_song_subtitle(&sub);
                results.push(MusicSearchResult {
                    id: format!("ytm_song_{}", video_id),
                    source: MusicSearchSource::YoutubeMusic,
                    kind: MusicSearchItemKind::Song,
                    release_type: None,
                    title,
                    artist,
                    album,
                    year: None,
                    duration_text,
                    url: format!("https://music.youtube.com/watch?v={}", video_id),
                    thumbnail,
                });
            }

            if results.len() >= 20 {
                break;
            }
        }
        if results.len() >= 20 {
            break;
        }
    }

    Ok(results)
}

fn extract_bandcamp_thumbnail(item: &Value) -> Option<String> {
    if let Some(art_id) = item
        .get("art_id")
        .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.trim().parse::<u64>().ok())))
        .filter(|&id| id > 0)
    {
        return Some(format!("https://f4.bcbits.com/img/a{:010}_9.jpg", art_id));
    }

    let raw = item
        .get("img")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?;

    if let Some((prefix, rest)) = raw.split_once("/img/") {
        if rest.starts_with(|c: char| c.is_ascii_digit()) {
            return Some(format!("{}/img/a{}", prefix, rest));
        }
    }

    Some(raw.to_string())
}

async fn search_bandcamp(
    client: &reqwest::Client,
    query: &str,
) -> Result<(Vec<MusicSearchResult>, Vec<MusicSearchResult>), String> {
    let resp = client
        .get("https://bandcamp.com/api/fuzzysearch/1/app_autocomplete")
        .query(&[("q", query)])
        .header(
            "User-Agent",
            concat!("VideoDownloader/", env!("CARGO_PKG_VERSION")),
        )
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("Bandcamp network error: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("Bandcamp HTTP {}", resp.status()));
    }

    let data: Value = resp
        .json()
        .await
        .map_err(|e| format!("Bandcamp JSON parse error: {}", e))?;

    let items = data
        .get("results")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();

    let mut albums = Vec::new();
    let mut songs = Vec::new();

    for item in items {
        let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if item_type != "a" && item_type != "t" {
            continue;
        }
        let name = item
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if name.is_empty() {
            continue;
        }
        let raw_url = item.get("url").and_then(|v| v.as_str()).unwrap_or("");
        let mut url = sanitize_bandcamp_url(raw_url);
        if url.starts_with('/') {
            url = format!("https://bandcamp.com{}", url);
        }
        if !url.starts_with("http://") && !url.starts_with("https://") {
            continue;
        }

        let artist = item
            .get("band_name")
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .unwrap_or("Unknown Artist")
            .to_string();

        let album_name = item
            .get("album_name")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let thumbnail = extract_bandcamp_thumbnail(&item);

        let id = item
            .get("id")
            .and_then(|v| v.as_u64())
            .map(|n| format!("bc_{}_{}", item_type, n))
            .unwrap_or_else(|| format!("bc_{}_{}", item_type, url));

        if item_type == "a" {
            albums.push(MusicSearchResult {
                id,
                source: MusicSearchSource::Bandcamp,
                kind: MusicSearchItemKind::Album,
                release_type: Some("Album".to_string()),
                title: name.clone(),
                artist,
                album: Some(name),
                year: None,
                duration_text: None,
                url,
                thumbnail,
            });
        } else {
            songs.push(MusicSearchResult {
                id,
                source: MusicSearchSource::Bandcamp,
                kind: MusicSearchItemKind::Song,
                release_type: None,
                title: name,
                artist,
                album: album_name,
                year: None,
                duration_text: None,
                url,
                thumbnail,
            });
        }
    }

    Ok((albums, songs))
}

pub async fn search_music(
    query: String,
    include_ytm: bool,
    include_bandcamp: bool,
) -> Result<Vec<MusicSearchResult>, String> {
    let trimmed = query.trim().to_string();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    if !include_ytm && !include_bandcamp {
        return Err("Select at least one search source (YouTube Music or Bandcamp).".to_string());
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .build()
        .map_err(|e| e.to_string())?;

    let (ytm_albums_res, ytm_songs_res, bc_res) = tokio::join!(
        async {
            if include_ytm {
                search_ytm_kind(&client, &trimmed, true).await
            } else {
                Ok(Vec::new())
            }
        },
        async {
            if include_ytm {
                search_ytm_kind(&client, &trimmed, false).await
            } else {
                Ok(Vec::new())
            }
        },
        async {
            if include_bandcamp {
                search_bandcamp(&client, &trimmed).await
            } else {
                Ok((Vec::new(), Vec::new()))
            }
        }
    );

    let mut errors = Vec::new();
    let ytm_albums = match ytm_albums_res {
        Ok(v) => v,
        Err(e) => {
            errors.push(e);
            Vec::new()
        }
    };
    let ytm_songs = match ytm_songs_res {
        Ok(v) => v,
        Err(e) => {
            errors.push(e);
            Vec::new()
        }
    };
    let (bc_albums, bc_songs) = match bc_res {
        Ok(v) => v,
        Err(e) => {
            errors.push(e);
            (Vec::new(), Vec::new())
        }
    };

    let expected_calls = (if include_ytm { 2 } else { 0 }) + (if include_bandcamp { 1 } else { 0 });
    if errors.len() >= expected_calls && !errors.is_empty() {
        return Err(errors.join(" | "));
    }

    // Interleave buckets (YTM Albums, YTM Songs, Bandcamp Albums, Bandcamp Songs)
    // so both full albums and individual songs from both platforms appear immediately at the top.
    let mut merged = Vec::new();
    let mut seen_urls = HashSet::new();
    let max_len = ytm_albums
        .len()
        .max(ytm_songs.len())
        .max(bc_albums.len())
        .max(bc_songs.len());

    let buckets: [&[MusicSearchResult]; 4] = [&ytm_albums, &ytm_songs, &bc_albums, &bc_songs];
    for idx in 0..max_len {
        for bucket in &buckets {
            if let Some(item) = bucket.get(idx) {
                let norm = item.url.trim().trim_end_matches('/').to_ascii_lowercase();
                if seen_urls.insert(norm) {
                    merged.push(item.clone());
                }
            }
        }
    }

    Ok(merged)
}

fn runs_text(val: Option<&Value>) -> String {
    val.and_then(|v| {
        if let Some(s) = v.get("simpleText").and_then(|t| t.as_str()) {
            return Some(s.to_string());
        }
        v.get("runs").and_then(|r| r.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|run| run.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("")
        })
    })
    .unwrap_or_default()
    .trim()
    .to_string()
}

fn collect_yt_video_renderers<'a>(val: &'a Value, out: &mut Vec<&'a Value>) {
    match val {
        Value::Object(map) => {
            if let Some(vr) = map.get("videoRenderer") {
                out.push(vr);
            }
            for v in map.values() {
                collect_yt_video_renderers(v, out);
            }
        }
        Value::Array(arr) => {
            for v in arr {
                collect_yt_video_renderers(v, out);
            }
        }
        _ => {}
    }
}

async fn search_youtube_videos(
    client: &reqwest::Client,
    query: &str,
) -> Result<Vec<MusicSearchResult>, String> {
    let payload = json!({
        "context": {
            "client": {
                "clientName": "WEB",
                "clientVersion": "2.20250101.00.00",
                "hl": "en",
                "gl": "US"
            }
        },
        "query": query,
        "params": "EgIQAQ%3D%3D"
    });

    let resp = client
        .post("https://www.youtube.com/youtubei/v1/search?prettyPrint=false")
        .header("User-Agent", BROWSER_UA)
        .header("Origin", "https://www.youtube.com")
        .header("Referer", "https://www.youtube.com/")
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("YouTube network error: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("YouTube HTTP {}", resp.status()));
    }

    let data: Value = resp
        .json()
        .await
        .map_err(|e| format!("YouTube JSON parse error: {}", e))?;

    let mut renderers = Vec::new();
    collect_yt_video_renderers(&data, &mut renderers);

    let mut results = Vec::new();
    for vr in renderers {
        let Some(video_id) = vr
            .get("videoId")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };

        let title = runs_text(vr.get("title"));
        if title.is_empty() {
            continue;
        }

        let artist = {
            let owner = runs_text(vr.get("ownerText"));
            if !owner.is_empty() {
                owner
            } else {
                let byline = runs_text(vr.get("longBylineText"));
                if !byline.is_empty() {
                    byline
                } else {
                    "YouTube".to_string()
                }
            }
        };

        let duration_text = {
            let d = runs_text(vr.get("lengthText"));
            if d.is_empty() {
                None
            } else {
                Some(d)
            }
        };

        let year = {
            let pub_t = runs_text(vr.get("publishedTimeText"));
            if pub_t.is_empty() {
                None
            } else {
                Some(pub_t)
            }
        };

        let thumbnail = vr
            .get("thumbnail")
            .and_then(|t| t.get("thumbnails"))
            .and_then(|arr| arr.as_array())
            .and_then(|arr| arr.last())
            .and_then(|item| item.get("url"))
            .and_then(|u| u.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| {
                if s.starts_with("//") {
                    format!("https:{}", s)
                } else {
                    s.to_string()
                }
            });

        results.push(MusicSearchResult {
            id: format!("yt_vid_{}", video_id),
            source: MusicSearchSource::Youtube,
            kind: MusicSearchItemKind::Video,
            release_type: None,
            title,
            artist,
            album: None,
            year,
            duration_text,
            url: format!("https://www.youtube.com/watch?v={}", video_id),
            thumbnail,
        });

        if results.len() >= 15 {
            break;
        }
    }

    Ok(results)
}

fn format_secs(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{}:{:02}:{:02}", h, m, s)
    } else {
        format!("{:02}:{:02}", m, s)
    }
}

async fn search_dailymotion_videos(
    client: &reqwest::Client,
    query: &str,
) -> Result<Vec<MusicSearchResult>, String> {
    let resp = client
        .get("https://api.dailymotion.com/videos")
        .query(&[
            ("search", query),
            (
                "fields",
                "id,title,owner.screenname,duration,thumbnail_360_url,url",
            ),
            ("limit", "10"),
        ])
        .header("User-Agent", BROWSER_UA)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("Dailymotion network error: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("Dailymotion HTTP {}", resp.status()));
    }

    let data: Value = resp
        .json()
        .await
        .map_err(|e| format!("Dailymotion JSON parse error: {}", e))?;

    let list = data
        .get("list")
        .and_then(|l| l.as_array())
        .cloned()
        .unwrap_or_default();

    let mut results = Vec::new();
    for item in list {
        let Some(id) = item
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let title = item
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if title.is_empty() {
            continue;
        }
        let url = item
            .get("url")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("https://www.dailymotion.com/video/{}", id));

        let artist = item
            .get("owner.screenname")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("Dailymotion")
            .to_string();

        let duration_text = item
            .get("duration")
            .and_then(|v| v.as_u64())
            .filter(|&d| d > 0)
            .map(format_secs);

        let thumbnail = item
            .get("thumbnail_360_url")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string);

        results.push(MusicSearchResult {
            id: format!("dm_vid_{}", id),
            source: MusicSearchSource::Dailymotion,
            kind: MusicSearchItemKind::Video,
            release_type: None,
            title,
            artist,
            album: None,
            year: None,
            duration_text,
            url,
            thumbnail,
        });
    }

    Ok(results)
}

fn percent_encode_path(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 2);
    for b in input.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char);
            }
            b => {
                out.push('%');
                out.push_str(&format!("{:02X}", b));
            }
        }
    }
    out
}

fn decode_html_entities(s: &str) -> String {
    let mut out = s
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#039;", "'")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ");
    while let Some(pos) = out.find("&#") {
        let rest = &out[pos + 2..];
        if let Some(semi) = rest.find(';') {
            if semi > 0 && semi <= 8 {
                let code = &rest[..semi];
                let parsed = if let Some(hex) = code.strip_prefix('x').or_else(|| code.strip_prefix('X')) {
                    u32::from_str_radix(hex, 16).ok()
                } else {
                    code.parse::<u32>().ok()
                };
                if let Some(ch) = parsed.and_then(char::from_u32) {
                    let mut buf = [0u8; 4];
                    let rep = ch.encode_utf8(&mut buf);
                    out.replace_range(pos..pos + 2 + semi + 1, rep);
                    continue;
                }
            }
        }
        break;
    }
    out
}

fn strip_html_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    decode_html_entities(out.trim())
}

fn extract_attr(chunk: &str, attr_prefix: &str) -> Option<String> {
    let idx = chunk.find(attr_prefix)?;
    let after = &chunk[idx + attr_prefix.len()..];
    let end = after.find('"')?;
    Some(after[..end].trim().to_string()).filter(|s| !s.is_empty())
}

fn extract_between<'a>(chunk: &'a str, start_marker: &str, end_marker: &str) -> Option<&'a str> {
    let idx = chunk.find(start_marker)?;
    let after = &chunk[idx + start_marker.len()..];
    let end = after.find(end_marker)?;
    Some(&after[..end])
}

async fn search_videa_videos(
    client: &reqwest::Client,
    query: &str,
) -> Result<Vec<MusicSearchResult>, String> {
    let encoded = percent_encode_path(query.trim());
    let url = format!("https://videa.hu/kereses/{}", encoded);

    let resp = client
        .get(&url)
        .header("User-Agent", BROWSER_UA)
        .header("Accept", "text/html")
        .send()
        .await
        .map_err(|e| format!("Videa network error: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("Videa HTTP {}", resp.status()));
    }

    let html = resp
        .text()
        .await
        .map_err(|e| format!("Videa read error: {}", e))?;

    let mut results = Vec::new();
    let mut search_from = 0usize;

    while let Some(rel) = html[search_from..].find("<h2 class=\"title\">") {
        let h2_pos = search_from + rel;
        search_from = h2_pos + 18;

        let h2_end = html[h2_pos..].find("</h2>").map(|d| h2_pos + d).unwrap_or(h2_pos);
        let h2_block = &html[h2_pos..h2_end];

        let Some(video_url) = extract_attr(h2_block, "href=\"") else {
            continue;
        };
        if !video_url.contains("videa.hu/videok/") {
            continue;
        }

        let title = extract_attr(h2_block, "title=\"")
            .map(|t| decode_html_entities(&t))
            .unwrap_or_else(|| strip_html_tags(h2_block));
        if title.is_empty() {
            continue;
        }

        let mut before_start = h2_pos.saturating_sub(750);
        while before_start < h2_pos && !html.is_char_boundary(before_start) {
            before_start += 1;
        }
        let before = &html[before_start..h2_pos];
        let mut after_end = (h2_pos + 650).min(html.len());
        while after_end < html.len() && !html.is_char_boundary(after_end) {
            after_end += 1;
        }
        let after = &html[h2_pos..after_end];

        let thumbnail = before
            .rsplit_once("data-image=\"")
            .and_then(|(_, rest)| rest.split('"').next())
            .map(str::trim)
            .filter(|s| s.starts_with("http"))
            .map(ToString::to_string);

        let duration_text = before
            .rsplit_once("<div class=\"length\">")
            .and_then(|(_, rest)| rest.split("</div>").next())
            .map(strip_html_tags)
            .filter(|s| !s.is_empty());

        let artist = extract_between(after, "<div class=\"uploader\">", "</div>")
            .map(strip_html_tags)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Videa.hu".to_string());

        results.push(MusicSearchResult {
            id: format!("videa_vid_{}", video_url),
            source: MusicSearchSource::Videa,
            kind: MusicSearchItemKind::Video,
            release_type: None,
            title,
            artist,
            album: None,
            year: None,
            duration_text,
            url: video_url,
            thumbnail,
        });

        if results.len() >= 10 {
            break;
        }
    }

    Ok(results)
}

async fn search_okru_videos(
    client: &reqwest::Client,
    query: &str,
) -> Result<Vec<MusicSearchResult>, String> {
    let slug = query
        .split(|c: char| c.is_whitespace() || c == '/' || c == '\\')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        return Ok(Vec::new());
    }

    let url = format!(
        "https://ok.ru/search/content/{}",
        percent_encode_path(&slug)
    );

    let resp = client
        .get(&url)
        .header("User-Agent", BROWSER_UA)
        .header("Accept", "text/html")
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .await
        .map_err(|e| format!("ok.ru network error: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("ok.ru HTTP {}", resp.status()));
    }

    let html = resp
        .text()
        .await
        .map_err(|e| format!("ok.ru read error: {}", e))?;

    let marker = "<anonym-content-search data-props=\"";
    let Some(m_idx) = html.find(marker) else {
        return Ok(Vec::new());
    };
    let start = m_idx + marker.len();
    let Some(rel_end) = html[start..].find('"') else {
        return Ok(Vec::new());
    };
    let raw_attr = &html[start..start + rel_end];
    let decoded_json = raw_attr
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">");

    let data: Value = serde_json::from_str(&decoded_json)
        .map_err(|e| format!("ok.ru JSON parse error: {}", e))?;

    let list = data
        .get("searchResult")
        .and_then(|sr| sr.get("videos"))
        .and_then(|v| v.get("content"))
        .and_then(|c| c.get("list"))
        .and_then(|l| l.as_array())
        .cloned()
        .unwrap_or_default();

    let mut results = Vec::new();
    for item in list {
        let Some(movie) = item.get("movie") else {
            continue;
        };
        if movie.get("blocked").and_then(|b| b.as_bool()) == Some(true) {
            continue;
        }
        let Some(id) = movie
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };

        let raw_title = item
            .get("name")
            .and_then(|v| v.as_str())
            .or_else(|| movie.get("title").and_then(|v| v.as_str()))
            .unwrap_or("")
            .trim();
        let title = decode_html_entities(raw_title);
        if title.is_empty() {
            continue;
        }

        let raw_href = movie
            .get("href")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("");
        let video_url = if raw_href.starts_with("http://") || raw_href.starts_with("https://") {
            raw_href.to_string()
        } else if raw_href.starts_with('/') {
            format!("https://ok.ru{}", raw_href)
        } else {
            format!("https://ok.ru/video/{}", id)
        };

        let artist = item
            .get("owner")
            .and_then(|o| {
                o.get("album")
                    .and_then(|a| a.get("name"))
                    .or_else(|| o.get("group").and_then(|g| g.get("name")))
                    .or_else(|| o.get("user").and_then(|u| u.get("name")))
                    .or_else(|| o.get("name"))
            })
            .and_then(|v| v.as_str())
            .map(decode_html_entities)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "ok.ru".to_string());

        let duration_text = movie
            .get("duration")
            .and_then(|v| v.as_u64())
            .filter(|&ms| ms >= 1000)
            .map(|ms| format_secs(ms / 1000));

        let thumbnail = movie
            .get("thumbnail")
            .and_then(|t| {
                t.get("big")
                    .or_else(|| t.get("small"))
                    .and_then(|v| v.as_str())
            })
            .or_else(|| item.get("imageUrl").and_then(|v| v.as_str()))
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| {
                if s.starts_with("//") {
                    format!("https:{}", s)
                } else {
                    s.to_string()
                }
            });

        results.push(MusicSearchResult {
            id: format!("okru_vid_{}", id),
            source: MusicSearchSource::OkRu,
            kind: MusicSearchItemKind::Video,
            release_type: None,
            title,
            artist,
            album: None,
            year: None,
            duration_text,
            url: video_url,
            thumbnail,
        });

        if results.len() >= 10 {
            break;
        }
    }

    Ok(results)
}

pub async fn search_all_supported(query: String) -> Result<Vec<MusicSearchResult>, String> {
    let trimmed = query.trim().to_string();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .build()
        .map_err(|e| e.to_string())?;

    let (yt_vid_res, ytm_albums_res, ytm_songs_res, bc_res, dm_res, videa_res, okru_res) = tokio::join!(
        search_youtube_videos(&client, &trimmed),
        search_ytm_kind(&client, &trimmed, true),
        search_ytm_kind(&client, &trimmed, false),
        search_bandcamp(&client, &trimmed),
        search_dailymotion_videos(&client, &trimmed),
        search_videa_videos(&client, &trimmed),
        search_okru_videos(&client, &trimmed)
    );

    let mut errors = Vec::new();
    let yt_videos = match yt_vid_res {
        Ok(v) => v,
        Err(e) => {
            errors.push(e);
            Vec::new()
        }
    };
    let ytm_albums = match ytm_albums_res {
        Ok(v) => v,
        Err(e) => {
            errors.push(e);
            Vec::new()
        }
    };
    let ytm_songs = match ytm_songs_res {
        Ok(v) => v,
        Err(e) => {
            errors.push(e);
            Vec::new()
        }
    };
    let (bc_albums, bc_songs) = match bc_res {
        Ok(v) => v,
        Err(e) => {
            errors.push(e);
            (Vec::new(), Vec::new())
        }
    };
    let dm_videos = match dm_res {
        Ok(v) => v,
        Err(e) => {
            errors.push(e);
            Vec::new()
        }
    };
    let videa_videos = match videa_res {
        Ok(v) => v,
        Err(e) => {
            errors.push(e);
            Vec::new()
        }
    };
    let okru_videos = match okru_res {
        Ok(v) => v,
        Err(e) => {
            errors.push(e);
            Vec::new()
        }
    };

    if errors.len() >= 7 {
        return Err(errors.join(" | "));
    }

    // Interleave all platforms and kinds (Videos, Albums, Songs) cleanly
    let mut merged = Vec::new();
    let mut seen_urls = HashSet::new();
    let buckets: [&[MusicSearchResult]; 8] = [
        &yt_videos,
        &ytm_albums,
        &ytm_songs,
        &bc_albums,
        &bc_songs,
        &dm_videos,
        &videa_videos,
        &okru_videos,
    ];
    let max_len = buckets.iter().map(|b| b.len()).max().unwrap_or(0);

    for idx in 0..max_len {
        for bucket in &buckets {
            if let Some(item) = bucket.get(idx) {
                let norm = item.url.trim().trim_end_matches('/').to_ascii_lowercase();
                if seen_urls.insert(norm) {
                    merged.push(item.clone());
                }
            }
        }
    }

    Ok(merged)
}
