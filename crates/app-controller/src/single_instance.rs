//! Prevent accidental duplicate recorder instances (APP-005).
//!
//! Windows: a named OS mutex held for the lifetime of the guard. Other
//! platforms: an exclusive lock file (portability fallback; the product
//! targets Windows first per spec §1).

use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("another Silk instance appears to be running")]
pub struct SingleInstanceError;

pub struct SingleInstanceGuard {
    #[cfg(windows)]
    handle: usize,
    #[cfg(not(windows))]
    lock_path: std::path::PathBuf,
}

// The OS mutex handle is portable across threads; holding it in a guard
// moved between threads remains safe.
#[cfg(windows)]
unsafe impl Send for SingleInstanceGuard {}

impl SingleInstanceGuard {
    /// Acquire the named instance slot or fail with [`SingleInstanceError`].
    pub fn try_acquire(name: &str) -> Result<Self, SingleInstanceError> {
        #[cfg(windows)]
        {
            use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
            use windows_sys::Win32::System::Threading::CreateMutexW;

            let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
            let handle = unsafe { CreateMutexW(std::ptr::null(), 0, wide.as_ptr()) };
            if handle.is_null() {
                return Err(SingleInstanceError);
            }
            if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                unsafe { CloseHandle(handle) };
                return Err(SingleInstanceError);
            }
            Ok(Self {
                handle: handle as usize,
            })
        }

        #[cfg(not(windows))]
        {
            let lock_path = std::env::temp_dir().join(format!("{name}.lock"));
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
            {
                Ok(_) => Ok(Self { lock_path }),
                Err(_) => Err(SingleInstanceError),
            }
        }
    }

    /// Force-release (crash-recovery helper for future segments).
    pub fn release(self) {}
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            use windows_sys::Win32::Foundation::CloseHandle;
            if self.handle != 0 {
                unsafe { CloseHandle(self.handle as _) };
                self.handle = 0;
            }
        }
        #[cfg(not(windows))]
        {
            let _ = std::fs::remove_file(&self.lock_path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_name(tag: &str) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        format!("silk-test-{tag}-{}-{nanos}", std::process::id())
    }

    #[test]
    fn second_acquisition_fails_until_released() {
        let name = unique_name("dup");
        let guard = SingleInstanceGuard::try_acquire(&name).expect("first acquire");
        assert!(matches!(
            SingleInstanceGuard::try_acquire(&name),
            Err(SingleInstanceError)
        ));
        drop(guard);
        let again = SingleInstanceGuard::try_acquire(&name);
        assert!(again.is_ok(), "slot must be free after release");
    }

    #[test]
    fn distinct_names_do_not_conflict() {
        let a = SingleInstanceGuard::try_acquire(&unique_name("a")).expect("a");
        let b = SingleInstanceGuard::try_acquire(&unique_name("b")).expect("b");
        drop(a);
        drop(b);
    }
}
