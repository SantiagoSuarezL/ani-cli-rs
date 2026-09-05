// src/lib.rs
//! Reusable Anikoto clients and cross-platform ani-cli support modules.

mod anikoto;
mod anikoto_cz;
mod download;
mod error;
mod history;
mod hls_relay;
mod i18n;
mod jkanime;
mod models;
mod player;
mod tioanime;

#[cfg(feature = "gui")]
pub mod gui;

pub use anikoto::{AnikotoClient, AnikotoClientBuilder, provider_from_show_id, requires_hls_relay};
pub use anikoto_cz::{AnikotoCzClient, AnikotoCzClientBuilder};
pub use download::{DownloadOptions, download_stream};
pub use error::{AniError, Result};
pub use history::{HistoryEntry, HistoryStore, SEARCH_HISTORY_LIMIT, SearchEntry, SearchHistory};
pub use hls_relay::{HlsRelay, relay_stream, relay_stream_without_hls_subtitles};
pub use i18n::{I18n, Locale};
pub use jkanime::{JkAnimeClient, JkAnimeClientBuilder};
pub use models::{
    CatalogProvider, LanguagePreference, RequestHeaders, SearchOptions, SearchResult, StreamLink,
    SubtitleTrack, TranslationType, choose_quality, effective_search_provider,
    expand_episode_selection, is_fallback_error, is_fallback_trigger, merge_spanish_search,
    require_language, should_fanout_spanish,
};
pub use player::{Player, PlayerKind, PlayerOptions};
pub use tioanime::{TioAnimeClient, TioAnimeClientBuilder, is_browser_only_error};
