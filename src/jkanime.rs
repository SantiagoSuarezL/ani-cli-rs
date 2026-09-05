//! Experimental JKAnime provider.
//!
//! Live behavior verified 2026-09-04 against `https://jkanime.net`
//! (Black Torch: search, anime page, episode 9, JKPlayer embed):
//! - search: `GET {base}/buscar/<query>`; results are
//!   `div[data-g="<base64 anime id>"] .anime__item a[href="/<slug>/"]`.
//! - anime page: `GET {base}/<slug>/`; carries `data-anime="<id>"` and the
//!   CSRF `<meta name="csrf-token">`.
//! - episodes: `POST {base}/ajax/episodes/<id>/<page>` with form `_token`,
//!   returning `{data: [{number, ...}], total: N}`.
//! - episode page: `GET {base}/<slug>/<episode>/`; embeds JKPlayer iframes in
//!   `video[N] = '<iframe ... src=".../jkplayer/...">'` plus one
//!   `a.servers.btn-show` label per `video[N]` entry (Desu, Magi, ...).
//! - player: `GET {base}/jkplayer/<path>?e=..&t=..`; the page embeds the
//!   playable HLS URL base64-encoded in `atob('...')` with signed `?st=&e=`
//!   query parameters, so stream URLs are temporary.
//!
//! Language observed on the live episode page: a single
//! `Japones Sub. Español` track selector (`lg_1`) and no external subtitle
//! tracks in the player data, so Spanish subtitles are treated as hardcoded
//! and `StreamLink.subtitles` stays empty. No regional (`es-419`/`es-ES`)
//! distinction was observed anywhere; nothing here guesses one.
//!
//! Only the JKPlayer embeds (`video[N]`) are resolved. The secondary
//! `servers` array (Mediafire/Mega/Streamwish/...) and the `jkplayer/c1`
//! download path are intentionally left unresolved until live evidence
//! requires them.
//!
//! This provider satisfies `--language es`: the catalog it serves is the
//! single "Japones Sub. Español" one described above.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use regex::Regex;
use reqwest::{Client, Response, StatusCode, header};
use scraper::{Html, Selector};
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
const DEFAULT_BASE: &str = "https://jkanime.net";
const DEFAULT_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36";
const CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const CACHE_LIMIT: usize = 100;
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_PLAYLIST_BYTES: usize = 4 * 1024 * 1024;
const MAX_EPISODE_PAGES: usize = 25;
/// Delay before the single retry of a failed catalog fetch (live 2026-09-05:
/// episode/embed pages intermittently answer HTTP 404 under burst load — a
/// fan-out search plus enrichment fires 12+ requests in seconds — then
/// succeed on immediate retry; see Regla 12.1).
const FETCH_RETRY_DELAY: Duration = Duration::from_millis(800);

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct JkAnimeId {
    slug: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    anime_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
}

#[derive(Clone, Debug)]
struct Cached<T> {
    expires_at: Instant,
    value: T,
}

#[derive(Clone, Debug)]
pub struct JkAnimeClientBuilder {
    base: String,
    user_agent: String,
    timeout: Duration,
}

impl Default for JkAnimeClientBuilder {
    fn default() -> Self {
        Self {
            base: DEFAULT_BASE.into(),
            user_agent: DEFAULT_AGENT.into(),
            timeout: Duration::from_secs(15),
        }
    }
}

impl JkAnimeClientBuilder {
    pub fn base_url(mut self, value: impl Into<String>) -> Self {
        self.base = value.into();
        self
    }

    pub fn timeout(mut self, value: Duration) -> Self {
        self.timeout = value;
        self
    }

