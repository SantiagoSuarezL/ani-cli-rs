# Technical Specification — Spanish Language Support

## 1. Scope

This specification defines the data and selection behavior for Spanish support. It intentionally does not prescribe implementation details until provider research is complete.

## 2. CLI

### New option

```text
--language <LANGUAGE>
```

Accepted initial values:

```text
es
es-419
es-ES
```

The implementation may retain the project's existing aliases/conventions if they exist.

### Semantics

`es`:
- Spanish requested.
- If only one Spanish variant is available, select it.
- If multiple variants are available for the selected anime/episode, ask the user to choose.

`es-419`:
- Prefer Latin American Spanish.
- Do not silently replace it with `es-ES`.

`es-ES`:
- Prefer Spain Spanish.
- Do not silently replace it with `es-419`.

## 3. Language metadata

Provider results should normalize language information.

Conceptual structure:

```text
LanguageInfo
 ├── language: es
 ├── region: 419 | ES | none
 ├── type: subtitle | audio | video
 └── label
```

Where provider information is insufficient to distinguish region, use generic `es` rather than guessing.

## 4. Stream requirements

A Spanish-compatible stream is any normalized stream satisfying one of:

1. Spanish subtitle track exists.
2. Spanish audio exists.
3. Provider explicitly identifies the video as Spanish-language.

Preference should be given to actual subtitle-track metadata when available.

## 5. Quality selection

Within the selected language:

```text
1080p > 720p > 480p > lower quality
```

Do not compare quality before enforcing the user's exact language variant.

Example:

```text
Request: es-419

Result A: es-419 / 720p
Result B: es-ES  / 1080p

Winner: Result A
```

## 6. Generic Spanish ambiguity

Example:

```text
Request: es

Available:
- es-419 / 1080p
- es-ES / 1080p
```

Prompt:

```text
Spanish versions available:

> Español Latinoamericano
  Español de España
```

After selection, ranking continues only inside that variant.

## 7. No-result behavior

If no Spanish result exists:

```text
No Spanish version is currently available for this anime.

Would you like to continue in English?
> Yes
  No
```

If the user declines, exit cleanly using existing application behavior.

## 8. Provider failure behavior

A provider returning:
- timeout
- HTTP error
- malformed response
- missing stream
- unusable stream

must be treated as a provider failure and allow the next provider to be evaluated where the existing architecture permits.

## 9. Subtitle handling

Do not create a second subtitle subsystem.

Normalize provider subtitle information into the project's existing `SubtitleTrack` representation.

When multiple subtitle languages exist, the Spanish track should be selected according to the language preference.

## 10. Testing requirements

Minimum test categories:

### Language parsing
- `es`
- `es-419`
- `es-ES`
- invalid language

### Ranking
- same language, different quality
- different Spanish variants, different quality
- unavailable preferred provider
- multiple providers with same variant

### Fallback
- Spanish available
- Spanish unavailable / English available
- no usable provider

### Regression
- existing no-language command
- existing Anikoto provider
- existing quality selection
- existing subtitle handling
- existing player invocation

## 11. Provider acceptance criteria

A provider should not be considered production-ready until it has demonstrated:

- search
- anime identification
- episode enumeration
- usable stream extraction
- quality information
- language information
- reproducible behavior on representative titles
- failure handling
