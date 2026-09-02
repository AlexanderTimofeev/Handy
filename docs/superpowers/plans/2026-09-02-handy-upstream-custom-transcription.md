# Handy Upstream Custom Transcription Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebase the maintained Handy fork onto the latest upstream while preserving the fork's Codex/Groq remote transcription, recovery, retry, and paste-last-transcription behavior.

**Architecture:** Keep the newest upstream architecture as the structural authority. Integrate the mature custom line (`origin/integration/upstream-0.9.4`) using its real Git ancestry, resolve conflicts by re-expressing custom behavior in current upstream managers/settings/shortcut code, then review the remote-provider layer for obsolete compatibility code. Existing custom unit tests are carried forward; any newly invented behavior or bug fix must add a failing regression test before production changes.

**Tech Stack:** Tauri 2, Rust, React 18, TypeScript, Bun/Vite, Git.

**Spec:** `docs/superpowers/specs/2026-09-02-upstream-custom-transcription-design.md`

## Global Constraints

- Latest `upstream/main` is authoritative for unrelated behavior and structure.
- Mature custom source is `origin/integration/upstream-0.9.4` (`71f719e`), not the old `origin/main`.
- Preserve Codex auth from local `~/.codex/auth.json` without logging or persisting tokens.
- Preserve Groq per-model API keys without logging or committing secrets.
- Preserve remote cancellation/timeouts, history recovery/retry, cancelled recording retention, and paste-last-transcription behavior unless current upstream already provides a superior equivalent.
- Do not regress current upstream shortcut or paste-reliability changes.
- Do not run ignored live provider tests automatically.
- Historical CI/retrigger-only port commits are not behavior requirements; the resulting source tree is.

---

### Task 1: Integrate the mature custom line into latest upstream

**Files:**
- Merge/resolve: `src-tauri/src/commands/history.rs`
- Merge/resolve: `src-tauri/src/lib.rs`
- Add/resolve: `src-tauri/src/managers/codex.rs`
- Add/resolve: `src-tauri/src/managers/groq.rs`
- Add/resolve: `src-tauri/src/managers/history_recovery.rs`
- Merge/resolve: `src-tauri/src/managers/mod.rs`
- Merge/resolve: `src-tauri/src/managers/model.rs`
- Add/resolve: `src-tauri/src/managers/remote.rs`
- Merge/resolve: `src-tauri/src/managers/transcription.rs`
- Merge/resolve: `src-tauri/src/settings.rs`
- Merge/resolve: `src-tauri/src/shortcut/mod.rs`
- Merge/resolve: `src-tauri/src/utils.rs`
- Merge/resolve: `src/bindings.ts`
- Merge/resolve: `src/components/onboarding/ModelCard.tsx`
- Merge/resolve if still semantically needed: `src/components/ui/Slider.tsx`

**Interfaces:**
- Consumes: current upstream manager/settings/shortcut APIs and the mature custom behavior from `origin/integration/upstream-0.9.4`.
- Produces: a compiling source tree containing `EngineType::Codex`, `EngineType::Groq`, shared remote request plumbing, provider model catalog entries, per-model remote API key settings, recovery manager registration, retry handling, and paste-last-transcription command/hotkey.

- [ ] **Step 1: Record the exact integration inputs**

Run:

```bash
git rev-parse HEAD upstream/main origin/integration/upstream-0.9.4
git merge-base upstream/main origin/integration/upstream-0.9.4
git status --short --branch
```

Expected: HEAD equals the fresh upstream worktree base and the tree contains only this plan/spec documentation before integration.

- [ ] **Step 2: Commit the migration documentation before touching production code**

Run:

```bash
git add docs/superpowers/specs/2026-09-02-upstream-custom-transcription-design.md docs/superpowers/plans/2026-09-02-handy-upstream-custom-transcription.md
git commit -m "docs: plan latest-upstream custom transcription migration"
```

- [ ] **Step 3: Merge the mature custom line without auto-committing**

Run:

```bash
git merge --no-ff --no-commit origin/integration/upstream-0.9.4
```

