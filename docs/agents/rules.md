# Decisions of the product owner

Binding for every agent. Newer lines win over older ones. Dates are 2026.

## Product

- Tokuly is a rebuild of the original Tokuly: carry over **every** original feature; look and implementation
  are free (not a clone). Audience: Japanese high schoolers and Gen Z first, then everyone, engineers too.
  Inspirations: Bump/Zenly (the map is the heart), Instagram, Twitter, Misskey.
- UI copy: Japanese first, casual and friendly (「友達と位置情報共有を楽しもう!」), English second.
- Minors: privacy-protective defaults decided at sign-up and stored, never recomputed from today's date.
  The defaults are the lead's proposal on #138 (11/27, left to the lead).
- Buying coins stays a web screen until store distribution is decided; decide before the first store
  release (Google Play / App Store billing or no purchase entry in store builds) (11/27, left to the lead).

## Engineering

- Pre-release: breaking changes are fine; restructure for readability rather than minimise diffs.
- Style reference: the code, READMEs and docs as they were up to commit `060b8f66` (PR #67). Match their
  structure, density and tone.
- READMEs say what a thing is, how to run it and the binding rules — nothing else. Docs keep one level of
  detail, state each fact once, no history or narration (11/26).
- Web and mobile share platform-neutral logic only (`packages/`); no multi-purpose "universal" components;
  small duplication beats a shared abstraction with platform branches (11/25).
- Never wait for CI: merge after review and local checks; collect CI failures and fix them in batches;
  everything green before a release tag (11/24, 11/26). A newer push cancels older runs, main included; End to end and the iOS
  builds run on main only, iOS end to end nightly — CI must never be the bottleneck (11/27).
- Pull requests that change the UI carry screenshots (phone and desktop, Japanese, test data only) (11/26).
- One outcome per issue, one concern per PR; review fixes go on the same PR (11/26).
- A machine-wide setting lives in machine config and is checked by `tools/agent/machine/env-check.sh`;
  state facts about the environment only after checking them (11/26).

## Tools and money

- The CI runs on GitHub-hosted runners only (public repo: no self-hosted runners); GHCR may be used freely;
  jobs stay isolated per runner (11/26).
- A prepaid Anthropic API key may be used up for the strongest model on reviews-with-fixes, design and
  architecture work; it is never stored in the repository (11/26).
