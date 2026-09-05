# ani-cli-rs 0.10.2

## Unreleased

### Search history (`--history`)
- `SearchHistory` stored in a separate `ani-search-hsts` file (not the Bash-compat `ani-hsts` watch log), capped at 50 entries, deduplicated, newest-first.
- Interactive flow offers recent queries in a picker before the typing prompt; `--select-nth` and pipes keep historic straight-to-prompt behavior.
- New subcommand `history --json` / `history --clear`; `-D`/`--delete` keeps deleting just the Bash-compat watch log.
- Suite 124 OK (86 lib + 23 bin + 19 CLI); tests isolate `ANI_CLI_HIST_DIR` for every CLI test that executes `search` (Regla 11.1).

### Fix decode IDs sin padding + retry JKAnime (Phase 12)
- `decode_id` in `jkanime.rs`/`tioanime.rs` now tries base64+JSON first and slug second — One Punch Man 3 (unpadded base64 that matched the slug regex) was routing to `/{base64}/` → HTTP 404; Black Torch worked only because its base64 ends in `==`.
- JKAnime `get_text`/`post_text` retry once after 800 ms on catalog HTTP 404 (transient WAF burst load), verified live with One Punch Man 3 episode 2 HLS; 2 wiremock tests + 2 regression tests with real unpadded IDs.
- Suite 128 OK (86 lib + 23 bin + 19 CLI).

### HLS cache generoso en mpv (Phase 13 / Regla 13.1)
- JKAnime HLS streams stalled every few seconds on fast links (850 Mbps cable) because mpv used its tiny default cache for segmented HLS — not a bandwidth issue but a buffer one. The `mpv_options` guard only applied generous cache (`--cache=yes --cache-secs=120 --demuxer-max-bytes=512MiB`) when `stream.hls && requires_hls_relay(stream)`, which only matched Anikoto/KotoCDN hosts; JKAnime direct HLS and the `127.0.0.1` relay URL both gave `false`. Fix: apply cache tuning to all `stream.hls` regardless of host/provider; MP4 progressive streams untouched.
- New regression test `hls_streams_get_generous_cache_even_without_anikoto_host` covers JKAnime direct + verifies MP4 does not receive cache flags.
- Suite 129 OK (87 lib + 23 bin + 19 CLI).

### TioAnime browser-only UX improvements (this cycle)
- **Opción 1 — Error message:** `browser_fallback_suffix` now says: `"This episode on TioAnime only has browser-only servers (Mega, Voe, Netu, etc.) — YourUpload was removed upstream (novideo.mp4). Try another episode (e.g., 2) or provider JKAnime, or open in a browser: <links>"` — clearly naming the servers and explaining the situation.
- **Opción 2 — Auto-skip:** When an episode fails with a browser-only error (TioAnime with Mega/Voe/Netu/YourUpload-novideo), `play_or_download_with_skip` automatically tries the next available episode and prints: `"⚠ Episode X of "Anime" is only available on browser-only servers (Mega, Voe, Netu, etc.) — not playable directly in the CLI. Auto-skipping to episode Y..."`. If it was the last episode, the original error is returned. Works in both the main playback loop and `interactive_after_play`. Helper `is_browser_only_error` exported from `ani_lib` root.
- Mega/Voe/Netu/VidGuard stay browser-only: Voe hides behind `eugenemakedraw.com` + obfuscated player, Mega needs its file-key API, Netu behind Cloudflare challenges — verified live 2026-09-05 and intentionally not reverse-engineered (Regla 10.1).
- Suite 129 OK.

### `clippy --all-targets -- -D warnings` on toolchain 1.96
- 14 `ok_or_else(|| …)` → `ok_or(…)` fixes in unit-variant sites; no behavior change.

### Full test suite
- Suite 129 OK (87 lib + 23 bin + 19 CLI).

**Full changelog:** [0.10.1...0.10.2](https://github.com/vorlie/ani-cli-rs/compare/v0.10.1...v0.10.2)

---

# ani-cli-rs 0.10.1

## Released 2026-09-05

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