If conflicts occur, list them with:

```bash
git diff --name-only --diff-filter=U
```

- [ ] **Step 4: Resolve each conflict semantically**

For every conflicted file, retain current upstream code for unrelated behavior and restore the exact custom contracts from the spec. In particular:

```rust
pub enum EngineType {
    // current upstream variants...
    Codex,
    Groq,
}

impl EngineType {
    pub fn is_remote(&self) -> bool {
        matches!(self, EngineType::Codex | EngineType::Groq)
    }
}
```

The final model catalog must contain the remote IDs:

```text
codex-dictation
groq-whisper-large-v3-turbo
groq-whisper-large-v3
```

The final Codex auth parser must require `tokens.access_token`, treat empty/missing `tokens.account_id` as absent, and never include refresh/id tokens in request handling.

- [ ] **Step 5: Preserve the carried provider tests**

The merged tree must still include the Codex parser tests:

```text
parse_auth_reads_access_token_and_account_id
parse_auth_account_id_optional
parse_auth_empty_account_id_treated_as_absent
parse_auth_missing_access_token_errors
parse_auth_missing_tokens_object_errors
parse_auth_invalid_json_errors
```

and the shared remote tests:

```text
normalize_language_auto_and_empty_are_none
normalize_language_chinese_variants_collapse_to_zh
normalize_language_passes_through_other_codes
encode_wav_roundtrips_through_hound
encode_wav_empty_samples_produces_valid_header
split_response_separates_body_and_status
split_response_handles_multiline_body_and_crlf
build_curl_config_includes_auth_headers_form_and_timeouts
retryable_http_statuses_are_classified
transcribe_response_parses_text_field
```

Do not un-ignore `live_codex_request`.

- [ ] **Step 6: Finish the merge commit only after all conflicts are resolved**

Run:

```bash
git diff --check
git status --short
git add -A
git commit -m "feat: port custom transcription providers to latest upstream"
```

Expected: no unmerged paths.

---

### Task 2: Reconcile provider integration with current upstream and harden only where needed

**Files:**
- Review/modify: `src-tauri/src/managers/codex.rs`
- Review/modify: `src-tauri/src/managers/groq.rs`
- Review/modify: `src-tauri/src/managers/remote.rs`
- Review/modify: `src-tauri/src/managers/model.rs`
- Review/modify: `src-tauri/src/managers/transcription.rs`
- Review/modify: `src-tauri/src/settings.rs`
- Review/modify: `src/components/onboarding/ModelCard.tsx`
- Regenerate/modify only through project mechanism if needed: `src/bindings.ts`

**Interfaces:**
- Consumes: merged provider implementation from Task 1 and current upstream transcription lifecycle.
- Produces: current-architecture provider code with no obsolete duplicate path and no secret exposure.

- [ ] **Step 1: Review the final provider data flow**

Trace these paths with semantic/code search:

```text
model selection -> engine load -> remote request -> cancellation -> transcription result -> history/output
model card -> per-model API key command -> settings SecretMap -> Groq engine construction
Codex engine -> auth_path -> parse_auth -> request headers
```

Confirm there is exactly one active implementation path for each provider.

- [ ] **Step 2: Add a regression test first for any newly discovered defect**

If the review reveals a defect not already covered by the carried tests, add one minimal Rust unit test demonstrating it and run the most targeted available test command. The test must fail for the defect before changing production code. Examples of valid regression targets include cancellation classification, secret-redaction, remote model auto-selection, or request language normalization.

- [ ] **Step 3: Apply only test-driven fixes or narrow refactors**

Keep these contracts:

```text
remote model => no local download
Codex => local Codex CLI auth, no persisted Handy token
Groq => per-model Handy SecretMap key
remote requests => bounded + cancellable
secrets => redacted from Debug/log output
```

Do not replace the transport solely for style. Replace old compatibility code only if the current upstream architecture offers a demonstrably simpler/safe equivalent and tests cover the change.

- [ ] **Step 4: Run static/frontend verification for the provider UI**

Run:

