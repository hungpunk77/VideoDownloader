use crate::types::{
    sanitize_bandcamp_url, MusicSearchItemKind, MusicSearchResult, MusicSearchSource, SearchBatch,
    SearchPagination,
};
use serde_json::{json, Value};
use std::collections::HashSet;

const YTM_SONGS_PARAMS: &str = "EgWKAQIIAWoKEAMQBBAJEAoQBQ%3D%3D";
const YTM_ALBUMS_PARAMS: &str = "EgWKAQIYAWoKEAMQBBAJEAoQBQ%3D%3D";

const BROWSER_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36";

fn build_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())
}

fn normalize_result_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_ascii_lowercase()
}

fn interleave_and_dedup(
    buckets: &[&[MusicSearchResult]],
    seen_urls: &mut HashSet<String>,
) -> Vec<MusicSearchResult> {
    let mut merged = Vec::new();
    let max_len = buckets.iter().map(|b| b.len()).max().unwrap_or(0);
    for idx in 0..max_len {
        for bucket in buckets {
            if let Some(item) = bucket.get(idx) {
                let norm = normalize_result_url(&item.url);
                if seen_urls.insert(norm) {
                    merged.push(item.clone());
                }
            }
        }
    }
    merged
}

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

fn extract_ytm_continuation(obj: &Value) -> Option<String> {
    obj.get("continuations")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|item| item.get("nextContinuationData"))
        .and_then(|ncd| ncd.get("continuation"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

fn parse_ytm_items(contents: &[Value], is_album: bool, out: &mut Vec<MusicSearchResult>) {
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
            out.push(MusicSearchResult {
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
            out.push(MusicSearchResult {
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
    }
}

async fn search_ytm_kind_paged(
    client: &reqwest::Client,
    query: &str,
    is_album: bool,
    start_continuation: Option<String>,
    max_pages: usize,
) -> Result<(Vec<MusicSearchResult>, Option<String>), String> {
    let params = if is_album {
        YTM_ALBUMS_PARAMS
    } else {
        YTM_SONGS_PARAMS
    };

    let mut results = Vec::new();
    let mut seen_ids = HashSet::new();
    let mut current_cont = start_continuation;

    for page_idx in 0..max_pages {
        if page_idx > 0 && current_cont.is_none() {
            break;
        }

        let payload = if let Some(tok) = current_cont.as_deref() {
            json!({
                "context": {
                    "client": {
                        "clientName": "WEB_REMIX",
                        "clientVersion": "1.20250101.01.00",
                        "hl": "en",
                        "gl": "US"
                    }
                },
                "continuation": tok
            })
        } else {
            json!({
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
            })
        };

        let resp = match client
            .post("https://music.youtube.com/youtubei/v1/search?prettyPrint=false")
            .header("User-Agent", BROWSER_UA)
            .header("Origin", "https://music.youtube.com")
            .header("Referer", "https://music.youtube.com/")
            .json(&payload)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                if page_idx == 0 {
                    return Err(format!("YouTube Music network error: {}", e));
                }
                break;
            }
        };

        if !resp.status().is_success() {
            if page_idx == 0 {
                return Err(format!("YouTube Music HTTP {}", resp.status()));
            }
            break;
        }

        let data: Value = match resp.json().await {
            Ok(d) => d,
            Err(e) => {
                if page_idx == 0 {
                    return Err(format!("YouTube Music JSON parse error: {}", e));
                }
                break;
            }
        };

        let mut page_items = Vec::new();
        let mut next_token: Option<String> = None;

        if current_cont.is_some() {
            if let Some(msc) = data
                .get("continuationContents")
                .and_then(|c| c.get("musicShelfContinuation"))
            {
                if let Some(contents) = msc.get("contents").and_then(|c| c.as_array()) {
                    parse_ytm_items(contents, is_album, &mut page_items);
                }
                next_token = extract_ytm_continuation(msc);
            }
        } else {
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

            for shelf in shelves {
                if let Some(contents) = shelf.get("contents").and_then(|c| c.as_array()) {
                    parse_ytm_items(contents, is_album, &mut page_items);
                }
                if next_token.is_none() {
                    next_token = extract_ytm_continuation(shelf);
                }
            }
        }

        let mut added = 0usize;
        for item in page_items {
            if seen_ids.insert(item.id.clone()) {
                results.push(item);
                added += 1;
            }
        }

        current_cont = if added > 0 { next_token } else { None };
        if current_cont.is_none() {
            break;
        }
    }

    Ok((results, current_cont))
}

fn extract_bandcamp_thumbnail(item: &Value) -> Option<String> {
    if let Some(art_id) = item
        .get("art_id")
        .and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_str().and_then(|s| s.trim().parse::<u64>().ok()))
        })
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
) -> Result<SearchBatch, String> {
    let trimmed = query.trim().to_string();
    if trimmed.is_empty() {
        return Ok(SearchBatch {
            results: Vec::new(),
            pagination: SearchPagination::default(),
        });
    }
    if !include_ytm && !include_bandcamp {
        return Err("Select at least one search source (YouTube Music or Bandcamp).".to_string());
    }

    let client = build_client()?;

    let (ytm_albums_res, ytm_songs_res, bc_res) = tokio::join!(
        async {
            if include_ytm {
                search_ytm_kind_paged(&client, &trimmed, true, None, 3).await
            } else {
                Ok((Vec::new(), None))
            }
        },
        async {
            if include_ytm {
                search_ytm_kind_paged(&client, &trimmed, false, None, 3).await
            } else {
                Ok((Vec::new(), None))
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
    let (ytm_albums, ytm_albums_cont) = match ytm_albums_res {
        Ok(v) => v,
        Err(e) => {
            errors.push(e);
            (Vec::new(), None)
        }
    };
    let (ytm_songs, ytm_songs_cont) = match ytm_songs_res {
        Ok(v) => v,
        Err(e) => {
            errors.push(e);
            (Vec::new(), None)
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

    let mut seen_urls = HashSet::new();
    let buckets: [&[MusicSearchResult]; 4] = [&ytm_albums, &ytm_songs, &bc_albums, &bc_songs];
    let merged = interleave_and_dedup(&buckets, &mut seen_urls);

    let ytm_has_more = ytm_albums_cont.is_some() || ytm_songs_cont.is_some();
    let pagination = SearchPagination {
        query: trimmed,
        include_ytm,
        include_bandcamp,
        ytm_songs_continuation: ytm_songs_cont,
        ytm_albums_continuation: ytm_albums_cont,
        ytm_has_more,
        ..Default::default()
    };

    Ok(SearchBatch {
        results: merged,
        pagination,
    })
}

pub async fn load_more_music(
    mut pagination: SearchPagination,
    mut existing_urls: HashSet<String>,
) -> Result<SearchBatch, String> {
    if !pagination.include_ytm || !pagination.ytm_has_more {
        return Ok(SearchBatch {
            results: Vec::new(),
            pagination,
        });
    }

    let client = build_client()?;
    let query = pagination.query.clone();
    let albums_tok = pagination.ytm_albums_continuation.clone();
    let songs_tok = pagination.ytm_songs_continuation.clone();

    let attempted = usize::from(albums_tok.is_some()) + usize::from(songs_tok.is_some());

    let (albums_res, songs_res) = tokio::join!(
        async {
            if let Some(tok) = albums_tok {
                search_ytm_kind_paged(&client, &query, true, Some(tok), 3).await
            } else {
                Ok((Vec::new(), None))
            }
        },
        async {
            if let Some(tok) = songs_tok {
                search_ytm_kind_paged(&client, &query, false, Some(tok), 3).await
            } else {
                Ok((Vec::new(), None))
            }
        }
    );

    let mut errors = Vec::new();
    let ytm_albums = match albums_res {
        Ok((v, next_tok)) => {
            pagination.ytm_albums_continuation = next_tok;
            v
        }
        Err(e) => {
            errors.push(e);
            Vec::new()
        }
    };
    let ytm_songs = match songs_res {
        Ok((v, next_tok)) => {
            pagination.ytm_songs_continuation = next_tok;
            v
        }
        Err(e) => {
            errors.push(e);
            Vec::new()
        }
    };

    if attempted > 0 && errors.len() >= attempted {
        return Err(errors.join(" | "));
    }

    pagination.ytm_has_more =
        pagination.ytm_albums_continuation.is_some() || pagination.ytm_songs_continuation.is_some();

    let buckets: [&[MusicSearchResult]; 2] = [&ytm_albums, &ytm_songs];
    let merged = interleave_and_dedup(&buckets, &mut existing_urls);

    Ok(SearchBatch {
        results: merged,
        pagination,
    })
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

fn extract_yt_cont_from_items(items: &[Value]) -> Option<String> {
    items.iter().find_map(|item| {
        item.get("continuationItemRenderer")
            .and_then(|ci| ci.get("continuationEndpoint"))
            .and_then(|ce| ce.get("continuationCommand"))
            .and_then(|cc| cc.get("token"))
            .and_then(|t| t.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
    })
}

fn extract_yt_main_continuation(data: &Value) -> Option<String> {
    if let Some(contents) = data
        .get("contents")
        .and_then(|c| c.get("twoColumnSearchResultsRenderer"))
        .and_then(|t| t.get("primaryContents"))
        .and_then(|p| p.get("sectionListRenderer"))
        .and_then(|s| s.get("contents"))
        .and_then(|a| a.as_array())
    {
        if let Some(tok) = extract_yt_cont_from_items(contents) {
            return Some(tok);
        }
    }

    if let Some(cmds) = data.get("onResponseReceivedCommands").and_then(|a| a.as_array()) {
        for cmd in cmds {
            if let Some(items) = cmd
                .get("appendContinuationItemsAction")
                .and_then(|a| a.get("continuationItems"))
                .and_then(|a| a.as_array())
            {
                if let Some(tok) = extract_yt_cont_from_items(items) {
                    return Some(tok);
                }
            }
        }
    }

    None
}

async fn search_youtube_videos_paged(
    client: &reqwest::Client,
    query: &str,
    start_continuation: Option<String>,
    max_pages: usize,
) -> Result<(Vec<MusicSearchResult>, Option<String>), String> {
    let mut results = Vec::new();
    let mut seen_ids = HashSet::new();
    let mut current_cont = start_continuation;

    for page_idx in 0..max_pages {
        if page_idx > 0 && current_cont.is_none() {
            break;
        }

        let payload = if let Some(tok) = current_cont.as_deref() {
            json!({
                "context": {
                    "client": {
                        "clientName": "WEB",
                        "clientVersion": "2.20250101.00.00",
                        "hl": "en",
                        "gl": "US"
                    }
                },
                "continuation": tok
            })
        } else {
            json!({
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
            })
        };

        let resp = match client
            .post("https://www.youtube.com/youtubei/v1/search?prettyPrint=false")
            .header("User-Agent", BROWSER_UA)
            .header("Origin", "https://www.youtube.com")
            .header("Referer", "https://www.youtube.com/")
            .json(&payload)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                if page_idx == 0 {
                    return Err(format!("YouTube network error: {}", e));
                }
                break;
            }
        };

        if !resp.status().is_success() {
            if page_idx == 0 {
                return Err(format!("YouTube HTTP {}", resp.status()));
            }
            break;
        }

        let data: Value = match resp.json().await {
            Ok(d) => d,
            Err(e) => {
                if page_idx == 0 {
                    return Err(format!("YouTube JSON parse error: {}", e));
                }
                break;
            }
        };

        let mut renderers = Vec::new();
        collect_yt_video_renderers(&data, &mut renderers);

        let mut added = 0usize;
        for vr in renderers {
            let Some(video_id) = vr
                .get("videoId")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                continue;
            };

            if !seen_ids.insert(video_id.to_string()) {
                continue;
            }

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
            added += 1;
        }

        let next_token = extract_yt_main_continuation(&data);
        current_cont = if added > 0 { next_token } else { None };
        if current_cont.is_none() {
            break;
        }
    }

    Ok((results, current_cont))
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

async fn search_dailymotion_videos_paged(
    client: &reqwest::Client,
    query: &str,
    start_page: u32,
    max_pages: usize,
) -> Result<(Vec<MusicSearchResult>, u32, Option<usize>, bool), String> {
    let mut results = Vec::new();
    let mut seen_ids = HashSet::new();
    let mut page = start_page.max(1);
    let mut total_count: Option<usize> = None;
    let mut has_more = false;

    for page_idx in 0..max_pages {
        let page_str = page.to_string();
        let resp = match client
            .get("https://api.dailymotion.com/videos")
            .query(&[
                ("search", query),
                (
                    "fields",
                    "id,title,owner.screenname,duration,thumbnail_360_url,url",
                ),
                ("limit", "100"),
                ("page", page_str.as_str()),
            ])
            .header("User-Agent", BROWSER_UA)
            .header("Accept", "application/json")
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                if page_idx == 0 {
                    return Err(format!("Dailymotion network error: {}", e));
                }
                break;
            }
        };

        if !resp.status().is_success() {
            if page_idx == 0 {
                return Err(format!("Dailymotion HTTP {}", resp.status()));
            }
            has_more = false;
            break;
        }

        let data: Value = match resp.json().await {
            Ok(d) => d,
            Err(e) => {
                if page_idx == 0 {
                    return Err(format!("Dailymotion JSON parse error: {}", e));
                }
                break;
            }
        };

        if let Some(total) = data.get("total").and_then(|v| v.as_u64()) {
            total_count = Some(total as usize);
        }
        has_more = data
            .get("has_more")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let list = data
            .get("list")
            .and_then(|l| l.as_array())
            .cloned()
            .unwrap_or_default();

        if list.is_empty() {
            has_more = false;
            break;
        }

        let mut added = 0usize;
        for item in list {
            let Some(id) = item
                .get("id")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                continue;
            };
            if !seen_ids.insert(id.to_string()) {
                continue;
            }

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
            added += 1;
        }

        page += 1;
        if !has_more || added == 0 {
            if added == 0 {
                has_more = false;
            }
            break;
        }
    }

    Ok((results, page, total_count, has_more))
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

fn extract_last_attr(chunk: &str, attr_prefix: &str) -> Option<String> {
    let idx = chunk.rfind(attr_prefix)?;
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

fn parse_videa_html_chunk(html: &str) -> (Vec<MusicSearchResult>, usize, Option<String>) {
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
    }

    let raw_item_count = html.matches("data-item-id=\"").count();
    let last_item_id = extract_last_attr(html, "data-item-id=\"");
    (results, raw_item_count, last_item_id)
}

#[allow(clippy::too_many_arguments)]
async fn search_videa_videos_paged(
    client: &reqwest::Client,
    query: &str,
    init_cache_id: Option<String>,
    init_sort: String,
    init_last_item_id: Option<String>,
    init_item_count: usize,
    init_total: Option<usize>,
    max_pages: usize,
) -> Result<
    (
        Vec<MusicSearchResult>,
        Option<String>,
        String,
        Option<String>,
        usize,
        Option<usize>,
        bool,
    ),
    String,
> {
    let encoded = percent_encode_path(query.trim());
    let mut results = Vec::new();
    let mut seen_urls = HashSet::new();

    let mut cache_id = init_cache_id;
    let resuming_lazy = cache_id.is_some();
    let mut sort = if init_sort.is_empty() {
        "0".to_string()
    } else {
        init_sort
    };
    let mut last_item_id = init_last_item_id;
    let mut item_count = init_item_count;
    let mut total_count = init_total;
    let mut has_more;
    let mut lazy_pages_remaining = max_pages;

    if !resuming_lazy {
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

        if let Some(raw_cnt) = extract_between(&html, "<div class=\"search-page-count\">", "</div>")
        {
            let digits: String = strip_html_tags(raw_cnt)
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let Ok(n) = digits.parse::<usize>() {
                total_count = Some(n);
            }
        }

        cache_id = extract_attr(&html, "data-cache-id=\"").map(|s| decode_html_entities(&s));
        if let Some(s) = extract_attr(&html, "data-sort=\"") {
            sort = s;
        }

        let (page_items, raw_count, chunk_last_id) = parse_videa_html_chunk(&html);
        for item in page_items {
            if seen_urls.insert(item.url.clone()) {
                results.push(item);
            }
        }

        item_count = raw_count;
        last_item_id = chunk_last_id;
        has_more = cache_id.is_some()
            && last_item_id.is_some()
            && raw_count >= 36
            && total_count.map_or(true, |t| item_count < t);
        lazy_pages_remaining = lazy_pages_remaining.saturating_sub(1);
    } else {
        has_more = cache_id.is_some() && last_item_id.is_some();
    }

    for lazy_idx in 0..lazy_pages_remaining {
        if !has_more {
            break;
        }
        let (Some(cid), Some(lid)) = (cache_id.as_deref(), last_item_id.as_deref()) else {
            has_more = false;
            break;
        };

        let lazy_url = format!("https://videa.hu/lazy/kereses/{}", encoded);
        let item_count_str = item_count.to_string();
        let resp = match client
            .get(&lazy_url)
            .query(&[
                ("cacheId", cid),
                ("lastItemId", lid),
                ("itemCount", item_count_str.as_str()),
                ("sort", sort.as_str()),
            ])
            .header("User-Agent", BROWSER_UA)
            .header("Accept", "text/html")
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                if resuming_lazy && lazy_idx == 0 {
                    return Err(format!("Videa HTTP {}", r.status()));
                }
                break;
            }
            Err(e) => {
                if resuming_lazy && lazy_idx == 0 {
                    return Err(format!("Videa network error: {}", e));
                }
                break;
            }
        };

        let html = match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                if resuming_lazy && lazy_idx == 0 {
                    return Err(format!("Videa read error: {}", e));
                }
                break;
            }
        };

        let (chunk_items, raw_count, chunk_last_id) = parse_videa_html_chunk(&html);
        if raw_count == 0 || chunk_last_id.is_none() || chunk_last_id == last_item_id {
            has_more = false;
            break;
        }

        for item in chunk_items {
            if seen_urls.insert(item.url.clone()) {
                results.push(item);
            }
        }

        item_count += raw_count;
        last_item_id = chunk_last_id;
        if raw_count < 36 || total_count.is_some_and(|t| item_count >= t) {
            has_more = false;
            break;
        }
    }

    Ok((
        results,
        cache_id,
        sort,
        last_item_id,
        item_count,
        total_count,
        has_more,
    ))
}

