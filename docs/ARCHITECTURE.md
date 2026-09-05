# Architecture — Spanish Language Support

## 1. Design principle

Extend the existing `ani-cli-rs` architecture rather than replacing it.

Current project capabilities already include multiple providers, quality selection, subtitles, and mpv/VLC playback. The Spanish feature should fit into these abstractions rather than create a parallel application.

## 2. Target architecture

```text
CLI
 │
 ├── query
 ├── language preference
 └── existing interactive selections
        │
        ▼
Provider Manager
        │
        ├── Anikoto
        ├── Anikoto2
        ├── JKAnime
        ├── TioAnime
        ├── AnimeFLV
        └── future providers
        │
        ▼
Normalized Provider Results
        │
        ├── anime identity
        ├── season
        ├── episode
        └── StreamLink
              ├── video URL
              ├── quality
              ├── audio language
              └── subtitle tracks
        │
        ▼
Language Filter
        │
        ├── exact variant: es-419
        ├── exact variant: es-ES
        └── generic Spanish: es
        │
        ▼
Ranking / Selection
        │
        ├── language match
        ├── stream availability
        ├── subtitle availability
        └── quality
        │
        ▼
Existing Player Layer
        │
        └── mpv / existing supported players
```

## 3. Language model

Use BCP-47-style language identifiers where practical:

- `es` — generic Spanish
- `es-419` — Latin American Spanish
- `es-ES` — Spain Spanish

`es` is a preference category, not a third dialect.

### Generic Spanish selection

When the user specifies `es`:
1. Find all Spanish-compatible results.
2. If only one variant is available, use it.
3. If multiple variants are available, ask the user to choose.
4. Preserve the selected variant for subsequent stream/quality ranking.

## 4. Provider abstraction

Each provider should expose the smallest interface required by the existing project.

Conceptually:

```text
Provider
 ├── search
 ├── seasons / show metadata
 ├── episodes
 └── links / streams
```

Providers should return normalized project-native types instead of leaking provider-specific structures into the rest of the application.

## 5. Stream normalization

A normalized stream may contain:

```text
StreamLink
 ├── URL
 ├── resolution / quality
 ├── provider
 ├── HLS flag
 ├── downloadable flag
 └── subtitle tracks
       ├── language
       ├── label
       └── URL
```

The Spanish feature should consume the existing subtitle representation whenever possible.

## 6. Ranking

Ranking must be lexicographic rather than a single opaque score.

Recommended priority:

1. User-requested language variant.
2. Working/usable stream.
3. Subtitle/audio compatibility.
4. Highest quality within the language constraint.
5. Provider reliability as a tie-breaker.

This prevents a 1080p Spanish-of-the-wrong-variant result from beating a 720p result in the language the user explicitly requested.

## 7. Fallback

Fallback occurs in layers:

```text
Exact requested Spanish variant
        ↓
Other Spanish variant only if user selected generic `es`
        ↓
Other provider
        ↓
No Spanish
        ↓
Offer existing/default English flow
```

Explicit `es-419` or `es-ES` must not silently downgrade to another Spanish variant.

## 8. Metadata

Jikan may be used as a metadata/identity source, not as a streaming provider.

Its potential role is canonical anime identification and cross-provider matching.

## 9. Player

Do not redesign the player.

Use the existing player abstraction and existing mpv integration. External subtitles should continue to flow through the existing subtitle-track mechanism.

## 10. Reliability

Provider-specific failures must remain isolated. A failing provider should not terminate the complete search when other providers can be tried.

## Provider Manager (Fase 5-7 updates)

- `CatalogProvider::spanish_providers()` returns `[JkAnime, TioAnime]` in reliability order (`ARCH §6 #5`).
- `should_fanout_spanish()` determines when `--language es` without `--provider` should query all Spanish-capable catalogs in parallel (`tokio::join!`).
- `merge_spanish_search()` merges parallel results: provider-side failures (`is_fallback_error()`) are isolated per `ARCH §10`; non-fallback errors (`InputEmptyQuery`, etc.) fail fast; when all catalogs fail with fallback errors, the last error is preserved so the caller enters the explicit English-fallback path.
- `effective_search_provider()` keeps `JkAnime` as the single-catalog `es` target (backward-compatible with existing `--language es` behavior); fan-out replaces it only when `should_fanout_spanish()` is true.

## 11. Security

Only trust provider-specific domains/headers when explicitly configured by the provider integration. Do not implement DRM circumvention, credential extraction, or access-control bypasses.
