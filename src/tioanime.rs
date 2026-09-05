//! Experimental TioAnime provider.
//!
//! Live behavior verified 2026-09-04 against `https://tioanime.com`
//! (Black Torch + Dragon Ball Super Latino as language witnesses):
//! - search: `POST {base}/api/search` with form `value`, returning
//!   `[{id, title, type, slug}]`.
//! - anime page: `GET {base}/anime/<slug>/`; episodes are embedded as
//!   `var episodes = [9,8,...]` alongside `var anime_info = [id,...]`.
//! - episode page: `GET {base}/ver/<slug>-<n>/`; servers are embedded as
//!   `var videos = [[label, embed_url, ?, risky], ...]` (Black Torch 9:
//!   Mega, Voe, YourUpload).
//! - YourUpload embeds serve a page with `<meta property="og:video">` (also
//!   the jwplayer `file:`) pointing at a direct MP4 on `vidcache.net`. That
//!   hand-off answers 302 to an edge URL and **requires the embed page as
//!   `Referer`** (verified: no Referer → HTTP 500; with Referer → 200
//!   video/mp4, ranges OK). The token path is date-stamped, so stream URLs
//!   are temporary: resolve close to playback, never persist.
//!
//! Language observed: every page says `sub español` / `Subtitulado` — even
//! the `dragon-ball-super-latino` entry and its download table. "Latino"
//! exists only as a title-slug convention for separate catalog entries, with
//! no structured region metadata. There is no `es-419`/`es-ES` distinction,
//! so this provider satisfies generic `--language es` only, with hardcoded
//! subtitles (`StreamLink.subtitles` stays empty).
//!
//! Only the YourUpload embeds resolve to direct media. Mega/Voe/siblings
//! (StreamSB, Okru, Netu, …) need embed-host crypto or JS-unpacking that
//! breaks often (Voe hides behind `eugenemakedraw.com` + obfuscated player;
//! Mega needs its file-key API), so they are surfaced as browser-fallback
//! URLs in the resolution error instead of silent "not implemented" text.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use regex::Regex;
use reqwest::{Client, Response, StatusCode, header};
use serde::Deserialize;
use serde_json::Value;
use url::Url;

use crate::{
    AniError, CatalogProvider, RequestHeaders, Result, SearchOptions, SearchResult, StreamLink,
    TranslationType,
    models::{sort_episodes, sort_streams},
};

// Default base verified live 2026-09-04. Old repository domains must never
// replace this without a new live verification (hard evidence gate).
const DEFAULT_BASE: &str = "https://tioanime.com";
const DEFAULT_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36";
const CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const CACHE_LIMIT: usize = 100;
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct TioAnimeId {
    slug: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
}

#[derive(Clone, Debug)]
struct Cached<T> {
    expires_at: Instant,
    value: T,
}

#[derive(Clone, Debug)]
pub struct TioAnimeClientBuilder {
    base: String,
    user_agent: String,
    timeout: Duration,
}

impl Default for TioAnimeClientBuilder {
    fn default() -> Self {
        Self {
            base: DEFAULT_BASE.into(),
            user_agent: DEFAULT_AGENT.into(),
            timeout: Duration::from_secs(15),
        }
    }
}

impl TioAnimeClientBuilder {
    pub fn base_url(mut self, value: impl Into<String>) -> Self {
        self.base = value.into();
        self
    }

    pub fn timeout(mut self, value: Duration) -> Self {
        self.timeout = value;
        self
    }

    pub fn build(self) -> Result<TioAnimeClient> {
        let http = Client::builder()
            .timeout(self.timeout)
            .user_agent(&self.user_agent)
            .cookie_store(true)
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()?;
        Ok(TioAnimeClient {
            inner: Arc::new(Inner {
                http,
                base: self.base.trim_end_matches('/').into(),
                searches: Mutex::new(HashMap::new()),
            }),
        })
    }
}

struct Inner {
    http: Client,
    base: String,
    searches: Mutex<HashMap<String, Cached<Vec<SearchResult>>>>,
}

