# Handy Latest-Upstream Custom Transcription Design

## Goal

Produce a maintained fork branch that starts from the latest `cjpais/Handy` upstream and preserves the mature custom transcription functionality currently represented by `origin/integration/upstream-0.9.4`.

## Source of truth

- Upstream base: `upstream/main` at the start of this migration (`fbd4e15`, Handy 0.9.6-era main).
- Mature custom implementation: `origin/integration/upstream-0.9.4` (`71f719e`).
- Historical merge base of those lines: `099df58a591bb8ab07ee16dc8e4e8d69d1270791`.
- The older `origin/main` is not the custom-feature source of truth; it contains only the installer commit on top of the old upstream.

## Functional requirements

1. Preserve the Codex remote transcription engine.
   - Authenticate from local Codex CLI credentials (`~/.codex/auth.json` / `%USERPROFILE%/.codex/auth.json`).
   - Require `tokens.access_token`; use `tokens.account_id` when present.
   - Send audio to the ChatGPT transcription backend using the Codex-compatible request identity.
   - Never persist or log the access token.

2. Preserve token-based hosted transcription through Groq.
   - Expose the supported hosted Whisper models as remote models.
   - Store a user-supplied API key per remote model using the existing application settings mechanism.
   - Treat remote models as no-download models in onboarding/model selection.
   - Never log or commit API keys.

3. Preserve shared remote-transcription hardening from the mature custom branch.
   - Bounded requests/timeouts.
   - Cancellation support integrated with the current transcription lifecycle.
   - Temporary WAV/request artifacts are cleaned up.
   - Language values are normalized before being sent to providers.

4. Preserve the ancillary custom behavior that shipped with those providers unless current upstream supersedes it with a better equivalent.
   - Recovery of recordings/transcriptions missing from history.
   - Retry history with the currently selected model.
   - Preservation of cancelled recordings for retry.
   - Hotkey/action to paste the last transcription.

5. Preserve all newer upstream behavior, especially shortcut, recording-state, paste-reliability, model-management, and current Tauri/frontend changes. Conflict resolution must prefer current upstream structure while re-expressing the custom behavior in that structure.

6. Keep generated bindings synchronized with Rust command/type definitions.

## Quality and security constraints

- No real Codex/Groq credentials in tests, logs, fixtures, commits, or generated artifacts.
- Unit tests may exercise auth parsing, language normalization, request construction, and settings behavior without making live network calls.
- Any live-provider diagnostic remains ignored/opt-in.
- Avoid shelling out when a current Rust library/API already provides a clearly safer equivalent; however, preserving the existing proven implementation is acceptable during the migration if replacing it would broaden scope or risk behavior.
- Do not regress latest-upstream paste or hotkey fixes.
- Keep changes narrowly scoped to the custom functionality; do not carry obsolete CI/retrigger-only commits from the historical port branches.

## Migration strategy

Start an isolated branch from the newest `upstream/main`. Merge the mature custom integration line so Git can use the real common ancestor, then resolve conflicts semantically. The resulting source tree, not preservation of every historical port/CI commit, is the deliverable. After the merge, review the custom areas against current upstream and simplify obsolete compatibility code when the newer architecture provides a direct replacement.

## Validation

- `bun install --frozen-lockfile`
- `bun run build`
- `bun run lint`
- `bun run format:check`
- Rust formatting/tests/checks when a Rust toolchain is available (`cargo fmt -- --check`, targeted unit tests, then `cargo test --lib` / `cargo check`).
- Inspect Git diff to verify only intended custom behavior plus migration documentation is present.
- Do not run ignored live Codex/Groq tests automatically.