    pub fn build(self) -> Result<JkAnimeClient> {
        let http = Client::builder()
            .timeout(self.timeout)
            .user_agent(&self.user_agent)
            .cookie_store(true)
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()?;
        Ok(JkAnimeClient {
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
pub struct JkAnimeClient {
    inner: Arc<Inner>,
}

impl JkAnimeClient {
    pub fn builder() -> JkAnimeClientBuilder {
        JkAnimeClientBuilder::default()
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
        // JKAnime serves a single "Japones Sub. Español" catalog: there is no
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
        let mut url = Url::parse(&self.inner.base)?;
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| AniError::Provider("invalid JKAnime base URL".into()))?;
            segments.push("buscar").push(query);
        }
        let html = self.get_text(url.as_str(), &self.inner.base, false).await?;
        let mut values = parse_search(&self.inner.base, &html)?;
        // Enrich search results with real episode counts when cheap:
        // `parse_search` has no count (cards lack it), but the anime page +
        // ajax endpoint do. Best-effort, never fails search; cap at 6 to
        // avoid spamming the provider and keep the picker snappy (first 6
        // get counts, rest stay 0 and are hidden in the picker per main.rs).
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
                match tokio::time::timeout(Duration::from_secs(6), client.episode_count(&id)).await
                {
                    Ok(Ok(count)) => count as f64,
                    _ => 0.0,
                }
            }
        });
        match tokio::time::timeout(Duration::from_secs(10), join_all(futures)).await {
            Ok(counts) => counts,
            Err(_) => vec![0.0; results.len()],
        }
    }

    async fn episode_count(&self, show_id: &str) -> Result<usize> {
        let id = decode_id(show_id)?;
        let anime_id = match id.anime_id.clone() {
            Some(anime_id) => anime_id,
            None => self.discover_anime_id(&id.slug).await?.0,
        };
        // Single page is enough for the total; avoids fetching 25 pages.
        let first = self.episode_page(&id.slug, &anime_id, 1).await?;
        if first.total > 0 {
            return Ok(first.total as usize);
        }
        if !first.data.is_empty() {
            return Ok(first.data.len());
        }
        // empty ajax: try fallback parser (last episode link)
        let fallback = self.episode_fallback(&id.slug).await?;
        Ok(fallback.len())
    }

    pub async fn episodes(&self, show_id: &str, _mode: TranslationType) -> Result<Vec<String>> {
        let id = decode_id(show_id)?;
        let anime_id = match id.anime_id.clone() {
            Some(anime_id) => anime_id,
            None => self.discover_anime_id(&id.slug).await?.0,
        };
        let mut episodes = Vec::new();
        for page in 1..=MAX_EPISODE_PAGES {
            let entries = self.episode_page(&id.slug, &anime_id, page).await?;
            if entries.data.is_empty() {
                break;
            }
            let total = entries.total;
            episodes.extend(entries.data.into_iter().filter_map(episode_number));
            if total == 0 || episodes.len() as u32 >= total {
                break;
            }
        }
        if episodes.is_empty() {
            // The anime page is server-rendered without episodes (they load
            // via the AJAX endpoint above); fall back to the "Último
            // episodio" link when the endpoint yields nothing.
            episodes = self.episode_fallback(&id.slug).await?;
        }
        sort_episodes(&mut episodes);
        if episodes.is_empty() {
            eprintln!("JKAnime has no episodes for {}", id.slug);
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
            "{}/{}/{}/",
            self.inner.base,
            encode_path(&id.slug),
            encode_path(&episode)
        );
        let html = self.get_text(&episode_url, &self.inner.base, false).await?;
        let embeds = parse_player_embeds(&self.inner.base, &html);
        if embeds.is_empty() {
            eprintln!("JKAnime episode {episode} exposed no JKPlayer embeds");
            return Err(AniError::UnavailableNoEpisodes);
        }
        let mut streams = Vec::new();
        let mut failures = Vec::new();
        for embed in embeds {
            match self.resolve_player(&embed, &episode_url).await {
                Ok(mut resolved) => streams.append(&mut resolved),
                Err(error) => failures.push(format!("{}: {error}", embed.label)),
            }
        }
        let mut seen = HashSet::new();
        streams.retain(|stream| seen.insert(stream.url.clone()));
        sort_streams(&mut streams);
        if streams.is_empty() {
            let detail = if failures.is_empty() {
                "no JKPlayer embeds resolved".into()
            } else {
                failures.join("; ")
            };
            eprintln!("JKAnime source resolution failed: {detail}");
            return Err(AniError::UnavailableNoEpisodes);
        }
        Ok(streams)
    }

    async fn discover_anime_id(&self, slug: &str) -> Result<(String, String)> {
        let url = format!("{}/{}/", self.inner.base, encode_path(slug));
        let html = self.get_text(&url, &self.inner.base, false).await?;
        let anime_id = parse_anime_id(&html).ok_or_else(|| {
            AniError::Provider("JKAnime anime page exposed no numeric show id".into())
        })?;
        let token = parse_csrf_token(&html)
            .ok_or_else(|| AniError::Provider("JKAnime anime page exposed no CSRF token".into()))?;
        Ok((anime_id, token))
    }

    async fn episode_page(&self, slug: &str, anime_id: &str, page: usize) -> Result<EpisodeList> {
        let (_, token) = self.discover_anime_id(slug).await?;
        let url = format!(
            "{}/ajax/episodes/{}/{}",
            self.inner.base,
            encode_path(anime_id),
            page
        );
        let text = self.post_text(&url, &token).await?;
        serde_json::from_str(&text)
            .map_err(|_| AniError::Provider("JKAnime episode list was not valid JSON".into()))
    }

    async fn episode_fallback(&self, slug: &str) -> Result<Vec<String>> {
        let url = format!("{}/{}/", self.inner.base, encode_path(slug));
        let html = self.get_text(&url, &self.inner.base, false).await?;
        let latest = parse_latest_episode(slug, &html).ok_or_else(|| {
            AniError::Provider("JKAnime anime page exposed no episode links".into())
        })?;
        let count: usize = latest.parse().map_err(|_| {
            AniError::Provider("JKAnime latest episode number was not numeric".into())
        })?;
        if count == 0 {
            return Ok(Vec::new());
        }
        Ok((1..=count).map(|episode| episode.to_string()).collect())
    }

    async fn resolve_player(
        &self,
        embed: &PlayerEmbed,
        episode_url: &str,
    ) -> Result<Vec<StreamLink>> {
        let embed_url = validate_remote_url(&embed.url)?;
        if !host_matches(embed_url.host_str().unwrap_or_default(), "jkanime.net") {
            return Err(AniError::Provider(format!(
                "unsupported JKAnime embed host {}",
                embed_url.host_str().unwrap_or_default()
            )));
        }
        let html = self
            .get_text(embed_url.as_str(), episode_url, false)
            .await?;
        let encoded = parse_embedded_media(&html).ok_or_else(|| {
            AniError::Provider("JKPlayer page exposed no playable media URL".into())
        })?;
        let bytes = STANDARD.decode(encoded).map_err(|_| {
            AniError::Provider("JKPlayer media payload was not valid base64".into())
        })?;
        let media = String::from_utf8(bytes)
            .map_err(|_| AniError::Provider("JKPlayer media payload was not UTF-8".into()))?;
        let parsed = validate_remote_url(media.trim())?;
        let hls = parsed.path().to_ascii_lowercase().contains(".m3u8")
            || parsed.query().is_some_and(|query| query.contains(".m3u8"));
        // Live evidence 2026-09-04: the media host plays with no extra
        // headers (bare mpv plays 1080p fine), while sending the JKPlayer
        // Referer/Origin/UA stalls playback. Per the "only headers proven
        // necessary" rule, send none here. The episode-page Referer stays on
        // the JKPlayer page fetch above, where it belongs.
        let headers = RequestHeaders::default();
        if hls {
            return self
                .expand_hls(parsed.as_str(), &embed.label, &headers)
                .await;
        }
        Ok(vec![StreamLink {
            url: parsed.to_string(),
            resolution: "Auto".into(),
            hls: false,
            provider: embed.label.clone(),
            downloadable: false,
            headers,
            subtitles: Vec::new(),
        }])
    }

    async fn expand_hls(
        &self,
        url: &str,
        provider: &str,
        headers: &RequestHeaders,
    ) -> Result<Vec<StreamLink>> {
        let mut request = self.inner.http.get(url);
        request = apply_headers(request, headers);
        let response = request.send().await?;
        if !response.status().is_success() {
            return Ok(vec![stream_link(url, provider, headers)]);
        }
        let bytes = response.bytes().await?;
        if bytes.len() > MAX_PLAYLIST_BYTES {
            return Err(AniError::Provider(
                "JKAnime playlist exceeds the 4 MiB limit".into(),
            ));
        }
        let text = String::from_utf8_lossy(&bytes);
        if !text.contains("#EXTM3U") || !text.contains("#EXT-X-STREAM-INF") {
            return Ok(vec![stream_link(url, provider, headers)]);
        }
        let base = Url::parse(url)?;
        let resolution = Regex::new(r"(?i)RESOLUTION=\d+x(\d+)").expect("static regex");
        let lines = text.lines().collect::<Vec<_>>();
        let mut variants = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            if !line.trim_start().starts_with("#EXT-X-STREAM-INF") {
                continue;
            }
            let label = resolution
                .captures(line)
                .map(|captures| format!("{}p", &captures[1]))
                .unwrap_or_else(|| "Auto".into());
            if let Some(path) = lines[index + 1..]
                .iter()
                .map(|line| line.trim())
                .find(|line| !line.is_empty() && !line.starts_with('#'))
            {
                let url = base.join(path)?;
                variants.push(StreamLink {
                    url: url.to_string(),
                    resolution: label,
                    hls: true,
                    provider: provider.into(),
                    downloadable: false,
                    headers: headers.clone(),
                    // Observed 2026-09-04: no external subtitle tracks in the
                    // JKPlayer data (single "Japones Sub. Español" hardcoded).
                    subtitles: Vec::new(),
                });
            }
        }
        if variants.is_empty() {
            variants.push(stream_link(url, provider, headers));
        }
        Ok(variants)
    }

    async fn get_text(&self, url: &str, referer: &str, ajax: bool) -> Result<String> {
        match self.get_text_once(url, referer, ajax).await {
            // One retry on catalog HTTP failures (transient WAF 404s under
            // burst load). Parse, network and rate-limit errors keep their
            // first outcome, so a genuinely missing page still fails —
            // roughly one retry delay later, never in a loop.
            Err(AniError::Catalog { .. }) => {
                tokio::time::sleep(FETCH_RETRY_DELAY).await;
                self.get_text_once(url, referer, ajax).await
            }
            outcome => outcome,
        }
    }

    async fn get_text_once(&self, url: &str, referer: &str, ajax: bool) -> Result<String> {
        let mut request = self
            .inner
            .http
            .get(url)
            .header(header::REFERER, referer)
            .header(header::ACCEPT_LANGUAGE, "es-ES,es;q=0.9,en;q=0.5");
        if ajax {
            request = request.header("X-Requested-With", "XMLHttpRequest").header(
                header::ACCEPT,
                "application/json, text/javascript, */*; q=0.01",
            );
        } else {
            request = request.header(header::ACCEPT, "text/html,application/xhtml+xml,*/*;q=0.8");
        }
        checked_text(request.send().await?, "JKAnime", MAX_RESPONSE_BYTES).await
    }

    async fn post_text(&self, url: &str, token: &str) -> Result<String> {
        match self.post_text_once(url, token).await {
            // Same single-retry policy as `get_text`: the episode-list AJAX
            // endpoint flaps the same way under burst load.
            Err(AniError::Catalog { .. }) => {
                tokio::time::sleep(FETCH_RETRY_DELAY).await;
                self.post_text_once(url, token).await
            }
            outcome => outcome,
        }
    }

    async fn post_text_once(&self, url: &str, token: &str) -> Result<String> {
        let request = self
            .inner
            .http
            .post(url)
            .header(header::REFERER, &self.inner.base)
            .header("X-Requested-With", "XMLHttpRequest")
            .header(
                header::ACCEPT,
                "application/json, text/javascript, */*; q=0.01",
            )
            .form(&[("_token", token)]);
        checked_text(request.send().await?, "JKAnime", MAX_RESPONSE_BYTES).await
    }
}

