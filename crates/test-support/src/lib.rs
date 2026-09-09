//! Deterministic test doubles and generators for the Silk engine.
//!
//! Everything here is production-safe but intended for tests only:
//! scripted capture sources, injectable-failure encoders, and a muxer
//! that exercises the staging/rename contract without real codecs.
//! Generators are deterministic per construction parameters so failures
//! reproduce exactly (spec §27.3).

pub mod mocks;
pub mod scripted;
pub mod temp_dir;

pub use mocks::{MockAudioEncoder, MockMuxer, MockVideoEncoder};
pub use scripted::{AudioStep, ScriptedAudioCapture, ScriptedVideoCapture, VideoStep};
pub use temp_dir::TempDir;
