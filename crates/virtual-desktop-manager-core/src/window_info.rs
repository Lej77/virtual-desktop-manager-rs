//! Helper methods to get window information.

use std::{collections::HashMap, fmt, ops::ControlFlow, sync::Arc};
use windows::{
    core::{Error, PWSTR},
    Win32::{
        Foundation::{CloseHandle, HANDLE, HWND},
        System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT,
            PROCESS_QUERY_LIMITED_INFORMATION,
        },
        UI::WindowsAndMessaging::{GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId},
    },
};

use crate::{nwg_ext::enum_child_windows, vd};

/// Simple wrapper around [`enum_child_windows`].
pub fn all_windows() -> Vec<HWND> {
    let mut result = Vec::new();
    enum_child_windows(None, |window| {
        result.push(window);
        ControlFlow::Continue(())
    });
    result
}

/// Get the title of a window.
///
/// # References
///
/// - Rust library for getting titles of all open windows:
///   <https://github.com/HiruNya/window_titles/blob/924feffac93c9ac7238d6fa5c39c1453815a0408/src/winapi.rs>
pub fn get_window_title(window: HWND) -> Result<String, Error> {
    let mut length = unsafe { GetWindowTextLengthW(window) };
    if length == 0 {
        return Ok(String::new());
    }
    length += 1;
    let mut title: Vec<u16> = vec![0; length as usize];
    let len = unsafe { GetWindowTextW(window, &mut title) };
    if len != 0 {
        Ok(String::from_utf16(title[0..(len as usize)].as_ref())?)
    } else {
        Err(Error::from_win32())
    }
}

/// Get the identifier of the process that created a specified window.
///
/// # References
///
/// - [GetWindowThreadProcessId function (winuser.h) - Win32 apps | Microsoft
///   Learn](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getwindowthreadprocessid)
pub fn get_window_process_id(window: HWND) -> Result<u32, Error> {
    let mut process_id = 0;
    let thread_id = unsafe { GetWindowThreadProcessId(window, Some(&mut process_id)) };
    if thread_id == 0 {
        Err(Error::from_win32())
    } else {
        Ok(process_id)
    }
}

/// Get the full name of a process.
///
/// # References
///
/// - [OpenProcess function (processthreadsapi.h) - Win32 apps | Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-openprocess)
/// - [QueryFullProcessImageNameW function (winbase.h) - Win32 apps | Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-queryfullprocessimagenamew)
/// - [windows - How to get the process name in C++ - Stack Overflow](https://stackoverflow.com/questions/4570174/how-to-get-the-process-name-in-c)
pub fn get_process_full_name(process_id: u32) -> Result<String, Error> {
    struct ProcessHandle(HANDLE);
    impl ProcessHandle {
        fn close(self) -> Result<(), Error> {
            let handle = self.0;
            std::mem::forget(self);
            unsafe { CloseHandle(handle) }
        }
    }
    impl Drop for ProcessHandle {
        fn drop(&mut self) {
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
    let handle = ProcessHandle(unsafe {
        // Note: required permission is specified in QueryFullProcessImageNameW docs
        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id)?
    });
    let mut buffer: Vec<u16> = vec![0; 1024];
    let mut length = buffer.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(
            handle.0,
            PROCESS_NAME_FORMAT(0),
            PWSTR::from_raw(buffer.as_mut_ptr()),
            &mut length,
        )?;
    }
    handle.close()?;
    Ok(String::from_utf16(&buffer[..length as usize])?)
}

