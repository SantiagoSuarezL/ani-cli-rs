# ani-cli-rs 0.10.1

## Spanish language experimental support (`--language es`)

### Provider selection and fallback (Phase 5)
- Multi-provider fan-out: `--language es` searches both `JKAnime` and `TioAnime` in parallel (`tokio::join!`), merges results in reliability order (JKAnime first for backward-compatible `--select-nth` ordering), and tags results with their provider (`[jkanime]`, `[tioanime]`).
- Provider isolation: provider-side failures (`Network`, `Provider`, `Catalog`, `RateLimited`, `Unavailable`) are isolated per `ARCHITECTURE.md` §10 — one failing catalog does not terminate the complete search.
- Explicit English fallback: interactive `Yes/No` prompt (`dialoguer`) with deterministic non-interactive error (`--json`, pipes, non-terminal stdin) explaining how to rerun without `--language es`.
- Language gate: `require_language()` allows only `JKAnime` and `TioAnime` for `--language es`; other providers reject explicitly with a clear message (`not available`).
- Regional variants: `es-419` and `es-ES` return an explicit planned-support error (`not supported yet; only --language es`), consistent with live evidence showing no structured region distinction.

### UX polish (Phase 6)
- Interactive flow messages: `Searching providers...`, `✓ Black Torch`, `Season:`, `Opening best available stream...`.
- Picker tags: when results span multiple providers (`[jkanime]` / `[tioanime]`), the interactive picker shows provider tags (`format!("{} ({} episodes) [{}]", ...)`); single-catalog results keep the historic untagged label.
- GUI language selector: `GuiState` gains `language` field initialized from `ANI_CLI_LANGUAGE` env; `render_search()` adds a `Language` dropdown (`Español (es)` / `Default`); `perform_search()` applies fan-out when `language == Spanish` and `provider == Anikoto` (default for `es` without explicit provider).

### Regression and compatibility (Phase 7)
- 115 tests passing (`cargo test --locked`): 78 lib + 20 bin + 16 CLI + 1 new CLI (`help_documents_spanish_fanout`).
- `cargo fmt --check`: clean.
- `cargo build --release --features gui`: compiles.
- CLI/scriptable commands verified live: `search`, `episodes`, `links`, `--provider`, `--language`, `--quality best`, `--json`, `--select-nth`.
- No regressions in default `Anikoto2` provider; existing `Anikoto`/`Anikoto2` behavior preserved.
- Memory updated: `roadmap.md` (Fase 5-7 `[x]`), `session_log.md` (rotated with Fase 5/6/7 detail), `observations.md` (Fase 5 completed line added).

### Provider assumptions (documented)
- `JKAnime` (`https://jkanime.net`): single `Japones Sub. Español` catalog, HLS 1080p, hardcoded subtitles (`Subtitulado`), verified live 2026-09-04. No `es-419`/`es-ES` distinction observed.
- `TioAnime` (`https://tioanime.com`): single `sub español` catalog, MP4 direct (`YourUpload` with `Referer` required), hardcoded subtitles, no `es-419`/`es-ES` distinction observed. Every page says `Subtitulado`; even `dragon-ball-super-latino-1` says `Subtitulado`, confirming no structured region metadata.
- `AnimeFLV` (`animeflv.net` + mirrors): unstable (`521` from this location). Clone (`animeflv.or.at`) only shows `sub español`. Addon code (`routes/animeFLV.js`) splits `SUB`/`DUB` without `es-419`/`es-ES`. No Priority-A provider distinguishes Spanish regional variants; model frozen at generic `es`.

### Files changed
- `src/models.rs`: `spanish_providers()`, `should_fanout_spanish()`, `is_fallback_error()`, `merge_spanish_search()`, 7 new unit tests.
- `src/lib.rs`: exports updated.
- `src/main.rs`: interactive/scriptable search fan-out (`search_spanish`), picker tags, language help text, flow messages.
- `tests/cli.rs`: `help_documents_spanish_fanout`.
- `tests/` (models): 4 new `merge_spanish_search` tests.
- `README.md`: `--language es` in quick start and features.
- `.agent/memory/`: `roadmap.md`, `session_log.md`, `observations.md` updated.

**Full changelog:** [0.10.0...0.10.1](https://github.com/vorlie/ani-cli-rs/compare/v0.10.0...v0.10.1)
