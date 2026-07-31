use crate::managers::audio::AudioRecordingManager;
use crate::managers::history::HistoryManager;
use crate::managers::transcription::TranscriptionManager;
use crate::shortcut;
use crate::TranscriptionCoordinator;
use log::{error, info};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

// Re-export all utility modules for easy access
// pub use crate::audio_feedback::*;
pub use crate::clipboard::*;
pub use crate::overlay::*;
pub use crate::tray::*;

/// Centralized cancellation function that can be called from anywhere in the app.
/// Handles cancelling both recording and transcription operations and updates UI state.
pub fn cancel_current_operation(app: &AppHandle) {
    info!("Initiating operation cancellation...");

    shortcut::unregister_cancel_shortcut(app);

    // Signal the remote request layer so any active cloud transcription exits.
    crate::managers::remote::cancel_active_requests();

    // Cancel recording and invalidate output from the current pipeline. This
    // prevents a result that finishes after cancellation from being pasted.
    let audio_manager = app.state::<Arc<AudioRecordingManager>>();
    let recording_was_active = audio_manager.is_recording();
    audio_manager.cancel_recording();

    let tm = app.state::<Arc<TranscriptionManager>>();
    tm.cancel_stream();

    change_tray_icon(app, crate::tray::TrayIconState::Idle);
    hide_recording_overlay(app);

    tm.maybe_unload_immediately("cancellation");

    if let Some(coordinator) = app.try_state::<TranscriptionCoordinator>() {
        coordinator.notify_cancel(recording_was_active);
    }

    // A WAV may already have been written even though a cancelled pipeline
    // deliberately skips normal history persistence. Recover it shortly after
    // cancellation so it can be retried with the currently selected model.
    let history_manager = Arc::clone(&app.state::<Arc<HistoryManager>>());
    let app_for_recovery = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(750)).await;
        match crate::managers::history_recovery::recover_orphaned_recordings(
            history_manager,
            Duration::from_secs(0),
        )
        .await
        {
            Ok(recovered) if recovered > 0 => {
                info!(
                    "Recovered {} cancelled recording(s) into transcription history",
                    recovered
                );
                let _ = app_for_recovery.emit(
                    "transcription-error",
                    "Transcription cancelled. The recording was saved in History and can be retried."
                        .to_string(),
                );
            }
            Ok(_) => {}
            Err(err) => error!("Failed to recover cancelled recording into History: {}", err),
        }
    });

    info!("Operation cancellation completed - returned to idle state");
}

/// Check if using the Wayland display server protocol
#[cfg(target_os = "linux")]
pub fn is_wayland() -> bool {
    std::env::var("WAYLAND_DISPLAY").is_ok()
        || std::env::var("XDG_SESSION_TYPE")
            .map(|v| v.to_lowercase() == "wayland")
            .unwrap_or(false)
}

/// Check if running on KDE Plasma desktop environment
#[cfg(target_os = "linux")]
pub fn is_kde_plasma() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .map(|v| v.to_uppercase().contains("KDE"))
        .unwrap_or(false)
        || std::env::var("KDE_SESSION_VERSION").is_ok()
}

/// Check if running on KDE Plasma with Wayland
#[cfg(target_os = "linux")]
pub fn is_kde_wayland() -> bool {
    is_wayland() && is_kde_plasma()
}
