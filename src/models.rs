use std::{cmp::Ordering, collections::BTreeMap, fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::{AniError, Result};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CatalogProvider {
    #[default]
    Anikoto,
    Anikoto2,
    JkAnime,
    TioAnime,
}

impl fmt::Display for CatalogProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Anikoto => "anikoto",
            Self::Anikoto2 => "anikoto2",
            Self::JkAnime => "jkanime",
            Self::TioAnime => "tioanime",
        })
    }
}

impl CatalogProvider {
    /// Providers whose catalog is live-verified Spanish content.
    /// JKAnime and TioAnime qualify (single "sub español" catalogs with
    /// hardcoded subtitles, verified live 2026-09-04). Flip a provider to
    /// `true` only with fresh live evidence, never from historical
    /// repositories.
    pub fn spanish_capable(self) -> bool {
        matches!(self, Self::JkAnime | Self::TioAnime)
    }

    /// Spanish providers in reliability order (ARCH §6 #5 tie-breaker).
    /// JKAnime first: it is the historic `--language es` auto-route target,
    /// so merged results keep backward-compatible ordering (`--select-nth`
    /// keeps picking the same entry when JKAnime is non-empty). TioAnime
    /// appends after. Reorder only with fresh live reliability evidence.
    pub fn spanish_providers() -> [Self; 2] {
        [Self::JkAnime, Self::TioAnime]
    }
}

impl FromStr for CatalogProvider {
    type Err = AniError;

    fn from_str(value: &str) -> Result<Self> {
        match value.to_ascii_lowercase().as_str() {
            "anikoto" | "anikoto1" | "anikoto-api" => Ok(Self::Anikoto),
            "anikoto2" | "anikoto-cz" | "anikoto.cz" => Ok(Self::Anikoto2),
            "jkanime" | "jk" | "jk-anime" => Ok(Self::JkAnime),
            "tioanime" | "tio" => Ok(Self::TioAnime),
            _ => Err(AniError::Input(format!(
                "provider must be anikoto, anikoto2, jkanime or tioanime, got {value}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TranslationType {
    #[default]
    Sub,
    Dub,
}

impl fmt::Display for TranslationType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Sub => "sub",
            Self::Dub => "dub",
        })
    }
}

impl FromStr for TranslationType {
    type Err = AniError;
    fn from_str(value: &str) -> Result<Self> {
        match value.to_ascii_lowercase().as_str() {
            "sub" => Ok(Self::Sub),
            "dub" => Ok(Self::Dub),
            _ => Err(AniError::Input(format!(
                "translation type must be sub or dub, got {value}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LanguagePreference {
    #[default]
    Default,
    Spanish,
}

impl fmt::Display for LanguagePreference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Default => "default",
            Self::Spanish => "es",
        })
    }
}

impl FromStr for LanguagePreference {
    type Err = AniError;
    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "es" | "espanol" | "español" => Ok(Self::Spanish),
            "es-419" | "es419" | "es-es" | "es_es" => Err(AniError::Input(
                "regional Spanish variants (es-419/es-ES) are not supported yet; only --language es in this experimental phase".into(),
            )),
            _ => Err(AniError::Input(format!(
                "language must be es, got {value}"
            ))),
        }
    }
}

/// Chooses the catalog for a search: an explicit `--provider` always wins;
/// without one, `--language es` auto-routes to the only verified Spanish
/// source instead of the default catalog.
pub fn effective_search_provider(
    selected: Option<CatalogProvider>,
    language: LanguagePreference,
) -> CatalogProvider {
    match (selected, language) {
        (Some(provider), _) => provider,
        (None, LanguagePreference::Spanish) => CatalogProvider::JkAnime,
        (None, LanguagePreference::Default) => CatalogProvider::default(),
    }
}