fn parse_okru_list_items(list: &[Value]) -> Vec<MusicSearchResult> {
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
    }
    results
}

async fn search_okru_html_fallback(
    client: &reqwest::Client,
    query: &str,
) -> Result<(Vec<MusicSearchResult>, Option<usize>), String> {
    let slug = query
        .split(|c: char| c.is_whitespace() || c == '/' || c == '\\')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        return Ok((Vec::new(), None));
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
        return Ok((Vec::new(), None));
    };
    let start = m_idx + marker.len();
    let Some(rel_end) = html[start..].find('"') else {
        return Ok((Vec::new(), None));
    };
    let raw_attr = &html[start..start + rel_end];
    let decoded_json = raw_attr
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">");

    let data: Value = serde_json::from_str(&decoded_json)
        .map_err(|e| format!("ok.ru JSON parse error: {}", e))?;

    let content = data
        .get("searchResult")
        .and_then(|sr| sr.get("videos"))
        .and_then(|v| v.get("content"));

    let total_count = content
        .and_then(|c| c.get("totalCount"))
        .and_then(|v| v.as_u64())
        .map(|n| n as usize);

    let list = content
        .and_then(|c| c.get("list"))
        .and_then(|l| l.as_array())
        .cloned()
        .unwrap_or_default();

    Ok((parse_okru_list_items(&list), total_count))
}