/// Get the name of a process.
#[allow(clippy::assigning_clones)]
pub fn get_process_name(process_id: u32) -> Result<String, Error> {
    let mut exe_path = get_process_full_name(process_id)?;
    if let Some(slash) = exe_path.rfind(['\\', '/']) {
        exe_path = exe_path[slash + 1..].to_owned();
    }
    if exe_path.ends_with(".exe") {
        exe_path.truncate(exe_path.len() - 4);
    }
    Ok(exe_path)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtualDesktopInfo {
    WindowPinned,
    AppPinned,
    AtDesktop {
        /// GUID identifier for the virtual desktop.
        desktop: vd::Desktop,
        // Zero-based index for the virtual desktop when the info was gathered
        // (it might have been moved after that).
        index: u32,
    },
}
impl fmt::Display for VirtualDesktopInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WindowPinned => write!(f, "Pinned Window"),
            Self::AppPinned => write!(f, "Pinned App"),
            Self::AtDesktop { index, .. } => fmt::Display::fmt(&(index + 1), f),
        }
    }
}
impl VirtualDesktopInfo {
    pub fn new(window: HWND) -> vd::Result<Self> {
        if vd::is_pinned_app(window)? {
            Ok(Self::AppPinned)
        } else if vd::is_pinned_window(window)? {
            Ok(Self::WindowPinned)
        } else {
            let desktop = vd::get_window_desktop(window)?;
            let index = desktop.get_index()?;
            Ok(Self::AtDesktop { desktop, index })
        }
    }

    /// Returns `true` if the virtual desktop info is [`WindowPinned`].
    ///
    /// [`WindowPinned`]: VirtualDesktopInfo::WindowPinned
    #[must_use]
    pub fn is_window_pinned(&self) -> bool {
        matches!(self, Self::WindowPinned)
    }

    /// Returns `true` if the virtual desktop info is [`AppPinned`].
    ///
    /// [`AppPinned`]: VirtualDesktopInfo::AppPinned
    #[must_use]
    pub fn is_app_pinned(&self) -> bool {
        matches!(self, Self::AppPinned)
    }

    /// Returns `true` if the virtual desktop info is [`AtDesktop`].
    ///
    /// [`AtDesktop`]: VirtualDesktopInfo::AtDesktop
    #[must_use]
    pub fn is_at_desktop(&self) -> bool {
        matches!(self, Self::AtDesktop { .. })
    }
}

#[derive(Debug, Clone)]
pub enum GetAllError {
    Title(Error),
    ProcessId(Error),
    ProcessName(Error),
    VirtualDesktop(vd::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowHandle(pub isize);
impl WindowHandle {
    pub fn as_hwnd(self) -> HWND {
        HWND(self.0 as *mut _)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    pub handle: WindowHandle,
    pub title: String,
    pub process_id: u32,
    pub process_name: Arc<str>,
    pub virtual_desktop: VirtualDesktopInfo,
}
impl WindowInfo {
    pub fn get_all() -> Vec<WindowInfo> {
        Self::try_get_all()
            .filter_map(|res| match res {
                Ok(info) => Some(info),
                Err(e) => {
                    tracing::trace!("Failed to get window info: {:?}", e);
                    None
                }
            })
            .collect()
    }
    pub fn try_get_all() -> impl Iterator<Item = Result<WindowInfo, GetAllError>> {
        let mut process_names: HashMap<u32, Arc<str>> = HashMap::new();
        all_windows()
            .into_iter()
            .map(move |handle| -> Result<WindowInfo, GetAllError> {
                let virtual_desktop =
                    VirtualDesktopInfo::new(handle).map_err(GetAllError::VirtualDesktop)?;
                let title = get_window_title(handle).map_err(GetAllError::Title)?;
                let process_id = get_window_process_id(handle).map_err(GetAllError::ProcessId)?;
                let process_name = if let Some(name) = process_names.get(&process_id) {
                    name.clone()
                } else {
                    let name = Arc::<str>::from(
                        get_process_name(process_id).map_err(GetAllError::ProcessName)?,
                    );
                    process_names.insert(process_id, name.clone());
                    name
                };
                Ok(WindowInfo {
                    handle: WindowHandle(handle.0 as isize),
                    title,
                    process_id,
                    process_name,
                    virtual_desktop,
                })
            })
    }
}
