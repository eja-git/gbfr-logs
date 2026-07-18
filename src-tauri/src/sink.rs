use engine::sink::{EncounterSink, EngineEvent};
use tauri::{AppHandle, Manager};

/// Forwards parser events to the frontend via Tauri's `emit_all`, broadcasting
/// to every window (matches the superset of what the parser used to do by
/// hand across `main`/`logs` before this was extracted into `engine`).
pub struct TauriSink {
    app: AppHandle,
}

impl TauriSink {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl EncounterSink for TauriSink {
    fn on_event(&self, event: EngineEvent<'_>) {
        match event {
            EngineEvent::EncounterSaved(id) => {
                let _ = self.app.emit_all("encounter-saved", id);
            }
            EngineEvent::EncounterSavedError(message) => {
                let _ = self.app.emit_all("encounter-saved-error", message);
            }
            EngineEvent::EncounterUpdate(state) => {
                let _ = self.app.emit_all("encounter-update", state);
            }
            EngineEvent::EncounterPartyUpdate(player_data) => {
                let _ = self.app.emit_all("encounter-party-update", player_data);
            }
            EngineEvent::OnAreaEnter(state) => {
                let _ = self.app.emit_all("on-area-enter", state);
            }
        }
    }
}
