use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{AniError, Result};

/// Cap for the search-query log: recent queries beyond this are dropped
/// (oldest first). Keeps the file small and the recents picker snappy.
pub const SEARCH_HISTORY_LIMIT: usize = 50;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryEntry {
    pub episode: String,
    pub show_id: String,
    pub title: String,
}

#[derive(Clone, Debug)]
pub struct HistoryStore {
    path: PathBuf,
}

impl HistoryStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn platform_default() -> Result<Self> {
        Ok(Self::new(state_dir()?.join("ani-hsts")))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn entries(&self) -> Result<Vec<HistoryEntry>> {
        let text = match tokio::fs::read_to_string(&self.path).await {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(error) => return Err(error.into()),
        };
        Ok(text
            .lines()
            .filter_map(|line| {
                let mut values = line.splitn(3, '\t');
                Some(HistoryEntry {
                    episode: values.next()?.into(),
                    show_id: values.next()?.into(),
                    title: values.next()?.into(),
                })
            })
            .collect())
    }

    pub async fn update(&self, entry: HistoryEntry) -> Result<()> {
        let mut entries = self.entries().await?;
        entry_valid(&entry)?;
        if let Some(existing) = entries
            .iter_mut()
            .find(|value| value.show_id == entry.show_id)
        {
            *existing = entry;
        } else {
            entries.push(entry);
        }
        self.write(&entries).await
    }

    pub async fn clear(&self) -> Result<()> {
        self.write(&[]).await
    }

    async fn write(&self, entries: &[HistoryEntry]) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let text = entries
            .iter()
            .map(|value| {
                format!(
                    "{}\t{}\t{}\n",
                    value.episode,
                    value.show_id,
                    value.title.replace(['\t', '\n'], " ")
                )
            })
            .collect::<String>();
        let temporary = self.path.with_extension("new");
        tokio::fs::write(&temporary, text).await?;
        if tokio::fs::try_exists(&self.path).await? {
            tokio::fs::remove_file(&self.path).await?;
        }
        tokio::fs::rename(temporary, &self.path).await?;
        Ok(())
    }
}

fn entry_valid(entry: &HistoryEntry) -> Result<()> {
    if entry.episode.is_empty() || entry.show_id.is_empty() || entry.title.is_empty() {
        Err(AniError::History(
            "history entry contains an empty field".into(),
        ))
    } else {
        Ok(())
    }
}

/// Shared state directory for both history files. `ANI_CLI_HIST_DIR`
/// overrides the platform default so tests and portable installs can
/// relocate the watch log and the search log together.
fn state_dir() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("ANI_CLI_HIST_DIR") {
        return Ok(PathBuf::from(path));
    }
    let project = directories::ProjectDirs::from("org", "ani-cli", "ani-cli")
        .ok_or(AniError::HistoryStateDirectory)?;
    Ok(project
        .state_dir()
        .unwrap_or_else(|| project.data_local_dir())
        .into())
}

/// One remembered search query: what was typed plus which catalog/language
/// scope it ran in (e.g. `es via jkanime+tioanime`, `anikoto2`). The store
/// lives in a separate file from the Bash-compatible watch log on purpose:
/// `ani-hsts` keeps its 3-column shape so the original ani-cli still reads
/// it, while search queries (known before any show is picked) get their own
/// log with timestamps for newest-first recall.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SearchEntry {
    pub query: String,
    pub context: String,
    pub timestamp: u64,
}

/// Append-only (capped) log of search queries in `ani-search-hsts`, next to
/// the watch log. Recording is best-effort by design: callers use `let _ =`
/// so a read-only state dir never fails a search.
#[derive(Clone, Debug)]
pub struct SearchHistory {
    path: PathBuf,
}

impl SearchHistory {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn platform_default() -> Result<Self> {
        Ok(Self::new(state_dir()?.join("ani-search-hsts")))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Entries newest-first. Malformed lines are skipped, never fatal.
    pub async fn entries(&self) -> Result<Vec<SearchEntry>> {
        let text = match tokio::fs::read_to_string(&self.path).await {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(error) => return Err(error.into()),
        };
        let mut entries = text
            .lines()
            .filter_map(|line| {
                let mut values = line.splitn(3, '\t');
                Some(SearchEntry {
                    timestamp: values.next()?.parse().ok()?,
                    query: values.next()?.into(),
                    context: values.next()?.into(),
                })
            })
            .filter(|entry| !entry.query.is_empty())
            .collect::<Vec<_>>();
        entries.reverse();
        Ok(entries)
    }

