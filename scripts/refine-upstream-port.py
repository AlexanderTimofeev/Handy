from pathlib import Path


def read(path: str) -> str:
    return Path(path).read_text(encoding="utf-8")


def write(path: str, text: str) -> None:
    Path(path).write_text(text, encoding="utf-8")


# Remove the unrelated Paste Last Transcript action and registration.
path = "src-tauri/src/actions.rs"
text = read(path)
if "// Paste Last Transcript Action" in text:
    start = text.index("// Paste Last Transcript Action")
    end = text.index("// Test Action", start)
    text = text[:start] + text[end:]
action_map = '''    map.insert(
        "paste_last_transcript".to_string(),
        Arc::new(PasteLastTranscriptAction) as Arc<dyn ShortcutAction>,
    );
'''
text = text.replace(action_map, "", 1)
write(path, text)

# Remove the unrelated default binding.
path = "src-tauri/src/settings.rs"
text = read(path)
binding_block = '''    #[cfg(target_os = "macos")]
    let default_paste_last_shortcut = "control+option+space";
    #[cfg(not(target_os = "macos"))]
    let default_paste_last_shortcut = "ctrl+alt+space";

    bindings.insert(
        "paste_last_transcript".to_string(),
        ShortcutBinding {
            id: "paste_last_transcript".to_string(),
            name: "Paste Last Transcription".to_string(),
            description: "Pastes the most recent transcription into the active window."
                .to_string(),
            default_binding: default_paste_last_shortcut.to_string(),
            current_binding: default_paste_last_shortcut.to_string(),
        },
    );

'''
text = text.replace(binding_block, "", 1)
write(path, text)

path = "src/components/settings/general/GeneralSettings.tsx"
text = read(path)
text = text.replace(
    '        <ShortcutInput shortcutId="paste_last_transcript" grouped={true} />\n',
    "",
    1,
)
write(path, text)

# Keep active remote cards reloadable without changing active local-card behavior.
path = "src/components/onboarding/ModelCard.tsx"
text = read(path)
old = '''  const isClickable =
    status === "available" || status === "active" || status === "downloadable";
  // Remote engines (Codex, Groq) have no local file: nothing to download,
  // size, or delete.
  const isRemote =
    model.engine_type === "Codex" || model.engine_type === "Groq";
'''
new = '''  // Remote engines (Codex, Groq) have no local file: nothing to download,
  // size, or delete. An active remote card remains clickable so the user can
  // explicitly reload it after changing credentials; active local models keep
  // the upstream no-op behavior.
  const isRemote =
    model.engine_type === "Codex" || model.engine_type === "Groq";
  const isClickable =
    status === "available" ||
    status === "downloadable" ||
    (status === "active" && isRemote);
'''
text = text.replace(old, new, 1)
text = text.replace(
    "          Saved\n",
    '          {t("common.saved", { defaultValue: "Saved" })}\n',
    1,
)
write(path, text)

# Capture the cancellation generation before preparing the WAV/temp file.
path = "src-tauri/src/managers/remote.rs"
text = read(path)
anchor = "    static COUNTER: AtomicU64 = AtomicU64::new(0);\n\n    let wav = encode_wav(samples)?;"
replacement = "    static COUNTER: AtomicU64 = AtomicU64::new(0);\n\n    let generation = cancellation_generation();\n    let wav = encode_wav(samples)?;"
text = text.replace(anchor, replacement, 1)
later = "    let config = build_curl_config(url, bearer, extra_headers, form_fields, &wav_arg);\n    let generation = cancellation_generation();\n"
text = text.replace(
    later,
    "    let config = build_curl_config(url, bearer, extra_headers, form_fields, &wav_arg);\n",
    1,
)
write(path, text)

# Remove the blocked workflow accidentally added directly to the product PR.
Path(".github/workflows/refine-upstream-port.yml").unlink(missing_ok=True)