async fn search_okru_videos_paged(
    client: &reqwest::Client,
    query: &str,
    start_offset: usize,
    max_pages: usize,
) -> Result<(Vec<MusicSearchResult>, usize, Option<usize>, bool), String> {
    let mut results = Vec::new();
    let mut seen_ids = HashSet::new();
    let mut offset = start_offset;
    let mut total_count: Option<usize> = None;
    let mut has_more = false;

    for page_idx in 0..max_pages {
        let payload = json!({
            "id": page_idx + 1,
            "parameters": {
                "displayMode": "Movie",
                "videosOffset": offset,
                "channelsOffset": 0,
                "searchQuery": query,
                "currentStateId": "anonymVideo"
            }
        });

        let api_res = client
            .post("https://ok.ru/web-api/v2/video/fetchSearchResult")
            .header("User-Agent", BROWSER_UA)
            .header("Origin", "https://ok.ru")
            .header("Referer", "https://ok.ru/")
            .header("Accept", "application/json")
            .header("Accept-Language", "en-US,en;q=0.9")
            .json(&payload)
            .send()
            .await;

        let resp = match api_res {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                if page_idx == 0 && start_offset == 0 {
                    let (fb_items, fb_total) = search_okru_html_fallback(client, query).await?;
                    let fb_len = fb_items.len();
                    return Ok((fb_items, fb_len, fb_total, false));
                }
                if page_idx == 0 {
                    return Err(format!("ok.ru HTTP {}", r.status()));
                }
                break;
            }
            Err(e) => {
                if page_idx == 0 && start_offset == 0 {
                    let (fb_items, fb_total) = search_okru_html_fallback(client, query).await?;
                    let fb_len = fb_items.len();
                    return Ok((fb_items, fb_len, fb_total, false));
                }
                if page_idx == 0 {
                    return Err(format!("ok.ru network error: {}", e));
                }
                break;
            }
        };

        let data: Value = match resp.json().await {
            Ok(d) => d,
            Err(e) => {
                if page_idx == 0 && start_offset == 0 {
                    let (fb_items, fb_total) = search_okru_html_fallback(client, query).await?;
                    let fb_len = fb_items.len();
                    return Ok((fb_items, fb_len, fb_total, false));
                }
                if page_idx == 0 {
                    return Err(format!("ok.ru JSON parse error: {}", e));
                }
                break;
            }
        };

        if data.get("success").and_then(|v| v.as_bool()) != Some(true) {
            if page_idx == 0 && start_offset == 0 {
                let (fb_items, fb_total) = search_okru_html_fallback(client, query).await?;
                let fb_len = fb_items.len();
                return Ok((fb_items, fb_len, fb_total, false));
            }
            break;
        }

        let Some(videos) = data.get("result").and_then(|r| r.get("videos")) else {
            has_more = false;
            break;
        };

        if let Some(t) = videos.get("totalCount").and_then(|v| v.as_u64()) {
            total_count = Some(t as usize);
        }
        has_more = videos
            .get("hasMore")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let list = videos
            .get("list")
            .and_then(|l| l.as_array())
            .cloned()
            .unwrap_or_default();

        let raw_len = list.len();
        if raw_len == 0 {
            has_more = false;
            break;
        }

        for item in parse_okru_list_items(&list) {
            if seen_ids.insert(item.id.clone()) {
                results.push(item);
            }
        }

        offset += raw_len;
        if !has_more {
            break;
        }
    }

    Ok((results, offset, total_count, has_more))
}