    /// Records a query: trims, ignores empties, de-duplicates by query
    /// (re-searching bumps it to the top with fresh context/timestamp) and
    /// caps the log at [`SEARCH_HISTORY_LIMIT`] (oldest dropped).
    pub async fn record(&self, query: &str, context: &str) -> Result<()> {
        let query = query.split_whitespace().collect::<Vec<_>>().join(" ");
        if query.is_empty() {
            return Ok(());
        }
        let mut entries = self.entries().await?;
        entries.retain(|entry| entry.query != query);
        entries.insert(
            0,
            SearchEntry {
                query,
                context: context.into(),
                timestamp: unix_now(),
            },
        );
        entries.truncate(SEARCH_HISTORY_LIMIT);
        self.write(&entries).await
    }

    pub async fn clear(&self) -> Result<()> {
        self.write(&[]).await
    }

    async fn write(&self, newest_first: &[SearchEntry]) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        // Persist oldest-first so the file reads chronologically; entries()
        // reverses back to newest-first.
        let text = newest_first
            .iter()
            .rev()
            .map(|value| {
                format!(
                    "{}\t{}\t{}\n",
                    value.timestamp,
                    value.query.replace(['\t', '\n'], " "),
                    value.context.replace(['\t', '\n'], " ")
                )
            })
            .collect::<String>();
        let temporary = self.path.with_extension("new");
        tokio::fs::write(&temporary, text).await?;
        if tokio::fs::try_exists(&self.path).await? {
            tokio::fs::remove_file(&self.path).await?;
        }
        tokio::fs::rename(temporary, &self.path).await?;
        Ok(())
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn reads_and_updates_legacy_format() {
        let directory = tempfile::tempdir().unwrap();
        let store = HistoryStore::new(directory.path().join("ani-hsts"));
        store
            .update(HistoryEntry {
                episode: "1".into(),
                show_id: "abc".into(),
                title: "Anime".into(),
            })
            .await
            .unwrap();
        store
            .update(HistoryEntry {
                episode: "2".into(),
                show_id: "abc".into(),
                title: "Anime".into(),
            })
            .await
            .unwrap();
        assert_eq!(
            store.entries().await.unwrap(),
            vec![HistoryEntry {
                episode: "2".into(),
                show_id: "abc".into(),
                title: "Anime".into()
            }]
        );
    }

    #[tokio::test]
    async fn search_history_round_trips_newest_first() {
        let directory = tempfile::tempdir().unwrap();
        let store = SearchHistory::new(directory.path().join("ani-search-hsts"));
        assert!(store.entries().await.unwrap().is_empty());
        store.record("black torch", "es via jkanime").await.unwrap();
        store.record("  frieren  ", "anikoto2").await.unwrap();
        // Empty/whitespace-only queries are ignored.
        store.record("   ", "anikoto2").await.unwrap();
        let entries = store.entries().await.unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].query, "frieren");
        assert_eq!(entries[0].context, "anikoto2");
        assert_eq!(entries[1].query, "black torch");
        assert!(entries[0].timestamp > 0);
    }

    #[tokio::test]
    async fn search_history_dedupes_and_caps() {
        let directory = tempfile::tempdir().unwrap();
        let store = SearchHistory::new(directory.path().join("ani-search-hsts"));
        store.record("naruto", "anikoto").await.unwrap();
        store.record("bleach", "anikoto").await.unwrap();
        // Re-searching bumps to the top with fresh context.
        store.record("naruto", "es via tioanime").await.unwrap();
        let entries = store.entries().await.unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].query, "naruto");
        assert_eq!(entries[0].context, "es via tioanime");
        for index in 0..(SEARCH_HISTORY_LIMIT + 10) {
            store
                .record(&format!("anime-{index}"), "anikoto")
                .await
                .unwrap();
        }
        let entries = store.entries().await.unwrap();
        assert_eq!(entries.len(), SEARCH_HISTORY_LIMIT);
        assert_eq!(
            entries[0].query,
            format!("anime-{}", SEARCH_HISTORY_LIMIT + 9)
        );
        // Malformed lines are skipped, never fatal.
        tokio::fs::write(store.path(), "not-a-timestamp\tnope\n")
            .await
            .unwrap();
        assert!(store.entries().await.unwrap().is_empty());
        store.record("one piece", "anikoto").await.unwrap();
        store.clear().await.unwrap();
        assert!(store.entries().await.unwrap().is_empty());
    }
}
