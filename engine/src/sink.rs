use crate::v1::{DerivedEncounterState, PlayerData};

/// A borrowed view of a parser event, handed to whatever sink is wired up
/// (Tauri's `emit_all`/`window.emit` for the GUI, a channel for the TUI).
pub enum EngineEvent<'a> {
    EncounterSaved(Option<i64>),
    EncounterSavedError(&'a str),
    EncounterUpdate(&'a DerivedEncounterState),
    EncounterPartyUpdate(&'a [Option<PlayerData>; 4]),
    OnAreaEnter(&'a DerivedEncounterState),
}

/// Receives live updates from the `Parser` as it processes events, so the
/// parsing/aggregation logic doesn't need to know how (or whether) a frontend
/// is attached.
pub trait EncounterSink: Send + Sync {
    fn on_event(&self, event: EngineEvent<'_>);
}
