# Implementation Plan — Spanish Language Support

## Status

Planning only. No implementation should begin until provider research is complete.

## Phase 0 — Design
- [x] Fork `ani-cli-rs`
- [x] Define Spanish feature objective
- [x] Define `es`, `es-419`, `es-ES`
- [x] Decide that language preference outranks quality
- [x] Define explicit choice when generic `es` has multiple variants
- [x] Define explicit English fallback
- [x] Preserve current default behavior
- [x] Preserve existing player/subtitle architecture
- [x] Define provider isolation and ranking principles

## Phase 1 — Provider research

### Priority A
1. JKAnime
2. TioAnime
3. AnimeFLV

### Priority B
4. AnimeJara
5. Henaojara
6. AnimeAV1
7. AnimeOnline/Ninja
8. MonosChinos

For each provider document:

```text
Identity
- current domain
- search mechanism
- title matching
- season handling

Episodes
- episode numbering
- specials/OVAs
- pagination

Streams
- server list
- direct/HLS/other formats
- URL lifetime
- required headers
- quality information

Language
- ES-419
- ES-ES
- generic ES
- subtitles vs dubbed audio
- embedded vs external subtitles

Reliability
- current availability
- failure modes
- maintenance activity
- known domain changes

Integration
- mapping to existing Provider abstraction
- mapping to StreamLink
- mapping to SubtitleTrack
- dependencies
```

### Research references

Use active implementations as behavioral references, especially Pigamer37's AnimES addon, which currently aggregates AnimeFLV, AnimeAV1, Henaojara, TioAnime, AnimeJara and JKAnime. The repository's recent workflow history also shows active provider work and fixes. Do not copy code blindly; use it to understand provider behavior.

Historical repositories such as `chrismichaelps/jkanime`, `jkanime-v2`, `ryuanime`, and `carlosfdezb/tioanime` should be treated as documentation of older provider behavior, not proof that their current endpoints still work.

Jikan is a metadata/identity candidate, not a video provider.

## Phase 2 — Provider proof of concept

Start with the provider that has the best combination of:
- active evidence
- Spanish availability
- stable stream resolution
- maintainability
- simple integration
- low dependency footprint

Implement only enough to:
- search
- select anime
- list episodes
- resolve one stream
- expose Spanish language metadata
- play through existing player code

## Phase 3 — Language normalization

Implement:

```text
es
es-419
es-ES
```

Add parsing and normalized comparison.

Do not yet optimize provider ranking beyond deterministic language/quality selection.

## Phase 4 — Multiple providers

Add providers one at a time.

For every provider:
1. Implement provider boundary.
2. Normalize results.
3. Add provider-specific tests.
4. Test representative Spanish titles.
5. Test failure behavior.
6. Verify no regression in existing providers.

## Phase 5 — Provider selection and fallback

Implement ordered selection:

```text
1. Requested language variant
2. Working stream
3. Subtitle/audio compatibility
4. Highest quality
5. Provider reliability
```

For generic `es`:

```text
one Spanish variant → automatic
multiple Spanish variants → user choice
```

For exact `es-419` / `es-ES`:

```text
exact variant only
```

If no Spanish exists:

```text
explicit English fallback prompt
```

## Phase 6 — UX polish

Ensure the normal flow resembles:

```text
$ ani-cli-rs --language es "Black Torch"

Searching providers...

✓ Black Torch

Season:
> Season 1

Episode:
> 1

Spanish versions:
> Español Latinoamericano
  Español de España

Opening best available stream...
```

The quality should be selected automatically unless the existing application already requires or offers interactive quality selection.

## Phase 7 — Regression and compatibility

Test:
- Windows
- Linux
- existing Anikoto providers
- mpv
- VLC if supported by existing player abstraction
- subtitles
- history
- continuation
- downloads where applicable
- JSON/scriptable commands

The feature is complete only when Spanish support does not degrade existing workflows.

## Phase 8 — Documentation and upstream preparation

Update:
- README
- CLI help
- provider documentation
- architecture documentation
- tests
- changelog

Before contacting upstream:
- minimize unrelated changes
- split commits logically
- document provider assumptions
- explain why language selection is user-controlled
- provide reproducible examples
- ensure the implementation follows the project's license and contribution requirements.

## Suggested commit sequence

```text
1. docs: define Spanish language support
2. feat(provider): add first Spanish provider
3. feat(language): normalize Spanish variants
4. feat(selection): rank Spanish streams by preference
5. feat(fallback): add explicit language fallback
6. test: cover Spanish selection and provider failures
7. docs: document Spanish providers and CLI usage
```

Do not create all commits in advance; adapt them to the actual code discovered during implementation.