#[derive(Clone)]
pub struct TioAnimeClient {
    inner: Arc<Inner>,
}

impl TioAnimeClient {
    pub fn builder() -> TioAnimeClientBuilder {
        TioAnimeClientBuilder::default()
    }

    pub fn new() -> Result<Self> {
        Self::builder().build()
    }

    pub async fn search(&self, query: &str, mode: TranslationType) -> Result<Vec<SearchResult>> {
        self.search_with_options(query, mode, SearchOptions::default())
            .await
    }

    pub async fn search_with_options(
        &self,
        query: &str,
        _mode: TranslationType,
        _options: SearchOptions,
    ) -> Result<Vec<SearchResult>> {
        // TioAnime serves a single "sub español" catalog: there is no
        // sub/dub split to filter on, so mode and options are accepted only
        // to match the existing provider call shape.
        let query = query.trim();
        if query.is_empty() {
            return Err(AniError::InputEmptyQuery);
        }
        let cache_key = query.to_ascii_lowercase();
        if let Some(value) = cache_get(&self.inner.searches, &cache_key) {
            return Ok(value);
        }
        let url = format!("{}/api/search", self.inner.base);
        let response = self
            .inner
            .http
            .post(&url)
            .header(header::REFERER, format!("{}/", self.inner.base))
            .header("X-Requested-With", "XMLHttpRequest")
            .header(
                header::ACCEPT,
                "application/json, text/javascript, */*; q=0.01",
            )
            .form(&[("value", query)])
            .send()
            .await?;
        let payload = checked_json(response, "TioAnime").await?;
        let mut values = parse_search(&payload)?;
        if !values.is_empty() {
            let enrich_len = values.len().min(6);
            let counts = self.enrich_episode_counts(&values[..enrich_len]).await;
            for (idx, count) in counts.into_iter().enumerate() {
                if count > 0.0 {
                    values[idx].episodes = count;
                }
            }
        }
        cache_put(&self.inner.searches, cache_key, values.clone());
        Ok(values)
    }

    async fn enrich_episode_counts(&self, results: &[SearchResult]) -> Vec<f64> {
        use futures_util::future::join_all;
        let futures = results.iter().map(|result| {
            let client = self.clone();
            let id = result.id.clone();
            async move {
                match tokio::time::timeout(
                    Duration::from_secs(8),
                    client.episodes(&id, TranslationType::Sub),
                )
                .await
                {
                    Ok(Ok(episodes)) => episodes.len() as f64,
                    _ => 0.0,
                }
            }
        });
        match tokio::time::timeout(Duration::from_secs(15), join_all(futures)).await {
            Ok(counts) => counts,
            Err(_) => vec![0.0; results.len()],
        }
    }

    pub async fn episodes(&self, show_id: &str, _mode: TranslationType) -> Result<Vec<String>> {
        let id = decode_id(show_id)?;
        // NOTE: the anime page 404s with a trailing slash; the episode
        // page below requires it. Both verified live 2026-09-04.
        let url = format!("{}/anime/{}", self.inner.base, encode_path(&id.slug));
        let html = self.get_text(&url, &self.inner.base).await?;
        let mut episodes = parse_episodes(&html)?;
        sort_episodes(&mut episodes);
        if episodes.is_empty() {
            eprintln!("TioAnime has no episodes for {}", id.slug);
            return Err(AniError::UnavailableNoEpisodes);
        }
        Ok(episodes)
    }