pub async fn search_all_supported(query: String) -> Result<SearchBatch, String> {
    let trimmed = query.trim().to_string();
    if trimmed.is_empty() {
        return Ok(SearchBatch {
            results: Vec::new(),
            pagination: SearchPagination::default(),
        });
    }

    let client = build_client()?;

    let (yt_vid_res, ytm_albums_res, ytm_songs_res, bc_res, dm_res, videa_res, okru_res) = tokio::join!(
        search_youtube_videos_paged(&client, &trimmed, None, 3),
        search_ytm_kind_paged(&client, &trimmed, true, None, 3),
        search_ytm_kind_paged(&client, &trimmed, false, None, 3),
        search_bandcamp(&client, &trimmed),
        search_dailymotion_videos_paged(&client, &trimmed, 1, 2),
        search_videa_videos_paged(&client, &trimmed, None, "0".to_string(), None, 0, None, 4),
        search_okru_videos_paged(&client, &trimmed, 0, 5)
    );

    let mut errors = Vec::new();
    let (yt_videos, yt_cont) = match yt_vid_res {
        Ok(v) => v,
        Err(e) => {
            errors.push(e);
            (Vec::new(), None)
        }
    };
    let (ytm_albums, ytm_albums_cont) = match ytm_albums_res {
        Ok(v) => v,
        Err(e) => {
            errors.push(e);
            (Vec::new(), None)
        }
    };
    let (ytm_songs, ytm_songs_cont) = match ytm_songs_res {
        Ok(v) => v,
        Err(e) => {
            errors.push(e);
            (Vec::new(), None)
        }
    };
    let (bc_albums, bc_songs) = match bc_res {
        Ok(v) => v,
        Err(e) => {
            errors.push(e);
            (Vec::new(), Vec::new())
        }
    };
    let (dm_videos, dm_next_page, dm_total, dm_has_more) = match dm_res {
        Ok(v) => v,
        Err(e) => {
            errors.push(e);
            (Vec::new(), 1, None, false)
        }
    };
    let (
        videa_videos,
        videa_cache_id,
        videa_sort,
        videa_last_item_id,
        videa_item_count,
        videa_total,
        videa_has_more,
    ) = match videa_res {
        Ok(v) => v,
        Err(e) => {
            errors.push(e);
            (Vec::new(), None, "0".to_string(), None, 0, None, false)
        }
    };
    let (okru_videos, okru_offset, okru_total, okru_has_more) = match okru_res {
        Ok(v) => v,
        Err(e) => {
            errors.push(e);
            (Vec::new(), 0, None, false)
        }
    };

    if errors.len() >= 7 {
        return Err(errors.join(" | "));
    }

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
    let merged = interleave_and_dedup(&buckets, &mut seen_urls);

    let pagination = SearchPagination {
        query: trimmed,
        include_ytm: true,
        include_bandcamp: true,
        yt_has_more: yt_cont.is_some(),
        yt_continuation: yt_cont,
        ytm_has_more: ytm_albums_cont.is_some() || ytm_songs_cont.is_some(),
        ytm_songs_continuation: ytm_songs_cont,
        ytm_albums_continuation: ytm_albums_cont,
        dm_next_page,
        dm_total,
        dm_has_more,
        okru_offset,
        okru_total,
        okru_has_more,
        videa_cache_id,
        videa_sort,
        videa_last_item_id,
        videa_item_count,
        videa_total,
        videa_has_more,
    };

    Ok(SearchBatch {
        results: merged,
        pagination,
    })
}

