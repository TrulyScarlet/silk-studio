//! Explicit owned D3D12 worker; never executed in DllMain or loader lock.
use capture_hook_prototype::transport::{Message, PACKET_SIZE};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use windows::core::PCWSTR;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::{
    GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN,
};

static ATTEMPTED: AtomicBool = AtomicBool::new(false);
fn run(parameter: *const c_void) -> Result<(), String> {
    capture_hook_prototype::d3d12_bootstrap::validate_owned_attached_process()?;
    if parameter.is_null() {
        return Err("null D3D12 bootstrap parameter".into());
    }
    if ATTEMPTED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err("one D3D12 helper worker per process".into());
    }
    let mut module = HMODULE::default();
    unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
            PCWSTR(silk_capture_d3d12_worker_v1 as *const u16),
            &mut module,
        )
    }
    .map_err(|e| e.to_string())?;
    // The normal loader allocated this exact fixed-size read/write data packet;
    // it remains allocated until this worker exits (or the owned process exits).
    let mut packet = [0u8; PACKET_SIZE];
    unsafe {
        std::ptr::copy_nonoverlapping(parameter.cast::<u8>(), packet.as_mut_ptr(), PACKET_SIZE);
    }
    let Message::Select(session) =
        Message::decode(&packet).map_err(|e| format!("invalid D3D12 helper parameter: {e:?}"))?
    else {
        return Err("D3D12 helper requires explicit selection".into());
    };
    capture_hook_prototype::d3d12_bootstrap::run_owned_attached_worker(session)
}

#[no_mangle]
pub unsafe extern "system" fn silk_capture_d3d12_worker_v1(parameter: *mut c_void) -> u32 {
    match std::panic::catch_unwind(|| run(parameter)) {
        Ok(Ok(())) => 0,
        Ok(Err(error)) => {
            eprintln!("owned D3D12 helper: {error}");
            1
        }
        Err(_) => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn foreign_test_process_is_rejected_before_parameter_or_module_work() {
        assert!(run(std::ptr::null()).is_err());
        assert!(!ATTEMPTED.load(Ordering::Acquire));
    }
}
