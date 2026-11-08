//! Windows Shell data objects provide real file drops (including CF_HDROP).
//! Run on the window's STA thread. Only COPY is offered, so dragging never moves a clip.
use std::path::Path;

#[cfg(windows)]
unsafe fn file_data(
    path: &Path,
) -> windows::core::Result<windows::Win32::System::Com::IDataObject> {
    use windows::{
        core::HSTRING,
        Win32::UI::Shell::{BHID_DataObject, IShellItem, SHCreateItemFromParsingName},
    };
    let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None)?;
    item.BindToHandler(None, &BHID_DataObject)
}

#[cfg(windows)]
pub fn drag_file(path: &Path) -> Result<(), String> {
    use windows::Win32::{
        System::Ole::{IDropSource, OleInitialize, OleUninitialize, DROPEFFECT_COPY},
        UI::Shell::SHDoDragDrop,
    };
    unsafe {
        OleInitialize(None).map_err(|e| e.to_string())?;
        struct OleGuard;
        impl Drop for OleGuard {
            fn drop(&mut self) {
                unsafe {
                    OleUninitialize();
                }
            }
        }
        let _guard = OleGuard;
        let data = file_data(path).map_err(|e| e.to_string())?;
        SHDoDragDrop(None, &data, None::<&IDropSource>, DROPEFFECT_COPY)
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use windows::Win32::{
        System::{
            Com::{DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL},
            Ole::{OleInitialize, OleUninitialize, ReleaseStgMedium, CF_HDROP},
        },
        UI::Shell::{DragQueryFileW, HDROP},
    };

    #[test]
    fn shell_data_object_contains_the_real_file_drop_path() {
        let folder = test_support::TempDir::new("native-file-drop").unwrap();
        let path = folder.path().join("clip with spaces.mp4");
        std::fs::write(&path, b"local file drop test").unwrap();
        unsafe {
            OleInitialize(None).unwrap();
            let data = file_data(&path).unwrap();
            let format = FORMATETC {
                cfFormat: CF_HDROP.0,
                dwAspect: DVASPECT_CONTENT.0,
                lindex: -1,
                tymed: TYMED_HGLOBAL.0 as u32,
                ..Default::default()
            };
            let mut medium = data.GetData(&format).unwrap();
            let drop = HDROP(medium.u.hGlobal.0);
            let count = DragQueryFileW(drop, u32::MAX, None);
            let mut name = [0u16; 32768];
            let length = DragQueryFileW(drop, 0, Some(&mut name));
            let actual = String::from_utf16(&name[..length as usize]).unwrap();
            ReleaseStgMedium(&mut medium);
            std::mem::drop(data);
            OleUninitialize();
            assert_eq!(count, 1);
            assert_eq!(Path::new(&actual), path);
        }
    }
}

#[cfg(not(windows))]
pub fn drag_file(_path: &Path) -> Result<(), String> {
    Err("File drag is currently available on Windows.".into())
}