pub async fn load_more_all_supported(
    mut pagination: SearchPagination,
    source_filter: Option<MusicSearchSource>,
    mut existing_urls: HashSet<String>,
) -> Result<SearchBatch, String> {
    if !pagination.has_more_for_source(source_filter) {
        return Ok(SearchBatch {
            results: Vec::new(),
            pagination,
        });
    }

    let client = build_client()?;
    let query = pagination.query.clone();

    let single_source = source_filter.is_some();
    let want_yt = matches!(source_filter, None | Some(MusicSearchSource::Youtube))
        && pagination.yt_has_more;
    let want_ytm = matches!(source_filter, None | Some(MusicSearchSource::YoutubeMusic))
        && pagination.ytm_has_more;
    let want_dm = matches!(source_filter, None | Some(MusicSearchSource::Dailymotion))
        && pagination.dm_has_more;
    let want_videa = matches!(source_filter, None | Some(MusicSearchSource::Videa))
        && pagination.videa_has_more;
    let want_okru = matches!(source_filter, None | Some(MusicSearchSource::OkRu))
        && pagination.okru_has_more;

    let yt_pages = if single_source { 4 } else { 2 };
    let ytm_pages = if single_source { 4 } else { 2 };
    let dm_pages = if single_source { 3 } else { 2 };
    let videa_pages = if single_source { 5 } else { 3 };
    let okru_pages = if single_source { 10 } else { 5 };

    let yt_tok = pagination.yt_continuation.clone();
    let ytm_albums_tok = pagination.ytm_albums_continuation.clone();
    let ytm_songs_tok = pagination.ytm_songs_continuation.clone();
    let dm_page = pagination.dm_next_page;
    let videa_cid = pagination.videa_cache_id.clone();
    let videa_sort = pagination.videa_sort.clone();
    let videa_last = pagination.videa_last_item_id.clone();
    let videa_cnt = pagination.videa_item_count;
    let videa_tot = pagination.videa_total;
    let okru_off = pagination.okru_offset;

    let (yt_res, ytm_albums_res, ytm_songs_res, dm_res, videa_res, okru_res) = tokio::join!(
        async {
            if want_yt {
                if let Some(tok) = yt_tok {
                    Some(search_youtube_videos_paged(&client, &query, Some(tok), yt_pages).await)
                } else {
                    None
                }
            } else {
                None
            }
        },
        async {
            if want_ytm {
                if let Some(tok) = ytm_albums_tok {
                    Some(search_ytm_kind_paged(&client, &query, true, Some(tok), ytm_pages).await)
                } else {
                    None
                }
            } else {
                None
            }
        },
        async {
            if want_ytm {
                if let Some(tok) = ytm_songs_tok {
                    Some(search_ytm_kind_paged(&client, &query, false, Some(tok), ytm_pages).await)
                } else {
                    None
                }
            } else {
                None
            }
        },
        async {
            if want_dm {
                Some(search_dailymotion_videos_paged(&client, &query, dm_page, dm_pages).await)
            } else {
                None
            }
        },
        async {
            if want_videa {
                Some(
                    search_videa_videos_paged(
                        &client,
                        &query,
                        videa_cid,
                        videa_sort,
                        videa_last,
                        videa_cnt,
                        videa_tot,
                        videa_pages,
                    )
                    .await,
                )
            } else {
                None
            }
        },
        async {
            if want_okru {
                Some(search_okru_videos_paged(&client, &query, okru_off, okru_pages).await)
            } else {
                None
            }
        }
    );

    let mut errors = Vec::new();
    let mut attempted = 0usize;

    let mut yt_videos = Vec::new();
    if let Some(res) = yt_res {
        attempted += 1;
        match res {
            Ok((v, next_tok)) => {
                yt_videos = v;
                pagination.yt_has_more = next_tok.is_some();
                pagination.yt_continuation = next_tok;
            }
            Err(e) => errors.push(e),
        }
    }

    let mut ytm_albums = Vec::new();
    if let Some(res) = ytm_albums_res {
        attempted += 1;
        match res {
            Ok((v, next_tok)) => {
                ytm_albums = v;
                pagination.ytm_albums_continuation = next_tok;
            }
            Err(e) => errors.push(e),
        }
    }

    let mut ytm_songs = Vec::new();
    if let Some(res) = ytm_songs_res {
        attempted += 1;
        match res {
            Ok((v, next_tok)) => {
                ytm_songs = v;
                pagination.ytm_songs_continuation = next_tok;
            }
            Err(e) => errors.push(e),
        }
    }
    pagination.ytm_has_more =
        pagination.ytm_albums_continuation.is_some() || pagination.ytm_songs_continuation.is_some();

    let mut dm_videos = Vec::new();
    if let Some(res) = dm_res {
        attempted += 1;
        match res {
            Ok((v, next_page, total, has_more)) => {
                dm_videos = v;
                pagination.dm_next_page = next_page;
                if total.is_some_and(|t| t > 0) {
                    pagination.dm_total = total;
                }
                pagination.dm_has_more = has_more;
            }
            Err(e) => errors.push(e),
        }
    }

    let mut videa_videos = Vec::new();
    if let Some(res) = videa_res {
        attempted += 1;
        match res {
            Ok((v, cid, srt, lid, cnt, tot, has_more)) => {
                videa_videos = v;
                pagination.videa_cache_id = cid;
                pagination.videa_sort = srt;
                pagination.videa_last_item_id = lid;
                pagination.videa_item_count = cnt;
                if tot.is_some() {
                    pagination.videa_total = tot;
                }
                pagination.videa_has_more = has_more;
            }
            Err(e) => errors.push(e),
        }
    }

    let mut okru_videos = Vec::new();
    if let Some(res) = okru_res {
        attempted += 1;
        match res {
            Ok((v, next_off, total, has_more)) => {
                okru_videos = v;
                pagination.okru_offset = next_off;
                if total.is_some() {
                    pagination.okru_total = total;
                }
                pagination.okru_has_more = has_more;
            }
            Err(e) => errors.push(e),
        }
    }

    if attempted > 0 && errors.len() >= attempted {
        return Err(errors.join(" | "));
    }

    let buckets: [&[MusicSearchResult]; 6] = [
        &yt_videos,
        &ytm_albums,
        &ytm_songs,
        &dm_videos,
        &videa_videos,
        &okru_videos,
    ];
    let merged = interleave_and_dedup(&buckets, &mut existing_urls);

    Ok(SearchBatch {
        results: merged,
        pagination,
    })
}