    pub async fn streams(
        &self,
        show_id: &str,
        episode: &str,
        _mode: TranslationType,
    ) -> Result<Vec<StreamLink>> {
        let id = decode_id(show_id)?;
        let episode = normalize_episode(episode)?;
        let episode_url = format!(
            "{}/ver/{}-{}",
            self.inner.base,
            encode_path(&id.slug),
            encode_path(&episode)
        );
        let html = self.get_text(&episode_url, &self.inner.base).await?;
        let servers = parse_servers(&html);
        if servers.is_empty() {
            eprintln!("TioAnime episode {episode} exposed no video servers");
            return Err(AniError::UnavailableNoEpisodes);
        }
        let mut streams = Vec::new();
        let mut failures = Vec::new();
        // Servers without a direct resolver (Mega, Voe, …) are kept as
        // browser fallbacks so a removed YourUpload file still leaves the
        // user with something actionable instead of a dead end.
        let mut browser_fallbacks: Vec<(String, String)> = Vec::new();
        let mut yourupload_removed = false;
        for server in servers {
            // Resolution scope: only YourUpload resolves to direct media
            // today (see module docs for why Mega/Voe stay browser-only).
            if !server.label.eq_ignore_ascii_case("yourupload") {
                browser_fallbacks.push((server.label.clone(), server.embed.clone()));
                failures.push(format!("{}: resolver not implemented in PoC", server.label));
                continue;
            }
            match self.resolve_yourupload(&server, &episode_url).await {
                Ok(stream) => streams.push(stream),
                Err(error) => {
                    yourupload_removed |= is_novideo_error(&error);
                    failures.push(format!("{}: {error}", server.label));
                }
            }
        }
        let mut seen = std::collections::HashSet::new();
        streams.retain(|stream| seen.insert(stream.url.clone()));
        sort_streams(&mut streams);
        if streams.is_empty() {
            let mut detail = if failures.is_empty() {
                "no video servers resolved".to_string()
            } else {
                failures.join("; ")
            };
            detail.push_str(&browser_fallback_suffix(
                yourupload_removed,
                &browser_fallbacks,
            ));
            eprintln!("TioAnime source resolution failed: {detail}");
            return Err(AniError::UnavailableNoEpisodes);
        }
        Ok(streams)
    }

    async fn resolve_yourupload(&self, server: &Server, episode_url: &str) -> Result<StreamLink> {
        let embed = validate_remote_url(&server.embed)?;
        if !host_matches(embed.host_str().unwrap_or_default(), "yourupload.com") {
            return Err(AniError::Provider(format!(
                "unexpected YourUpload embed host {}",
                embed.host_str().unwrap_or_default()
            )));
        }
        let html = self.get_text(embed.as_str(), episode_url).await?;
        let media = parse_yourupload_media(&html).ok_or_else(|| {
            AniError::Provider("YourUpload page exposed no direct media URL".into())
        })?;
        // Deleted/expired YourUpload files serve `og:video=/embed/novideo.mp4`
        // and `jwplayer file: '/embed/novideo.mp4'` — not a real video.
        if media.contains("novideo") {
            return Err(AniError::Provider(
                "YourUpload video removed or unavailable (novideo.mp4) — try another episode (e.g., 2) or provider JKAnime".into(),
            ));
        }
        let parsed = validate_remote_url(&media).map_err(|_| {
            AniError::Provider(format!(
                "YourUpload returned an invalid media URL `{media}` — video may be removed"
            ))
        })?;
        Ok(StreamLink {
            url: parsed.to_string(),
            // The embed carries no quality metadata; the anime pages only
            // market generic "HD". Never guess a resolution.
            resolution: "Auto".into(),
            hls: false,
            provider: format!("TioAnime {}", server.label),
            downloadable: true,
            headers: RequestHeaders {
                // Proven necessary live 2026-09-04: the media hand-off
                // answers HTTP 500 without the embed page as Referer.
                referer: Some(embed.to_string()),
                origin: None,
                extra: Default::default(),
            },
            // Observed 2026-09-04: no subtitle tracks anywhere; the
            // "Subtitulado" catalog is hardcoded into the video.
            subtitles: Vec::new(),
        })
    }