/// Whether a `--language es` search without an explicit `--provider` should
/// fan out to every Spanish-capable catalog instead of a single auto-route
/// target (ARCH §7 "Other provider" layer + ARCH §10 isolation). An explicit
/// `--provider` always wins and never fans out.
pub fn should_fanout_spanish(
    selected: Option<CatalogProvider>,
    language: LanguagePreference,
) -> bool {
    selected.is_none() && language == LanguagePreference::Spanish
}

/// Whether a failed/empty Spanish search should offer the explicit English
/// fallback (TECH §7) instead of propagating the error: provider-side
/// failures (network, malformed data, rate limits, nothing found) qualify;
/// user input errors and local bugs never do.
pub fn is_fallback_trigger<T>(result: &Result<T>) -> bool {
    match result {
        Ok(_) => true, // callers only pass through here when results are empty
        Err(error) => is_fallback_error(error),
    }
}

/// Provider-side failure predicate behind [`is_fallback_trigger`], exposed so
/// multi-provider merges can classify an owned error without cloning it.
pub fn is_fallback_error(error: &AniError) -> bool {
    matches!(
        error,
        AniError::Network(_)
            | AniError::Provider(_)
            | AniError::Catalog { .. }
            | AniError::ProviderRateLimited { .. }
            | AniError::Unavailable(_)
            | AniError::UnavailableNoResults
            | AniError::UnavailableNoEpisodes
            | AniError::UnavailableNoStreams,
    )
}

/// Merges the two Spanish catalog searches in [`CatalogProvider::spanish_providers`]
/// order (JKAnime first for backward-compatible `--select-nth`).
/// Provider-side failures ([`is_fallback_error`]) are isolated per ARCH §10:
/// a failing catalog contributes nothing while the other still serves. A
/// non-fallback error (empty query, local bug) fails fast and never merges.
/// When every catalog fails with a fallback error, the last one is returned
/// so callers still enter the explicit English-fallback path.
pub fn merge_spanish_search(
    jk: Result<Vec<SearchResult>>,
    tio: Result<Vec<SearchResult>>,
) -> Result<Vec<SearchResult>> {
    let mut merged = Vec::new();
    let mut trigger_error: Option<AniError> = None;
    for result in [jk, tio] {
        match result {
            Ok(values) => merged.extend(values),
            Err(error) if is_fallback_error(&error) => trigger_error = Some(error),
            Err(error) => return Err(error),
        }
    }
    if !merged.is_empty() {
        return Ok(merged);
    }
    if let Some(error) = trigger_error {
        return Err(error);
    }
    Ok(merged)
}