```bash
bun run build
bun run lint
```

Expected: both succeed with no new errors.

- [ ] **Step 5: Commit reviewed provider integration**

Run:

```bash
git add src-tauri/src/managers src-tauri/src/settings.rs src/components/onboarding/ModelCard.tsx src/bindings.ts
git diff --cached --check
git commit -m "refactor: align remote transcription with current upstream"
```

If review requires no source changes, skip this commit and record that the merge implementation already satisfies the task.

---

### Task 3: Verify recovery, retry, and shortcut behavior against latest upstream state handling

**Files:**
- Review/modify: `src-tauri/src/managers/history_recovery.rs`
- Review/modify: `src-tauri/src/commands/history.rs`
- Review/modify: `src-tauri/src/lib.rs`
- Review/modify: `src-tauri/src/managers/transcription.rs`
- Review/modify: `src-tauri/src/shortcut/mod.rs`

**Interfaces:**
- Consumes: latest upstream recording/shortcut coordinator plus the custom recovery and paste-last behavior.
- Produces: no duplicated shortcut state machine, recoverable failed/cancelled remote recordings, retry through the currently selected model, and a working paste-last action.

- [ ] **Step 1: Trace current upstream shortcut/transcription state transitions**

Identify the current recording coordinator/state guard used for start/stop/cancel and ensure the custom paste-last command does not mutate recording state.

- [ ] **Step 2: Verify recovery registration and retry model choice**

Confirm app initialization registers the history recovery manager and that retry resolves the model from current settings rather than from a stale historical model identifier.

- [ ] **Step 3: Add a failing regression test before changing behavior**

If current-upstream integration reveals a conflict (for example duplicate busy-state handling, stale selected model, or cancelled-recording loss), add the smallest unit test that fails for that integration bug before editing production behavior.

- [ ] **Step 4: Fix only integration defects and retain upstream shortcut fixes**

Do not restore historical shortcut code that current upstream has replaced. Re-express only the paste-last action and recovery hooks using the new coordinator APIs.

- [ ] **Step 5: Commit shortcut/recovery reconciliation if changed**

Run:

```bash
git diff --check
git add src-tauri/src/managers/history_recovery.rs src-tauri/src/commands/history.rs src-tauri/src/lib.rs src-tauri/src/managers/transcription.rs src-tauri/src/shortcut/mod.rs
git commit -m "fix: reconcile recovery and shortcuts with latest upstream"
```

If no source changes are needed, skip the commit.

---

### Task 4: Final validation and branch packaging

**Files:**
- Verify all changed files.
- No new production files unless a validation failure requires a test-driven fix.

**Interfaces:**
- Consumes: Tasks 1-3.
- Produces: a clean, reviewable branch ready to push/merge into the fork.

- [ ] **Step 1: Run frontend/static checks**

Run:

```bash
bun run build
bun run lint
bun run format:check
bun run check:translations
bun run check:model-languages
```

- [ ] **Step 2: Run Rust checks when toolchain is available**

Run from `src-tauri`:

```bash
cargo fmt -- --check
cargo test --lib
cargo check
```

If the local Rust toolchain is unavailable, record that limitation explicitly and use the repository's CI for Rust verification before calling the branch fully verified.

- [ ] **Step 3: Inspect the final delta**

Run:

```bash
git status --short --branch
git log --oneline --decorate upstream/main..HEAD
git diff --stat upstream/main...HEAD
git diff --check upstream/main...HEAD
```

Confirm no credentials, temporary audio files, build outputs, or unrelated historical CI scaffolding are introduced.

- [ ] **Step 4: Perform code review**

Review the complete branch specifically for:

```text
secret leakage
remote request cancellation/timeout regressions
model selector/download behavior
shortcut state regressions
history recovery lifecycle
bindings/frontend-backend type mismatch
```

- [ ] **Step 5: Leave the result on the isolated branch**

Do not rewrite or force-update the fork's `main`. The completed branch is `sync/upstream-2026-09-02`; pushing it to the fork is a separate external side effect and should occur only when authorized or clearly required for CI.