    async fn get_text(&self, url: &str, referer: &str) -> Result<String> {
        let request = self
            .inner
            .http
            .get(url)
            .header(header::REFERER, referer)
            .header(header::ACCEPT_LANGUAGE, "es-ES,es;q=0.9,en;q=0.5")
            .header(header::ACCEPT, "text/html,application/xhtml+xml,*/*;q=0.8");
        checked_text(request.send().await?, "TioAnime", MAX_RESPONSE_BYTES).await
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Server {
    label: String,
    embed: String,
}

fn encode_id(value: &TioAnimeId) -> Result<String> {
    Ok(format!(
        "tioanime:{}",
        STANDARD.encode(serde_json::to_vec(value)?)
    ))
}

fn decode_id(value: &str) -> Result<TioAnimeId> {
    if let Some(payload) = value.strip_prefix("tioanime:") {
        if payload.is_empty() {
            return Err(AniError::Input("invalid TioAnime show ID".into()));
        }
        // Structured metadata first: unpadded base64 IDs also pass the slug
        // shape, and guessing slug first misroutes them to a 404 (same class
        // of bug as JKAnime One Punch Man 3 — live 2026-09-05). A bare slug
        // can never survive the base64+JSON round-trip into this struct.
        if let Ok(bytes) = STANDARD.decode(payload)
            && let Ok(decoded) = serde_json::from_slice::<TioAnimeId>(&bytes)
            && validate_slug(&decoded.slug).is_ok()
        {
            return Ok(decoded);
        }
        // Historical shorthand: a bare slug after the prefix.
        validate_slug(payload)?;
        return Ok(TioAnimeId {
            slug: payload.into(),
            title: None,
        });
    }
    validate_slug(value)?;
    Ok(TioAnimeId {
        slug: value.into(),
        title: None,
    })
}

#[derive(Clone, Debug, Default, Deserialize)]
struct SearchEntry {
    #[serde(default)]
    title: String,
    #[serde(default)]
    slug: String,
}

fn parse_search(payload: &Value) -> Result<Vec<SearchResult>> {
    let entries: Vec<SearchEntry> = match payload {
        Value::Array(entries) => entries
            .iter()
            .filter_map(|entry| serde_json::from_value(entry.clone()).ok())
            .collect(),
        _ => {
            return Err(AniError::Provider(
                "TioAnime search response was not a list".into(),
            ));
        }
    };
    let mut seen = std::collections::HashSet::new();
    let mut results = Vec::new();
    for entry in entries {
        if entry.slug.is_empty() || !seen.insert(entry.slug.clone()) {
            continue;
        }
        validate_slug(&entry.slug)?;
        let title = entry.title.trim();
        let title = if title.is_empty() {
            entry.slug.replace('-', " ")
        } else {
            clean_text(title)
        };
        results.push(SearchResult {
            id: encode_id(&TioAnimeId {
                slug: entry.slug,
                title: Some(title.clone()),
            })?,
            name: title,
            // The search entries carry no episode count.
            episodes: 0.0,
            provider: CatalogProvider::TioAnime,
        });
    }
    Ok(results)
}

fn parse_episodes(html: &str) -> Result<Vec<String>> {
    let array = Regex::new(r"var\s+episodes\s*=\s*(\[.*?\])")
        .expect("static regex")
        .captures(html)
        .map(|captures| captures[1].to_string())
        .ok_or_else(|| AniError::Provider("TioAnime page exposed no episode list".into()))?;
    let numbers: Vec<Value> = serde_json::from_str(&array)
        .map_err(|_| AniError::Provider("TioAnime episode list was not valid JSON".into()))?;
    let mut episodes = Vec::new();
    for number in numbers {
        let text = match &number {
            Value::Number(number) => number.to_string(),
            Value::String(number) => number.clone(),
            _ => continue,
        };
        if let Ok(normalized) = normalize_episode(&text) {
            episodes.push(normalized);
        }
    }
    Ok(episodes)
}

fn parse_servers(html: &str) -> Vec<Server> {
    let Some(array) = Regex::new(r"var\s+videos\s*=\s*(\[.*?\])\s*;")
        .expect("static regex")
        .captures(html)
        .map(|captures| captures[1].to_string())
    else {
        return Vec::new();
    };
    // The entries HTML-escape slashes (`https:\/\/`); unescape before JSON.
    let array = array.replace("\\/", "/");
    let Ok(entries) = serde_json::from_str::<Vec<Value>>(&array) else {
        return Vec::new();
    };
    entries
        .into_iter()
        .filter_map(|entry| {
            let label = entry.get(0)?.as_str()?.trim().to_string();
            let embed = entry.get(1)?.as_str()?.trim().to_string();
            if label.is_empty() || embed.is_empty() {
                return None;
            }
            validate_remote_url(&embed).ok()?;
            Some(Server { label, embed })
        })
        .collect()
}

fn parse_yourupload_media(html: &str) -> Option<String> {
    let og = Regex::new(r#"<meta\s+property="og:video"\s+content="([^"]+)""#)
        .expect("static regex")
        .captures(html)
        .map(|captures| captures[1].to_string());
    if og.is_some() {
        return og;
    }
    Regex::new(r"file:\s*'([^']+)'")
        .expect("static regex")
        .captures(html)
        .map(|captures| captures[1].to_string())
}

/// True when `error` is the removed-file signal from [`TioAnimeClient::resolve_yourupload`]
/// (deleted/expired YourUpload files serve `/embed/novideo.mp4`). Lets
/// [`TioAnimeClient::streams`] tell a removed file apart from a generic
/// resolution failure so the fallback hint can point at sibling servers or
/// another episode instead of suggesting a bare retry.
fn is_novideo_error(error: &AniError) -> bool {
    matches!(error, AniError::Provider(message) if message.contains("novideo"))
}

/// Appends an actionable suffix to the `streams` resolution error listing
/// sibling servers as browser fallbacks. Pure (no I/O) so it is unit-testable.
fn browser_fallback_suffix(
    yourupload_removed: bool,
    browser_fallbacks: &[(String, String)],
) -> String {
    if browser_fallbacks.is_empty() {
        return String::new();
    }
    let alternates = browser_fallbacks
        .iter()
        .map(|(label, embed)| format!("{label} {embed}"))
        .collect::<Vec<_>>()
        .join("; ");
    if yourupload_removed {
        format!(
            "; YourUpload file was removed upstream — try another episode (e.g., 2) or provider JKAnime, or open in a browser: {alternates}"
        )
    } else {
        format!("; not directly playable — open in a browser: {alternates}")
    }
}

fn normalize_episode(value: &str) -> Result<String> {
    let number = value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|number| number.is_finite() && *number >= 0.0)
        .ok_or_else(|| AniError::Input(format!("invalid episode number: {value}")))?;
    if number.fract() == 0.0 {
        Ok(format!("{number:.0}"))
    } else {
        Ok(number.to_string())
    }
}

fn validate_slug(value: &str) -> Result<()> {
    if Regex::new(r"(?i)^[a-z0-9][a-z0-9-]{0,199}$")
        .expect("static regex")
        .is_match(value)
    {
        Ok(())
    } else {
        Err(AniError::Input("invalid TioAnime show slug".into()))
    }
}

fn validate_remote_url(value: &str) -> Result<Url> {
    let url = Url::parse(value)?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(AniError::Provider("unsafe provider URL".into()));
    }
    if url
        .host_str()
        .is_some_and(|host| host.parse::<std::net::IpAddr>().is_ok())
    {
        return Err(AniError::Provider(
            "literal-IP provider URLs are not allowed".into(),
        ));
    }
    Ok(url)
}

fn host_matches(host: &str, domain: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == domain || host.ends_with(&format!(".{domain}"))
}

fn clean_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn encode_path(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

async fn checked_text(response: Response, provider: &str, max_bytes: usize) -> Result<String> {
    let status = response.status();
    if status == StatusCode::TOO_MANY_REQUESTS {
        let retry_after_seconds = response
            .headers()
            .get(header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse().ok())
            .unwrap_or(120);
        return Err(AniError::ProviderRateLimited {
            provider: provider.into(),
            retry_after_seconds,
        });
    }
    if !status.is_success() {
        return Err(AniError::Catalog {
            provider: provider.into(),
            message: format!("HTTP {status}"),
        });
    }
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(AniError::Provider(
            "provider response exceeded the safety limit".into(),
        ));
    }
    let bytes = response.bytes().await?;
    if bytes.len() > max_bytes {
        return Err(AniError::Provider(
            "provider response exceeded the safety limit".into(),
        ));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

async fn checked_json(response: Response, provider: &str) -> Result<Value> {
    let text = checked_text(response, provider, MAX_RESPONSE_BYTES).await?;
    serde_json::from_str(&text)
        .map_err(|_| AniError::Provider("provider returned invalid JSON".into()))
}

fn cache_get<T: Clone>(cache: &Mutex<HashMap<String, Cached<T>>>, key: &str) -> Option<T> {
    let mut cache = cache.lock().ok()?;
    cache.retain(|_, value| value.expires_at > Instant::now());
    cache.get(key).map(|value| value.value.clone())
}

fn cache_put<T>(cache: &Mutex<HashMap<String, Cached<T>>>, key: String, value: T) {
    if let Ok(mut cache) = cache.lock() {
        if cache.len() >= CACHE_LIMIT
            && let Some(key) = cache.keys().next().cloned()
        {
            cache.remove(&key);
        }
        cache.insert(
            key,
            Cached {
                expires_at: Instant::now() + CACHE_TTL,
                value,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_and_raw_slugs_are_supported() {
        let id = TioAnimeId {
            slug: "black-torch".into(),
            title: Some("Black Torch".into()),
        };
        let encoded = encode_id(&id).unwrap();
        assert!(encoded.starts_with("tioanime:"));
        assert_eq!(decode_id(&encoded).unwrap(), id);
        assert_eq!(decode_id("black-torch").unwrap().slug, "black-torch");
        assert_eq!(
            decode_id("tioanime:black-torch").unwrap().slug,
            "black-torch"
        );
    }

    #[test]
    fn invalid_ids_fail_before_network() {
        assert!(decode_id("").is_err());
        assert!(decode_id("tioanime:").is_err());
        assert!(decode_id("tioanime:not-base64!!!").is_err());
        assert!(decode_id("not a slug!").is_err());
    }

    #[test]
    fn unpadded_ids_decode_to_metadata_not_slugs() {
        // Same class of bug as JKAnime One Punch Man 3 (live 2026-09-05):
        // base64 without `=`/`+`/`/` must not be mistaken for a bare slug.
        let id = TioAnimeId {
            slug: "one-punch-man-3".into(),
            title: Some("One Punch Man 3".into()),
        };
        let encoded = encode_id(&id).unwrap();
        assert_eq!(decode_id(&encoded).unwrap(), id);
        // And the shorthand still works for real slugs.
        assert_eq!(
            decode_id("tioanime:black-torch").unwrap().slug,
            "black-torch"
        );
    }

    #[test]
    fn parses_search_entries() {
        let payload = serde_json::json!([
            {"id": "4452", "title": "Black Torch", "type": "0", "slug": "black-torch"},
            {"id": "1", "title": "", "type": "0", "slug": "otro-anime"},
            {"id": "2", "title": "Duplicate", "type": "0", "slug": "black-torch"},
        ]);
        let values = parse_search(&payload).unwrap();
        assert_eq!(values.len(), 2);
        assert_eq!(values[0].name, "Black Torch");
        assert_eq!(values[0].provider, CatalogProvider::TioAnime);
        assert_eq!(decode_id(&values[0].id).unwrap().slug, "black-torch");
        assert_eq!(values[1].name, "otro anime");
    }

    #[test]
    fn malformed_search_payload_is_rejected() {
        assert!(parse_search(&serde_json::json!({"ok": true})).is_err());
    }

    #[test]
    fn parses_embedded_episode_list() {
        let html = r#"<script>var anime_info = ["4452","black-torch","Black Torch","2026-09-05"];
            var episodes = [9,8,"7",7.5];</script>"#;
        let mut episodes = parse_episodes(html).unwrap();
        sort_episodes(&mut episodes);
        assert_eq!(episodes, vec!["7", "7.5", "8", "9"]);
        assert!(parse_episodes("<html></html>").is_err());
    }

    #[test]
    fn parses_video_servers_with_labels() {
        let html = r#"<script>var videos = [["Mega","https:\/\/mega.nz\/embed\/!abc!def",0,0],["YourUpload","https:\/\/www.yourupload.com\/embed\/xyz",0,0]];</script>"#;
        let servers = parse_servers(html);
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].label, "Mega");
        assert_eq!(servers[1].label, "YourUpload");
        assert!(parse_servers("<html></html>").is_empty());
    }

    #[test]
    fn extracts_yourupload_direct_media() {
        let html = r#"<meta property="og:video" content="https://vidcache.net:8161/a20260904ToEX572c3ay/video.mp4">"#;
        assert!(
            parse_yourupload_media(html)
                .unwrap()
                .ends_with("/video.mp4")
        );
        let jwplayer = "<script>var x = {file: 'https://cdn.example.invalid/v.mp4'};</script>";
        assert_eq!(
            parse_yourupload_media(jwplayer).as_deref(),
            Some("https://cdn.example.invalid/v.mp4")
        );
        assert!(parse_yourupload_media("<html></html>").is_none());
    }

    #[test]
    fn unsafe_media_urls_are_rejected() {
        assert!(validate_remote_url("http://media.example.invalid/x.mp4").is_err());
        assert!(validate_remote_url("https://192.0.2.1/x.mp4").is_err());
    }

    #[test]
    fn novideo_errors_are_detected_for_browser_fallback() {
        let removed = AniError::Provider(
            "YourUpload video removed or unavailable (novideo.mp4) — try another episode (e.g., 2) or provider JKAnime".into(),
        );
        assert!(is_novideo_error(&removed));
        assert!(!is_novideo_error(&AniError::Provider(
            "YourUpload page exposed no direct media URL".into()
        )));
        assert!(!is_novideo_error(&AniError::UnavailableNoEpisodes));
    }

    #[test]
    fn browser_fallback_suffix_points_at_sibling_servers() {
        let siblings = vec![
            (
                "Mega".to_string(),
                "https://mega.nz/embed/!abc!def".to_string(),
            ),
            (
                "Voe".to_string(),
                "https://voe.sx/e/zinf7arp3m40".to_string(),
            ),
        ];
        let removed = browser_fallback_suffix(true, &siblings);
        assert!(removed.contains("removed upstream"));
        assert!(removed.contains("https://mega.nz/embed/!abc!def"));
        assert!(removed.contains("https://voe.sx/e/zinf7arp3m40"));
        let generic = browser_fallback_suffix(false, &siblings);
        assert!(generic.contains("open in a browser"));
        assert!(generic.contains("https://voe.sx/e/zinf7arp3m40"));
        assert!(browser_fallback_suffix(true, &[]).is_empty());
        assert!(browser_fallback_suffix(false, &[]).is_empty());
    }

    #[tokio::test]
    async fn live_tioanime_smoke_test_is_opt_in() {
        if std::env::var("ANI_CLI_LIVE_TIOANIME").as_deref() != Ok("1") {
            return;
        }
        let client = TioAnimeClient::new().unwrap();
        let results = client
            .search("black torch", TranslationType::Sub)
            .await
            .unwrap();
        let show = results
            .iter()
            .find(|result| result.name.eq_ignore_ascii_case("Black Torch"))
            .unwrap();
        let episodes = client
            .episodes(&show.id, TranslationType::Sub)
            .await
            .unwrap();
        assert!(!episodes.is_empty());
        let latest = episodes.last().cloned().unwrap();
        let streams = client
            .streams(&show.id, &latest, TranslationType::Sub)
            .await
            .unwrap();
        assert!(!streams.is_empty());
        assert!(streams.iter().all(|stream| stream.downloadable));
    }
}
