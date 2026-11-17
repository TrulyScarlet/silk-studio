//! Normal OS DLL loading ONLY into an exact child launched in passive fixture mode.
//! No API accepts arbitrary PIDs, games, elevation, or executable payloads.
use crate::cooperative_peer::FixturePeer;
use crate::shared_transport::validate_selected_child;
use crate::transport::{Message, PACKET_SIZE};
use crate::SessionIdentity;

/// Enumerate only windows belonging to the exact passive owned child. The title
/// distinguishes its selected fixture window from its deliberately unrelated one.
pub fn select_owned_window(peer: &FixturePeer) -> Result<SessionIdentity, String> {
    use windows::core::BOOL;
    use windows::Win32::Foundation::{FILETIME, HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextW, GetWindowThreadProcessId,
    };
    struct Search {
        pid: u32,
        found: Option<HWND>,
    }
    unsafe extern "system" fn visit(window: HWND, context: LPARAM) -> BOOL {
        let search = &mut *(context.0 as *mut Search);
        let mut pid = 0;
        GetWindowThreadProcessId(window, Some(&mut pid));
        if pid == search.pid {
            let mut text = [0u16; 128];
            let length = GetWindowTextW(window, &mut text) as usize;
            if String::from_utf16_lossy(&text[..length])
                == "Silk development-only D3D11 hook fixture"
            {
                search.found = Some(window);
                return false.into();
            }
        }
        true.into()
    }
    if peer.passive_target().is_none() {
        return Err("window discovery requires the explicit passive owned child".into());
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
    let window = loop {
        let mut search = Search {
            pid: peer.pid(),
            found: None,
        };
        unsafe {
            let _ = EnumWindows(Some(visit), LPARAM(&mut search as *mut Search as isize));
        }
        if let Some(window) = search.found {
            break window;
        }
        if !peer.is_running() || std::time::Instant::now() > deadline {
            return Err("owned passive window not found".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe {
        windows::Win32::System::Threading::GetProcessTimes(
            peer.handle(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    }
    .map_err(err)?;
    let session = SessionIdentity {
        target: crate::TargetIdentity {
            pid: peer.pid(),
            creation_time: (u64::from(created.dwHighDateTime) << 32)
                | u64::from(created.dwLowDateTime),
            window: window.0 as u64,
        },
        generation: 1,
        epoch: 1,
    };
    unsafe { validate_selected_child(peer.handle(), peer.pid(), session) }?;
    Ok(session)
}
use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};
use windows::core::{s, w, PCWSTR};
use windows::Win32::Foundation::{HANDLE, HMODULE, WAIT_OBJECT_0};
use windows::Win32::System::Diagnostics::Debug::WriteProcessMemory;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Module32FirstW, Module32NextW, MODULEENTRY32W, TH32CS_SNAPMODULE,
    TH32CS_SNAPMODULE32,
};
use windows::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetModuleHandleExW, GetModuleHandleW, GetProcAddress, LoadLibraryExW,
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32,
};
use windows::Win32::System::Memory::{
    VirtualAllocEx, VirtualFreeEx, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE,
};
use windows::Win32::System::SystemInformation::{
    IMAGE_FILE_MACHINE, IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_UNKNOWN,
};
use windows::Win32::System::Threading::{
    CreateRemoteThread, GetExitCodeThread, IsWow64Process2, WaitForSingleObject,
};

fn err(error: windows::core::Error) -> String {
    error.to_string()
}
// Private sum type: no arbitrary PID/handle constructor is exposed.
enum AttachmentPeer<'a> {
    D3d11(&'a FixturePeer),
    #[cfg(feature = "d3d12-remote-helper-fixture")]
    D3d12(&'a crate::d3d12_cross_process::Peer),
}
impl AttachmentPeer<'_> {
    fn pid(&self) -> u32 {
        match self {
            Self::D3d11(peer) => peer.pid(),
            #[cfg(feature = "d3d12-remote-helper-fixture")]
            Self::D3d12(peer) => peer.pid(),
        }
    }
    fn handle(&self) -> HANDLE {
        match self {
            Self::D3d11(peer) => peer.handle(),
            #[cfg(feature = "d3d12-remote-helper-fixture")]
            Self::D3d12(peer) => peer.handle(),
        }
    }
    fn owned_process_handle(&self) -> Result<OwnedHandle, String> {
        match self {
            Self::D3d11(peer) => peer.owned_process_handle(),
            #[cfg(feature = "d3d12-remote-helper-fixture")]
            Self::D3d12(peer) => peer.process_handle(),
        }
    }
    fn passive_target(&self) -> Option<&Path> {
        match self {
            Self::D3d11(peer) => peer.passive_target(),
            #[cfg(feature = "d3d12-remote-helper-fixture")]
            Self::D3d12(peer) => peer.passive_target(),
        }
    }
    fn is_running(&self) -> bool {
        match self {
            Self::D3d11(peer) => peer.is_running(),
            #[cfg(feature = "d3d12-remote-helper-fixture")]
            Self::D3d12(peer) => peer.is_running(),
        }
    }
    fn reserve_attachment(&self) -> Result<(), String> {
        match self {
            Self::D3d11(peer) => peer.reserve_attachment(),
            #[cfg(feature = "d3d12-remote-helper-fixture")]
            Self::D3d12(peer) => peer.reserve_attachment(),
        }
    }
}
struct Module {
    base: usize,
    size: usize,
    path: PathBuf,
}
fn modules(pid: u32) -> Result<Vec<Module>, String> {
    let mut retries = 0;
    let handle = loop {
        match unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid) } {
            Ok(handle) => break handle,
            Err(error)
                if error.code() == windows::Win32::Foundation::ERROR_BAD_LENGTH.to_hresult()
                    && retries < 32 =>
            {
                retries += 1;
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(error) => return Err(format!("owned module snapshot failed: {error}")),
        }
    };
    let _owned = unsafe { OwnedHandle::from_raw_handle(handle.0) };
    let mut entry = MODULEENTRY32W {
        dwSize: std::mem::size_of::<MODULEENTRY32W>() as u32,
        ..Default::default()
    };
    unsafe { Module32FirstW(handle, &mut entry) }.map_err(err)?;
    let mut result = Vec::new();
    loop {
        if result.len() >= 512 {
            return Err("owned module snapshot exceeds bound".into());
        }
        let length = entry
            .szExePath
            .iter()
            .position(|value| *value == 0)
            .ok_or("module path truncated")?;
        result.push(Module {
            base: entry.modBaseAddr as usize,
            size: entry.modBaseSize as usize,
            path: PathBuf::from(String::from_utf16_lossy(&entry.szExePath[..length])),
        });
        if let Err(error) = unsafe { Module32NextW(handle, &mut entry) } {
            if error.code() != windows::Win32::Foundation::ERROR_NO_MORE_FILES.to_hresult() {
                return Err(err(error));
            }
            break;
        }
    }
    Ok(result)
}
fn same_path(a: &Path, b: &Path) -> bool {
    a.canonicalize()
        .ok()
        .zip(b.canonicalize().ok())
        .is_some_and(|(a, b)| a == b)
}
fn remote_export(
    peer: &AttachmentPeer<'_>,
    module: HMODULE,
    address: usize,
) -> Result<usize, String> {
    let mut path = [0u16; 32768];
    let length = unsafe { GetModuleFileNameW(Some(module), &mut path) } as usize;
    if length == 0 || length >= path.len() {
        return Err("system/helper module path unavailable".into());
    }
    let path = PathBuf::from(String::from_utf16_lossy(&path[..length]));
    let remote = modules(peer.pid())?
        .into_iter()
        .find(|entry| same_path(&entry.path, &path))
        .ok_or("expected module absent in owned target")?;
    relocate(module.0 as usize, address, remote.base, remote.size)
}

fn relocate(local: usize, address: usize, remote: usize, size: usize) -> Result<usize, String> {
    let offset = address
        .checked_sub(local)
        .ok_or("export precedes its module")?;
    if offset >= size {
        return Err("export offset outside the verified remote module".into());
    }
    remote
        .checked_add(offset)
        .ok_or("remote export address overflow".into())
}

struct Allocation {
    process: OwnedHandle,
    pointer: *mut c_void,
    reclaim: bool,
}
impl Allocation {
    fn write(peer: &AttachmentPeer<'_>, bytes: &[u8]) -> Result<Self, String> {
        if bytes.is_empty() || bytes.len() > 65536 {
            return Err("remote data allocation exceeds bound".into());
        }
        let process = peer.owned_process_handle()?;
        let handle = HANDLE(process.as_raw_handle());
        let pointer = unsafe {
            VirtualAllocEx(
                handle,
                None,
                bytes.len(),
                MEM_COMMIT | MEM_RESERVE,
                PAGE_READWRITE,
            )
        };
        if pointer.is_null() {
            return Err(err(windows::core::Error::from_thread()));
        }
        let allocation = Self {
            process,
            pointer,
            reclaim: true,
        };
        let mut written = 0;
        unsafe {
            WriteProcessMemory(
                handle,
                pointer,
                bytes.as_ptr().cast(),
                bytes.len(),
                Some(&mut written),
            )
        }
        .map_err(err)?;
        if written != bytes.len() {
            return Err("short remote data write".into());
        }
        Ok(allocation)
    }
}
impl Drop for Allocation {
    fn drop(&mut self) {
        if self.reclaim {
            unsafe {
                let _ = VirtualFreeEx(
                    HANDLE(self.process.as_raw_handle()),
                    self.pointer,
                    0,
                    MEM_RELEASE,
                );
            }
        }
    }
}
fn start(
    peer: &AttachmentPeer<'_>,
    address: usize,
    parameter: *mut c_void,
) -> Result<OwnedHandle, String> {
    let entry = unsafe {
        std::mem::transmute::<usize, unsafe extern "system" fn(*mut c_void) -> u32>(address)
    };
    let thread = unsafe {
        CreateRemoteThread(
            peer.handle(),
            None,
            0,
            Some(entry),
            Some(parameter),
            0,
            None,
        )
    }
    .map_err(err)?;
    Ok(unsafe { OwnedHandle::from_raw_handle(thread.0) })
}

pub struct RemoteWorker {
    thread: OwnedHandle,
    parameter: Allocation,
}
impl RemoteWorker {
    pub fn finish(&mut self) -> Result<(), String> {
        if unsafe { WaitForSingleObject(HANDLE(self.thread.as_raw_handle()), 5000) }
            != WAIT_OBJECT_0
        {
            return Err("resident worker did not stop by deadline; keep helper loaded".into());
        }
        self.parameter.reclaim = true;
        let mut status = 0;
        unsafe { GetExitCodeThread(HANDLE(self.thread.as_raw_handle()), &mut status) }
            .map_err(err)?;
        if status != 0 {
            return Err(format!("resident worker failed: status={status}"));
        }
        Ok(())
    }
}

pub fn attach_owned_fixture(
    peer: &FixturePeer,
    session: SessionIdentity,
) -> Result<RemoteWorker, String> {
    attach(&AttachmentPeer::D3d11(peer), session)
}
#[cfg(feature = "d3d12-remote-helper-fixture")]
pub fn attach_owned_d3d12_fixture(
    peer: &crate::d3d12_cross_process::Peer,
    session: SessionIdentity,
) -> Result<RemoteWorker, String> {
    attach(&AttachmentPeer::D3d12(peer), session)
}
fn attach(peer: &AttachmentPeer<'_>, session: SessionIdentity) -> Result<RemoteWorker, String> {
    let executable = peer
        .passive_target()
        .ok_or("attachment requires a child explicitly launched in passive fixture mode")?;
    if !peer.is_running() {
        return Err("owned target already exited".into());
    }
    unsafe { validate_selected_child(peer.handle(), peer.pid(), session) }?;
    let mut process_machine = IMAGE_FILE_MACHINE::default();
    let mut native = IMAGE_FILE_MACHINE::default();
    unsafe { IsWow64Process2(peer.handle(), &mut process_machine, Some(&mut native)) }
        .map_err(err)?;
    if process_machine != IMAGE_FILE_MACHINE_UNKNOWN || native != IMAGE_FILE_MACHINE_AMD64 {
        return Err("only native AMD64 owned targets are supported".into());
    }
    let directory = executable
        .parent()
        .ok_or("owned fixture directory unavailable")?;
    let helper = directory
        .join("silk_capture_hook_helper.dll")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if helper.parent() != Some(directory) {
        return Err("helper must be a canonical fixture-directory sibling".into());
    }
    // Verify the retained target's image against the launched canonical file.
    if !modules(peer.pid())?
        .iter()
        .any(|module| same_path(&module.path, executable))
    {
        return Err("launched fixture image changed".into());
    }
    let kernel = unsafe { GetModuleHandleW(w!("kernel32.dll")) }.map_err(err)?;
    let load = unsafe { GetProcAddress(kernel, s!("LoadLibraryW")) }
        .ok_or("LoadLibraryW unavailable")? as usize;
    let mut owner = HMODULE::default();
    unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(load as *const u16),
            &mut owner,
        )
    }
    .map_err(err)?;
    let remote_load = remote_export(peer, owner, load)?;
    peer.reserve_attachment()?;
    let wide: Vec<u16> = helper.as_os_str().encode_wide().chain(Some(0)).collect();
    let bytes: Vec<u8> = wide.iter().flat_map(|value| value.to_le_bytes()).collect();
    let mut path = Allocation::write(peer, &bytes)?;
    unsafe { validate_selected_child(peer.handle(), peer.pid(), session) }?;
    let loader = start(peer, remote_load, path.pointer)?;
    path.reclaim = false;
    if unsafe { WaitForSingleObject(HANDLE(loader.as_raw_handle()), 10000) } != WAIT_OBJECT_0 {
        return Err("owned DLL load timed out; retain remote data until target exit".into());
    }
    path.reclaim = true;
    let wide: Vec<u16> = helper.as_os_str().encode_wide().chain(Some(0)).collect();
    // Keep both local and remote module references resident. No FreeLibrary.
    let local = unsafe {
        LoadLibraryExW(
            PCWSTR(wide.as_ptr()),
            None,
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    }
    .map_err(err)?;
    let name = match peer {
        AttachmentPeer::D3d11(_) => s!("silk_capture_resident_worker_v1"),
        #[cfg(feature = "d3d12-remote-helper-fixture")]
        AttachmentPeer::D3d12(_) => s!("silk_capture_d3d12_worker_v1"),
    };
    let entry = unsafe { GetProcAddress(local, name) }
        .ok_or("build helper with the selected resident-helper/d3d12-helper feature")?
        as usize;
    let remote_entry = remote_export(peer, local, entry)?;
    let mut parameter = Allocation::write(peer, &Message::Select(session).encode())?;
    debug_assert_eq!(PACKET_SIZE, 128);
    unsafe { validate_selected_child(peer.handle(), peer.pid(), session) }?;
    let thread = start(peer, remote_entry, parameter.pointer)?;
    parameter.reclaim = false;
    Ok(RemoteWorker { thread, parameter })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn export_relocation_rejects_out_of_image_and_overflow() {
        assert_eq!(relocate(100, 112, 1000, 64).unwrap(), 1012);
        for (local, address, remote, size) in [
            (100, 99, 1000, 64),
            (100, 164, 1000, 64),
            (100, 101, usize::MAX, 64),
            (100, 100, 1000, 0),
        ] {
            assert!(relocate(local, address, remote, size).is_err());
        }
    }
}