fn stream_link(url: &str, provider: &str, headers: &RequestHeaders) -> StreamLink {
    StreamLink {
        url: url.into(),
        resolution: "Auto".into(),
        hls: true,
        provider: provider.into(),
        downloadable: false,
        headers: headers.clone(),
        subtitles: Vec::new(),
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
struct EpisodeList {
    #[serde(default)]
    data: Vec<EpisodeEntry>,
    #[serde(default)]
    total: u32,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct EpisodeEntry {
    #[serde(default)]
    number: Value,
}

fn episode_number(entry: EpisodeEntry) -> Option<String> {
    match entry.number {
        Value::Number(number) => {
            if let Some(value) = number.as_u64() {
                Some(value.to_string())
            } else if let Some(value) = number.as_f64() {
                normalize_episode(&value.to_string()).ok()
            } else {
                None
            }
        }
        Value::String(number) => normalize_episode(&number).ok(),
        _ => None,
    }
}

struct PlayerEmbed {
    label: String,
    url: String,
}

fn encode_id(value: &JkAnimeId) -> Result<String> {
    Ok(format!(
        "jkanime:{}",
        STANDARD.encode(serde_json::to_vec(value)?)
    ))
}

fn decode_id(value: &str) -> Result<JkAnimeId> {
    if let Some(payload) = value.strip_prefix("jkanime:") {
        if payload.is_empty() {
            return Err(AniError::Input("invalid JKAnime show ID".into()));
        }
        // Structured metadata first: unpadded base64 IDs (no `=`/`+`/`/`,
        // e.g. One Punch Man 3 — live 2026-09-05) also pass the slug shape
        // below, so guessing slug first misroutes them to a 404. A bare slug
        // can never survive the base64+JSON round-trip into this struct.
        if let Ok(bytes) = STANDARD.decode(payload)
            && let Ok(decoded) = serde_json::from_slice::<JkAnimeId>(&bytes)
            && validate_slug(&decoded.slug).is_ok()
        {
            return Ok(decoded);
        }
        // Historical shorthand: a bare slug after the prefix.
        validate_slug(payload)?;
        return Ok(JkAnimeId {
            slug: payload.into(),
            anime_id: None,
            title: None,
        });
    }
    validate_slug(value)?;
    Ok(JkAnimeId {
        slug: value.into(),
        anime_id: None,
        title: None,
    })
}

fn parse_search(base: &str, html: &str) -> Result<Vec<SearchResult>> {
    let document = Html::parse_document(html);
    let cards = Selector::parse("div[data-g]")
        .map_err(|_| AniError::Provider("invalid JKAnime search selector".into()))?;
    let links = Selector::parse("a[href]")
        .map_err(|_| AniError::Provider("invalid JKAnime search link selector".into()))?;
    let titles = Selector::parse("h5")
        .map_err(|_| AniError::Provider("invalid JKAnime search title selector".into()))?;
    let base = Url::parse(base)?;
    let expected_host = base.host_str().unwrap_or_default();
    let mut seen = HashMap::new();
    let mut results = Vec::new();
    for card in document.select(&cards) {
        let anime_id = card
            .value()
            .attr("data-g")
            .and_then(|value| STANDARD.decode(value).ok())
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .filter(|value| value.bytes().all(|byte| byte.is_ascii_digit()) && !value.is_empty());
        let mut slug: Option<String> = None;
        for element in card.select(&links) {
            let Some(href) = element.value().attr("href") else {
                continue;
            };
            let Ok(url) = base.join(href) else {
                continue;
            };
            if url.scheme() != "https" || url.host_str() != Some(expected_host) {
                continue;
            }
            let path = url.path().trim_matches('/');
            if path.is_empty() || path.contains('/') || validate_slug(path).is_err() {
                continue;
            }
            slug = Some(path.into());
            break;
        }
        let Some(slug) = slug else { continue };
        if seen.contains_key(&slug) {
            continue;
        }
        let title = card
            .select(&titles)
            .next()
            .map(|value| clean_text(&value.text().collect::<String>()))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| slug.replace('-', " "));
        seen.insert(slug.clone(), ());
        results.push(SearchResult {
            id: encode_id(&JkAnimeId {
                slug,
                anime_id,
                title: Some(title.clone()),
            })?,
            name: title,
            // The search cards carry no episode count.
            episodes: 0.0,
            provider: CatalogProvider::JkAnime,
        });
    }
    Ok(results)
}

fn parse_anime_id(html: &str) -> Option<String> {
    Regex::new(r#"data-anime="(\d+)""#)
        .expect("static regex")
        .captures(html)
        .map(|captures| captures[1].into())
}

fn parse_csrf_token(html: &str) -> Option<String> {
    Regex::new(r#"<meta\s+name="csrf-token"\s+content="([^"]+)""#)
        .expect("static regex")
        .captures(html)
        .map(|captures| captures[1].into())
}

fn parse_latest_episode(slug: &str, html: &str) -> Option<String> {
    let pattern = format!(r#"href="https?://[^"/]+/{}/(\d+)/?""#, regex::escape(slug));
    let matches: Vec<String> = Regex::new(&pattern)
        .expect("static regex")
        .captures_iter(html)
        .map(|captures| captures[1].into())
        .collect();
    matches
        .into_iter()
        .max_by_key(|value| value.parse::<u64>().unwrap_or(0))
}

fn parse_player_embeds(base: &str, html: &str) -> Vec<PlayerEmbed> {
    let labels: Vec<String> = Html::parse_document(html)
        .select(&Selector::parse("a.servers.btn-show").expect("static selector"))
        .map(|element| clean_text(&element.text().collect::<String>()))
        .collect();
    let base = Url::parse(base).expect("verified base URL");
    Regex::new(r#"video\[\d+\]\s*=\s*'<iframe[^>]+src="([^"]+)""#)
        .expect("static regex")
        .captures_iter(html)
        .enumerate()
        .filter_map(|(index, captures)| {
            let src = captures[1].to_string();
            let url = base.join(&src).ok()?.to_string();
            let label = labels
                .get(index)
                .filter(|label| !label.is_empty())
                .cloned()
                .unwrap_or_else(|| format!("Player {}", index + 1));
            Some(PlayerEmbed {
                label: format!("JKAnime {label}"),
                url,
            })
        })
        .collect()
}

fn parse_embedded_media(html: &str) -> Option<String> {
    Regex::new(r#"atob\('([A-Za-z0-9+/=]{32,})'\)"#)
        .expect("static regex")
        .captures(html)
        .map(|captures| captures[1].into())
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
        Err(AniError::Input("invalid JKAnime show slug".into()))
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

fn apply_headers(
    mut request: reqwest::RequestBuilder,
    headers: &RequestHeaders,
) -> reqwest::RequestBuilder {
    if let Some(referer) = &headers.referer {
        request = request.header(header::REFERER, referer);
    }
    if let Some(origin) = &headers.origin {
        request = request.header(header::ORIGIN, origin);
    }
    for (name, value) in &headers.extra {
        request = request.header(name, value);
    }
    request
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
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    #[test]
    fn ids_round_trip_and_raw_slugs_are_supported() {
        let id = JkAnimeId {
            slug: "black-torch".into(),
            anime_id: Some("4772".into()),
            title: Some("Black Torch".into()),
        };
        let encoded = encode_id(&id).unwrap();
        assert!(encoded.starts_with("jkanime:"));
        assert_eq!(decode_id(&encoded).unwrap(), id);
        assert_eq!(decode_id("black-torch").unwrap().slug, "black-torch");
        assert_eq!(
            decode_id("jkanime:black-torch").unwrap().slug,
            "black-torch"
        );
    }

    #[test]
    fn invalid_ids_fail_before_network() {
        assert!(decode_id("").is_err());
        assert!(decode_id("jkanime:").is_err());
        assert!(decode_id("jkanime:not-base64!!!").is_err());
        assert!(decode_id("not a slug!").is_err());
    }

    #[test]
    fn unpadded_ids_decode_to_metadata_not_slugs() {
        // Live 2026-09-05 (One Punch Man 3): base64 without `=`/`+`/`/`
        // also matches the slug shape, and the old slug-first guess routed
        // it to `/{base64}/2/` → HTTP 404.
        let id = JkAnimeId {
            slug: "one-punch-man-3".into(),
            anime_id: Some("4353".into()),
            title: Some("One Punch Man 3".into()),
        };
        let encoded = encode_id(&id).unwrap();
        assert!(
            !encoded
                .strip_prefix("jkanime:")
                .unwrap()
                .contains(['=', '+', '/']),
            "test needs an unpadded ID to cover the regression"
        );
        assert_eq!(decode_id(&encoded).unwrap(), id);
    }

    #[test]
    fn parses_search_results_with_embedded_anime_ids() {
        let html = r#"
            <div class="row page_directorio">
              <div class="col-lg-2" data-g="NDc3Mg==">
                <div class="anime__item">
                  <a href="https://jkanime.net/black-torch/"></a>
                  <div class="anime__item__text"><h5><a href="https://jkanime.net/black-torch/">Black Torch</a></h5></div>
                </div>
              </div>
              <div class="col-lg-2" data-g="bm90LWEtaWQ=">
                <div class="anime__item">
                  <a href="https://jkanime.net/otro-anime/"></a>
                  <div class="anime__item__text"><h5><a href="https://jkanime.net/otro-anime/">Otro Anime</a></h5></div>
                </div>
              </div>
            </div>
        "#;
        let values = parse_search("https://jkanime.net", html).unwrap();
        assert_eq!(values.len(), 2);
        assert_eq!(values[0].name, "Black Torch");
        assert_eq!(values[0].provider, CatalogProvider::JkAnime);
        let decoded = decode_id(&values[0].id).unwrap();
        assert_eq!(decoded.slug, "black-torch");
        assert_eq!(decoded.anime_id.as_deref(), Some("4772"));
    }

    #[test]
    fn malformed_search_markup_yields_no_results() {
        let values = parse_search(
            "https://jkanime.net",
            "<html><body>no results here</body></html>",
        )
        .unwrap();
        assert!(values.is_empty());
    }

    #[test]
    fn parses_anime_id_and_csrf_token() {
        let html = r#"<meta name="csrf-token" content="token123"><div id="guardar-anime" data-anime="4772">"#;
        assert_eq!(parse_anime_id(html).as_deref(), Some("4772"));
        assert_eq!(parse_csrf_token(html).as_deref(), Some("token123"));
        assert!(parse_anime_id("<html></html>").is_none());
    }

    #[test]
    fn parses_episode_entries_with_mixed_number_types() {
        let list: EpisodeList = serde_json::from_str(
            r#"{"data": [{"number": 9}, {"number": "8"}, {"number": "7.5"}], "total": 3}"#,
        )
        .unwrap();
        let numbers = list
            .data
            .into_iter()
            .filter_map(episode_number)
            .collect::<Vec<_>>();
        assert_eq!(numbers, vec!["9", "8", "7.5"]);
    }

    #[test]
    fn extracts_jkplayer_embeds_with_server_labels() {
        let html = r##"
            <a id="btn-show-0" data-id="0" class="servers btn-show lg_1 mode_normal" href="#option0">Desu</a>
            <a id="btn-show-1" data-id="1" class="servers btn-show lg_1 mode_normal" href="#option1">Magi</a>
            <script>video[0] = '<iframe class="player_conte" src="https://jkanime.net/jkplayer/um?e=abc&t=def">';</script>
            <script>video[1] = '<iframe class="player_conte" src="/jkplayer/umv?e=ghi">';</script>
        "##;
        let embeds = parse_player_embeds("https://jkanime.net", html);
        assert_eq!(embeds.len(), 2);
        assert_eq!(embeds[0].label, "JKAnime Desu");
        assert_eq!(embeds[0].url, "https://jkanime.net/jkplayer/um?e=abc&t=def");
        assert_eq!(embeds[1].label, "JKAnime Magi");
        assert_eq!(embeds[1].url, "https://jkanime.net/jkplayer/umv?e=ghi");
    }

    #[test]
    fn decodes_embedded_hls_payload() {
        // Shape observed live 2026-09-04 on the JKPlayer page: the playable
        // HLS URL is base64-encoded inside atob('...').
        let payload =
            STANDARD.encode("https://media.example.invalid/episode-9/master.m3u8?st=x&e=1");
        let html = format!("<script>url: atob('{payload}'),</script>");
        let encoded = parse_embedded_media(&html).unwrap();
        let bytes = STANDARD.decode(encoded).unwrap();
        let url = validate_remote_url(String::from_utf8(bytes).unwrap().trim()).unwrap();
        assert!(url.path().contains(".m3u8"));
    }

    #[test]
    fn unsafe_media_urls_are_rejected() {
        assert!(validate_remote_url("http://media.example.invalid/x.m3u8").is_err());
        assert!(validate_remote_url("https://192.0.2.1/x.m3u8").is_err());
        assert!(parse_embedded_media("<html>no player here</html>").is_none());
    }

    #[tokio::test]
    async fn transient_catalog_404_is_retried_once() {
        // Live 2026-09-05: episode/embed pages flap HTTP 404 under burst
        // load, then succeed. The 404 mount is consumed after one match
        // (`up_to_n_times(1)` — `expect` alone only verifies, it does not
        // stop matching), so the retry observably hits the 200 below; the
        // received count assertion fails loudly if that ever changes.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/flaky"))
            .respond_with(ResponseTemplate::new(404))
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/flaky"))
            .respond_with(ResponseTemplate::new(200).set_body_string("recovered"))
            .expect(1)
            .mount(&server)
            .await;
        let client = JkAnimeClient::builder()
            .base_url(server.uri())
            .build()
            .unwrap();
        let body = client
            .get_text(&format!("{}/flaky", server.uri()), &server.uri(), false)
            .await
            .unwrap();
        assert_eq!(body, "recovered");
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn persistent_catalog_404_still_fails_after_one_retry() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/gone"))
            .respond_with(ResponseTemplate::new(404))
            .expect(2)
            .mount(&server)
            .await;
        let client = JkAnimeClient::builder()
            .base_url(server.uri())
            .build()
            .unwrap();
        let error = client
            .get_text(&format!("{}/gone", server.uri()), &server.uri(), false)
            .await
            .unwrap_err();
        assert!(matches!(error, AniError::Catalog { .. }));
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn live_jkanime_smoke_test_is_opt_in() {
        if std::env::var("ANI_CLI_LIVE_JKANIME").as_deref() != Ok("1") {
            return;
        }
        let client = JkAnimeClient::new().unwrap();
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
        assert!(streams.iter().any(|stream| stream.hls));
    }
}