/// Minimal experimental gate: only providers with live-verified Spanish
/// content satisfy `--language es`. JKAnime qualifies: its live episode pages
/// expose a single "Japones Sub. Español" catalog with hardcoded subtitles
/// (verified 2026-09-04, see `jkanime.rs`). Every other provider fails
/// explicitly instead of silently serving its default catalog. Regional
/// variants and cross-provider fallback arrive in a later phase.
pub fn require_language(provider: CatalogProvider, language: LanguagePreference) -> Result<()> {
    match (provider, language) {
        (_, LanguagePreference::Default) => Ok(()),
        (CatalogProvider::JkAnime, LanguagePreference::Spanish) => Ok(()),
        (CatalogProvider::TioAnime, LanguagePreference::Spanish) => Ok(()),
        (provider, LanguagePreference::Spanish) => Err(AniError::Unavailable(format!(
            "Spanish audio/subtitles are not available from the {provider} catalog in this experimental phase"
        ))),
    }
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SearchOptions {
    /// Include titles marked as adult by the selected catalog.
    pub allow_adult: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct SearchResult {
    pub id: String,
    pub name: String,
    pub episodes: f64,
    #[serde(default)]
    pub provider: CatalogProvider,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SubtitleTrack {
    pub label: String,
    pub url: String,
    #[serde(default)]
    pub default: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct RequestHeaders {
    pub referer: Option<String>,
    pub origin: Option<String>,
    #[serde(default)]
    pub extra: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct StreamLink {
    pub url: String,
    pub resolution: String,
    pub hls: bool,
    pub provider: String,
    pub downloadable: bool,
    #[serde(default)]
    pub headers: RequestHeaders,
    #[serde(default)]
    pub subtitles: Vec<SubtitleTrack>,
}

fn resolution_weight(value: &str) -> i32 {
    let digits: String = value.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().unwrap_or_else(|_| {
        if value.eq_ignore_ascii_case("auto") {
            -1
        } else {
            0
        }
    })
}

fn provider_weight(value: &str) -> i32 {
    let value = value.to_ascii_lowercase();
    if value.contains("s-mp4") {
        3_000
    } else if value.contains("mp4") {
        2_000
    } else if value.contains("default") {
        1_000
    } else {
        0
    }
}

pub(crate) fn sort_streams(streams: &mut [StreamLink]) {
    streams.sort_by(|a, b| {
        provider_weight(&b.provider)
            .cmp(&provider_weight(&a.provider))
            .then_with(|| resolution_weight(&b.resolution).cmp(&resolution_weight(&a.resolution)))
            .then_with(|| b.hls.cmp(&a.hls))
            .then_with(|| a.provider.cmp(&b.provider))
    });
}

pub fn choose_quality<'a>(streams: &'a [StreamLink], quality: &str) -> Option<&'a StreamLink> {
    if streams.is_empty() {
        return None;
    }
    match quality.to_ascii_lowercase().as_str() {
        "best" => streams.first(),
        "worst" => streams
            .iter()
            .filter(|s| resolution_weight(&s.resolution) > 0)
            .min_by_key(|s| resolution_weight(&s.resolution))
            .or_else(|| streams.last()),
        requested => streams
            .iter()
            .find(|s| s.resolution.to_ascii_lowercase().contains(requested))
            .or_else(|| streams.first()),
    }
}

fn episode_number(value: &str) -> Option<f64> {
    value.parse::<f64>().ok().filter(|n| n.is_finite())
}

pub fn sort_episodes(episodes: &mut [String]) {
    episodes.sort_by(|a, b| match (episode_number(a), episode_number(b)) {
        (Some(a), Some(b)) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
        _ => a.cmp(b),
    });
}

pub fn expand_episode_selection(selection: &str, available: &[String]) -> Result<Vec<String>> {
    let trimmed = selection.trim();
    if trimmed == "-1" {
        return available
            .last()
            .cloned()
            .map(|v| vec![v])
            .ok_or_else(|| AniError::UnavailableNoEpisodes);
    }
    if trimmed.contains(char::is_whitespace) {
        let requested: Vec<_> = trimmed.split_whitespace().map(str::to_owned).collect();
        if requested.iter().all(|v| available.contains(v)) {
            return Ok(requested);
        }
        eprintln!("One or more selected episodes do not exist");
        return Err(AniError::InputInvalidEpisode);
    }
    if let Some((start, end)) = trimmed.split_once('-') {
        let end = if end == "-1" || end.is_empty() {
            available.last().map(String::as_str).unwrap_or("")
        } else {
            end
        };
        let start_index = available
            .iter()
            .position(|v| v == start)
            .ok_or_else(|| AniError::InputInvalidEpisode)?;
        let end_index = available
            .iter()
            .position(|v| v == end)
            .ok_or_else(|| AniError::InputInvalidEpisode)?;
        if start_index > end_index {
            eprintln!("Episode range is reversed");
            return Err(AniError::InputInvalidEpisode);
        }
        return Ok(available[start_index..=end_index].to_vec());
    }
    if available.iter().any(|v| v == trimmed) {
        Ok(vec![trimmed.to_owned()])
    } else {
        Err(AniError::InputInvalidEpisode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stream(resolution: &str) -> StreamLink {
        StreamLink {
            url: resolution.into(),
            resolution: resolution.into(),
            hls: false,
            provider: "Default".into(),
            downloadable: true,
            headers: RequestHeaders::default(),
            subtitles: vec![],
        }
    }

    #[test]
    fn quality_selection_falls_back_to_best() {
        let streams = vec![stream("1080p"), stream("720p"), stream("480p")];
        assert_eq!(
            choose_quality(&streams, "worst").unwrap().resolution,
            "480p"
        );
        assert_eq!(choose_quality(&streams, "720").unwrap().resolution, "720p");
        assert_eq!(
            choose_quality(&streams, "1440p").unwrap().resolution,
            "1080p"
        );
    }

    #[test]
    fn expands_ranges_and_latest() {
        let eps = vec!["1".into(), "2".into(), "2.5".into(), "3".into()];
        assert_eq!(
            expand_episode_selection("2-3", &eps).unwrap(),
            vec!["2", "2.5", "3"]
        );
        assert_eq!(expand_episode_selection("-1", &eps).unwrap(), vec!["3"]);
    }

    #[test]
    fn language_preference_parses_spanish_only() {
        assert_eq!(
            LanguagePreference::from_str("es").unwrap(),
            LanguagePreference::Spanish
        );
        assert_eq!(
            LanguagePreference::from_str(" ES ").unwrap(),
            LanguagePreference::Spanish
        );
        assert!(LanguagePreference::from_str("xx").is_err());
    }

    #[test]
    fn regional_variants_fail_with_planned_message() {
        // NOTE: `AniError::Input` displays as plain "invalid input"; the
        // detail lives in the inner message, so match the variant.
        for variant in ["es-419", "es-ES"] {
            match LanguagePreference::from_str(variant) {
                Err(AniError::Input(message)) => assert!(
                    message.contains("not supported yet"),
                    "unexpected message for {variant}: {message}"
                ),
                other => panic!("expected planned Input error for {variant}, got {other:?}"),
            }
        }
    }

    #[test]
    fn spanish_gate_passes_default_and_jkanime_only() {
        for provider in [
            CatalogProvider::Anikoto,
            CatalogProvider::Anikoto2,
            CatalogProvider::JkAnime,
            CatalogProvider::TioAnime,
        ] {
            require_language(provider, LanguagePreference::Default).unwrap();
        }
        require_language(CatalogProvider::JkAnime, LanguagePreference::Spanish).unwrap();
        require_language(CatalogProvider::TioAnime, LanguagePreference::Spanish).unwrap();
        assert!(require_language(CatalogProvider::Anikoto, LanguagePreference::Spanish).is_err());
        assert!(require_language(CatalogProvider::Anikoto2, LanguagePreference::Spanish).is_err());
    }

    #[test]
    fn search_provider_auto_routes_spanish_to_jkanime() {
        use CatalogProvider as P;
        assert_eq!(
            effective_search_provider(None, LanguagePreference::Default),
            CatalogProvider::default()
        );
        assert_eq!(
            effective_search_provider(None, LanguagePreference::Spanish),
            P::JkAnime
        );
        for provider in [P::Anikoto, P::Anikoto2, P::JkAnime, P::TioAnime] {
            assert!(provider.spanish_capable() == matches!(provider, P::JkAnime | P::TioAnime));
            for language in [LanguagePreference::Default, LanguagePreference::Spanish] {
                assert_eq!(
                    effective_search_provider(Some(provider), language),
                    provider
                );
            }
        }
    }

    #[test]
    fn fallback_triggers_on_provider_failures_only() {
        let empty: Result<Vec<SearchResult>> = Ok(Vec::new());
        assert!(is_fallback_trigger(&empty));
        assert!(is_fallback_trigger::<Vec<SearchResult>>(&Err(
            AniError::UnavailableNoResults
        )));
        assert!(is_fallback_trigger::<Vec<SearchResult>>(&Err(
            AniError::Network("down".into())
        )));
        assert!(!is_fallback_trigger::<Vec<SearchResult>>(&Err(
            AniError::InputEmptyQuery
        )));
        assert!(!is_fallback_trigger::<Vec<SearchResult>>(&Err(
            AniError::InputSelectionOutOfRange
        )));
    }

    #[test]
    fn spanish_providers_returns_jk_tio_in_order() {
        let providers = CatalogProvider::spanish_providers();
        assert_eq!(providers[0], CatalogProvider::JkAnime);
        assert_eq!(providers[1], CatalogProvider::TioAnime);
    }

    #[test]
    fn should_fanout_spanish_requires_no_provider_and_es() {
        assert!(should_fanout_spanish(None, LanguagePreference::Spanish));
        assert!(!should_fanout_spanish(
            Some(CatalogProvider::JkAnime),
            LanguagePreference::Spanish
        ));
        assert!(!should_fanout_spanish(None, LanguagePreference::Default));
        assert!(!should_fanout_spanish(
            Some(CatalogProvider::TioAnime),
            LanguagePreference::Default
        ));
    }

    #[test]
    fn is_fallback_error_classifies_provider_side_only() {
        assert!(is_fallback_error(&AniError::Network("x".into())));
        assert!(is_fallback_error(&AniError::Provider("x".into())));
        assert!(is_fallback_error(&AniError::Catalog {
            provider: "x".into(),
            message: "y".into()
        }));
        assert!(is_fallback_error(&AniError::ProviderRateLimited {
            provider: "x".into(),
            retry_after_seconds: 1
        }));
        assert!(is_fallback_error(&AniError::Unavailable("x".into())));
        assert!(is_fallback_error(&AniError::UnavailableNoResults));
        assert!(is_fallback_error(&AniError::UnavailableNoEpisodes));
        assert!(is_fallback_error(&AniError::UnavailableNoStreams));
        assert!(!is_fallback_error(&AniError::InputEmptyQuery));
        assert!(!is_fallback_error(&AniError::InputSelectionOutOfRange));
        assert!(!is_fallback_error(&AniError::Input("x".into())));
    }

    #[test]
    fn merge_spanish_search_concats_jk_then_tio() {
        let jk = Ok(vec![SearchResult {
            id: "jkanime:a".into(),
            name: "A".into(),
            episodes: 1.0,
            provider: CatalogProvider::JkAnime,
        }]);
        let tio = Ok(vec![SearchResult {
            id: "tioanime:b".into(),
            name: "B".into(),
            episodes: 1.0,
            provider: CatalogProvider::TioAnime,
        }]);
        let merged = merge_spanish_search(jk, tio).unwrap();
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].provider, CatalogProvider::JkAnime);
        assert_eq!(merged[1].provider, CatalogProvider::TioAnime);
    }

    #[test]
    fn merge_spanish_search_isolates_provider_failures() {
        let jk = Ok(vec![SearchResult {
            id: "jkanime:a".into(),
            name: "A".into(),
            episodes: 1.0,
            provider: CatalogProvider::JkAnime,
        }]);
        let tio = Err(AniError::Network("tio down".into()));
        let merged = merge_spanish_search(jk, tio).unwrap();
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].provider, CatalogProvider::JkAnime);
    }

    #[test]
    fn merge_spanish_search_all_fail_returns_last_fallback_error() {
        let jk = Err(AniError::Network("jk down".into()));
        let tio = Err(AniError::UnavailableNoResults);
        let err = merge_spanish_search(jk, tio).unwrap_err();
        assert!(is_fallback_error(&err));
    }

    #[test]
    fn merge_spanish_search_non_fallback_fails_fast() {
        let jk = Err(AniError::InputEmptyQuery);
        let tio = Ok(vec![]);
        let err = merge_spanish_search(jk, tio).unwrap_err();
        assert!(matches!(err, AniError::InputEmptyQuery));
    }
}
