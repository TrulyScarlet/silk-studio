//! Cooperative DLL ABI. Buffers always contain exactly one transport packet.
pub const VERSION: u32 = 1;
pub const ATTACH: u32 = 1;
pub const POLL: u32 = 2;
pub const ACK: u32 = 3;
pub const STOP: u32 = 4;
pub const OK: u32 = 0;
pub const EMPTY: u32 = 1;
pub const REJECTED: u32 = 10;
pub const PANICKED: u32 = 11;
pub type Dispatch = unsafe extern "system" fn(
    version: u32,
    operation: u32,
    chain: *mut std::ffi::c_void,
    request: *const u8,
    request_len: usize,
    response: *mut u8,
    response_len: usize,
) -> u32;
