# PRD — Spanish Language Support

## Status
Draft — Phase 0 / design only.

## 1. Objective
Add Spanish-language discovery and playback to `ani-cli-rs` while preserving the existing user experience and provider architecture as much as possible.

The primary user flow should be:

```bash
ani-cli-rs --language es "Black Torch"
```

The application searches supported providers, lets the user select the anime/season/episode using the existing interactive flow, and then selects the best available Spanish stream according to the user's explicit language preference and quality preference.

## 2. User goals
- Watch anime with Spanish subtitles or Spanish-language video when available.
- Distinguish Latin American Spanish (`es-419`) from Spain Spanish (`es-ES`).
- Choose the preferred Spanish variant even when that choice has lower video quality.
- Avoid forcing the user to choose a provider manually in the normal flow.
- Preserve the current default behavior when `--language` is not supplied.
- Fall back gracefully when Spanish is unavailable.

## 3. Non-goals
- Replacing mpv or redesigning the player.
- Rewriting the existing Anikoto providers.
- Circumventing DRM, authentication, paywalls, or access controls.
- Building a torrent/debrid system for the initial feature.
- Guaranteeing that every anime is available in every Spanish variant.

## 4. User-facing behavior

### Default
```bash
ani-cli-rs "Black Torch"
```
Keep the existing provider/language behavior.

### Spanish
```bash
ani-cli-rs --language es "Black Torch"
```
Interpret `es` as "Spanish". If both `es-419` and `es-ES` are available, the user must be allowed to choose the preferred variant.

### Explicit variants
```bash
ani-cli-rs --language es-419 "Black Torch"
ani-cli-rs --language es-ES "Black Torch"
```

These requests should prefer the exact requested variant and should not silently substitute the other variant.

## 5. Selection philosophy

Language preference has higher priority than quality.

Example:

```text
ES-419  720p
ES-ES   1080p
```

If the user selected `es-419`, choose ES-419 even though ES-ES has better quality.

Within the selected language/variant, prefer the highest working quality unless the user explicitly requests another quality.

## 6. Spanish fallback behavior

If no Spanish stream is available:

```text
No Spanish version is currently available for this anime.

Available language:
English

Continue in English?
> Yes
  No
```

The fallback must be explicit. The application must never silently change from Spanish to English.

## 7. Provider scope

Initial candidates for research/integration:
- JKAnime
- TioAnime
- AnimeFLV
- AnimeJara
- Henaojara
- AnimeAV1
- AnimeOnline/Ninja
- MonosChinos

The first implementation should use the smallest viable provider set and expand only when justified by availability/reliability.

## 8. Success criteria
- Existing commands continue working.
- `--language es` can discover Spanish content.
- `es-419` and `es-ES` can be represented distinctly.
- Provider selection is automatic after the user's language choice.
- Quality is optimized within the selected language.
- Spanish absence produces an actionable fallback message.
- Existing mpv/subtitle handling is reused.
