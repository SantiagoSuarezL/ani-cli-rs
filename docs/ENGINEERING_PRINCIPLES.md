# Engineering Principles

## 1. Preserve `ani-cli-rs`

The existing project is the product. Spanish support is a feature.

Do not rewrite working systems merely to make the new feature look cleaner.

## 2. Minimal surface-area change

Prefer:
- existing provider interfaces
- existing `StreamLink`
- existing subtitle structures
- existing player abstraction
- existing interactive selectors
- existing error handling

Only redesign an abstraction when the current one cannot represent the required behavior.

## 3. Language before quality

A user-selected language variant is a hard preference.

Never choose 1080p Spanish from the wrong requested variant over 720p Spanish from the requested variant.

## 4. Generic language means user choice

`es` means Spanish, not "pick a hidden default dialect".

If both Latin American and Spain Spanish are genuinely available, expose the choice.

## 5. No silent language fallback

Do not silently turn:

```text
--language es-419
```

into:

```text
es-ES
```

If the requested language is unavailable, report it and offer an explicit fallback.

## 6. Provider isolation

Providers are unreliable external dependencies.

One provider failing must not make the entire application fail when another provider can satisfy the request.

## 7. Normalize at the boundary

Provider-specific HTML, JSON, server names, and quirks should be converted into project-native structures at the provider boundary.

Do not spread provider-specific logic throughout the application.

## 8. Prefer deterministic selection

Given the same available streams and preferences, selection should be predictable.

Avoid opaque scoring systems when ordered rules can express the policy clearly.

## 9. No unnecessary dependencies

Use the project's existing Rust stack whenever possible.

A provider should not require an external runtime merely because another scraper happens to use one.

## 10. Graceful degradation

The feature should degrade in this order:

```text
Best requested result
    ↓
Another valid provider
    ↓
Explicit alternative language
    ↓
Clean exit
```

Never crash because one external source changed.

## 11. Do not bypass access controls

The project must not implement DRM circumvention, credential theft, paywall bypassing, or authentication circumvention.

Provider integrations should operate within technically and legally appropriate access paths.

## 12. Reproducibility

Provider behavior changes frequently.

Keep provider-specific assumptions documented and tests focused on stable contracts.

## 13. Small commits

Each provider and architectural change should be independently reviewable.

Preferred progression:

```text
provider skeleton
→ search
→ episodes
→ streams
→ language normalization
→ tests
→ integration
```

## 14. Upstream compatibility

Whenever possible, design changes so they could eventually be proposed upstream to `ani-cli-rs`.

Avoid naming, abstractions, or architecture that only make sense for this fork.

## 15. User experience first

Normal usage should remain short:

```bash
ani-cli-rs --language es "Black Torch"
```

Advanced users may control provider/language/quality explicitly, but ordinary users should not need to understand provider internals.
