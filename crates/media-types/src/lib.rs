//! Shared media data types for the Silk recorder.
//!
//! This crate is the single home for stream identifiers, time bases,
//! encoded packets, frame types, and snapshot containers. Capture, audio,
//! encoder, buffer, and muxer crates depend on these types; none of them
//! may re-define parallel concepts (spec §13).
//!
//! The interfaces here are original to this project (spec §5.3).

pub mod frame;
pub mod packet;
pub mod snapshot;
pub mod time_base;

pub use frame::{
    AudioDeviceInfo, AudioDeviceKind, AudioFrame, ChromaSubsampling, FramePayload, GpuFrameHandle,
    PixelFormat, SampleFormat, VideoFrame, VideoSourceInfo, WindowSourceInfo,
};
pub use packet::{EncodedPacket, MediaType, PacketPayload, StreamId};
pub use snapshot::{MediaSnapshot, StreamDescriptor, StreamPackets};
pub use time_base::TimeBase;
