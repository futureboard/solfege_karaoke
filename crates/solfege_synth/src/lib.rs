//! # solfege_synth
//!
//! The sound engine shared by the Solfege apps: WAV / SFZ / SF2
//! instruments, a realtime rack of instrument slots with mixer and effects,
//! a Standard MIDI File player, and the `cpal` audio output that drives it.
//!
//! The engine runs inside the audio callback. Other threads talk to it
//! through [`engine::Command`]s and read meters and the player position
//! from [`engine::Shared`].

pub mod audio;
pub mod engine;
pub mod instrument;
pub mod smf;
