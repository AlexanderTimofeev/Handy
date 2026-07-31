use crate::managers::history::HistoryManager;
use anyhow::Result;
use log::{debug, warn};
use std::collections::HashSet;
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

static RECOVERY_RUNNING: AtomicBool = AtomicBool::new(false);

struct RecoveryGuard;

impl Drop for RecoveryGuard {
    fn drop(&mut self) {
        RECOVERY_RUNNING.store(false, Ordering::Release);
    }
}

fn is_old_enough(path: &std::path::Path, minimum_age: Duration) -> bool {
    if minimum_age.is_zero() {
        return true;
    }

    path.metadata()
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age >= minimum_age)
}

/// Add WAV files that exist in the recordings directory but have no matching
/// history row. This repairs recordings left behind by a cancelled request,
/// process termination, or an older build that persisted history too late.
///
/// `minimum_age` prevents an in-flight transcription from being mistaken for
/// an orphan when the History screen is opened while it is still processing.
pub async fn recover_orphaned_recordings(
    history_manager: Arc<HistoryManager>,
    minimum_age: Duration,
) -> Result<usize> {
    if RECOVERY_RUNNING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Ok(0);
    }
    let _guard = RecoveryGuard;

    let existing = history_manager.get_history_entries(None, None).await?;
    let known_files: HashSet<String> = existing
        .entries
        .into_iter()
        .map(|entry| entry.file_name)
        .collect();

    let mut recovered = 0usize;
    for directory_entry in fs::read_dir(history_manager.recordings_dir())? {
        let directory_entry = match directory_entry {
            Ok(entry) => entry,
            Err(err) => {
                warn!("Failed to inspect recording directory entry: {}", err);
                continue;
            }
        };
        let path = directory_entry.path();
        let is_wav = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("wav"));
        if !is_wav || !is_old_enough(&path, minimum_age) {
            continue;
        }

        let Some(file_name) = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        if known_files.contains(&file_name) {
            continue;
        }

        match crate::audio_toolkit::read_wav_samples(&path) {
            Ok(samples) if !samples.is_empty() => {}
            Ok(_) => {
                warn!("Skipping empty orphan recording: {}", file_name);
                continue;
            }
            Err(err) => {
                warn!("Skipping unreadable orphan recording {}: {}", file_name, err);
                continue;
            }
        }

        history_manager.save_entry(file_name.clone(), String::new(), false, None, None)?;
        recovered += 1;
        debug!("Recovered orphan recording into History: {}", file_name);
    }

    Ok(recovered)
}
