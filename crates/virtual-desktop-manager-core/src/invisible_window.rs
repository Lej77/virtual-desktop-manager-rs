//! This module can create an invisible window and focus it to allow for
//! animation when switching virtual desktop.

use std::{any::TypeId, cell::Cell, fmt, ptr::null_mut, rc::Rc, sync::OnceLock, time::Duration};

use nwd::{NwgPartial, NwgUi};
use nwg::{NativeUi, PartialUi};
use windows::Win32::{Foundation::HWND, UI::WindowsAndMessaging::SetForegroundWindow};

use crate::{
    dynamic_gui::DynamicUiHooks,
    nwg_ext::{to_utf16, FastTimerControl, LazyUi, ParentCapture},
    tray::{SystemTray, TrayPlugin, TrayRoot},
    vd,
};

#[derive(Default, NwgPartial, NwgUi)]
pub struct InvisibleWindow {
    pub parent: Option<nwg::ControlHandle>,

    pub ex_flags: u32,

    #[nwg_control(
        parent: data.parent,
        flags: "VISIBLE | POPUP",
        ex_flags: data.ex_flags,
        size: (0, 0),
        title: "",
    )]
    pub window: nwg::Window,
}
impl InvisibleWindow {
    pub fn get_handle(&self) -> HWND {
        HWND(
            self.window
                .handle
                .hwnd()
                .expect("Tried to use an invisible window that was't created yet")
                .cast(),
        )
    }
    pub fn set_foreground(&self) {
        let Some(handle) = self.window.handle.hwnd() else {
            return;
        };
        unsafe {
            let _ = SetForegroundWindow(HWND(handle.cast()));
        }
    }
}

impl crate::nwg_ext::LazyUiHooks for InvisibleWindow {
    fn set_parent(&mut self, parent: Option<nwg::ControlHandle>) {
        self.parent = parent;
    }
}

#[derive(nwd::NwgPartial, Default)]
pub struct SmoothDesktopSwitcher {
    /// Captures the parent that this partial UI is instantiated with.
    #[nwg_control]
    capture: ParentCapture,

    /// Using this kind of window as parent to the invisible window works best
    /// when we want to switch virtual desktop.
    #[nwg_control]
    parent: nwg::MessageWindow,

    /// Note: don't set parent here since that will prevent the window from
    /// showing up in the task bar.
    #[nwg_partial(parent: parent)]
    pub invisible_window: LazyUi<InvisibleWindow>,

    /// `true` if `invisible_window` is created and hasn't been closed yet.
    active: Cell<bool>,

    #[nwg_control(parent: capture)]
    #[nwg_events(OnNotice: [Self::on_close_tick])]
    close_timer: FastTimerControl,

    #[nwg_control(parent: capture)]
    #[nwg_events(OnNotice: [Self::on_focus_tick])]
    focus_timer: FastTimerControl,

    #[nwg_control(parent: capture)]
    #[nwg_events(OnNotice: [Self::on_refocus_tick])]
    refocus_timer: FastTimerControl,

    #[nwg_control(parent: capture)]
    #[nwg_events(OnNotice: [Self::on_refocus_finished])]
    refocus_finished: FastTimerControl,

    started_at: core::cell::Cell<Option<std::time::Instant>>,
}
impl fmt::Debug for SmoothDesktopSwitcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SmoothDesktopSwitcher")
            .field("captured_parent", &self.capture.captured_parent)
            .field("active", &self.active.get())
            .field("started_at", &self.started_at)
            .finish()
    }
}
impl DynamicUiHooks<SystemTray> for SmoothDesktopSwitcher {
    fn before_partial_build(
        &mut self,
        tray_ui: &Rc<SystemTray>,
        _should_build: &mut bool,
    ) -> Option<(nwg::ControlHandle, TypeId)> {
        Some((tray_ui.root().window.handle, TypeId::of::<TrayRoot>()))
    }
    fn before_rebuild(&mut self, _dynamic_ui: &Rc<SystemTray>) {
        self.close_window();
        *self = Default::default()
    }
}
impl TrayPlugin for SmoothDesktopSwitcher {}
impl SmoothDesktopSwitcher {
    pub fn close_window(&self) {
        let mut window = self.invisible_window.ui.borrow_mut();
        if !window.window.handle.blank() {
            // Close previous window:
            window.window.close();
            window.window.handle.destroy();
            self.active.set(false);
            self.close_timer.cancel_last();
            self.focus_timer.cancel_last();
            self.refocus_finished.cancel_last();
        }
        self.active.set(false);
    }
    fn create_invisible_window(&self, to_refocus: bool) -> HWND {
        self.close_window();
        let mut window = self.invisible_window.ui.borrow_mut();
        window.ex_flags = if to_refocus {
            // Hide taskbar button (virtual desktop library can't find this window):
            windows::Win32::UI::WindowsAndMessaging::WS_EX_TOOLWINDOW.0
        } else {
            0
        };
        // Create new window:
        let parent = if to_refocus {
            // This seems to work better for re-capturing focus (but it will
            // show a taskbar button for the window):
            None
        } else {
            // Virtual desktop move might fail if we don't use this parent:
            // self.capture.captured_parent
            Some(self.parent.handle)
        };
        window.parent = parent;
        InvisibleWindow::build_partial(&mut window, parent)
            .expect("Failed to build invisible window");
        self.active.set(true);
        window.get_handle()
    }
    /// Open and then quickly close an invisible window to refocus the last
    /// active window. Useful when closing a context menu or a popup.
    #[tracing::instrument]
    pub fn refocus_last_window(&self) {
        self.started_at.set(Some(std::time::Instant::now()));
        self.refocus_timer.notify_after(Duration::from_millis(25));
    }
    #[tracing::instrument]
    pub fn cancel_refocus(&self) {
        self.refocus_timer.cancel_last();
    }
    fn on_refocus_tick(&self) {
        tracing::info!(
            already_active = self.active.get(),
            after = ?self.started_at.get().unwrap().elapsed(),
            "InvisibleWindow::on_refocus_tick()",
        );
        if self.active.get() {
            return;
        }
        self.create_invisible_window(true);
        {
            let guard = self.invisible_window.borrow();
            guard.window.set_visible(true);
            guard.set_foreground();
            guard.window.set_focus();
        }
        // Close after it has gained focus:
        self.on_refocus_finished();
        //self.refocus_finished.notify_after(Duration::from_millis(50));
    }
    fn on_refocus_finished(&self) {
        tracing::info!(
            after = ?self.started_at.get().unwrap().elapsed(),
            "InvisibleWindow::on_refocus_finished()",
        );
        self.close_window();
    }

    pub fn switch_desktop_to(&self, desktop: vd::Desktop) -> vd::Result<()> {
        let window_handle = self.create_invisible_window(false);

        // Move to wanted desktop:
        //
        // IMPORTANT: don't hold the RefCell lock during this call since it can
        // call other window procedures to handle events.
        let res = vd::move_window_to_desktop(desktop, &window_handle).or_else(|_e| {
            // Sometimes winvd doesn't find the created window. (not often, but
            // still better to retry than to give an error message)
            tracing::error!("InvisibleWindow: Failed to find the created window: {_e:?}");
            std::thread::sleep(Duration::from_millis(100));
            vd::move_window_to_desktop(desktop, &window_handle)
        });
        if let Err(e) = res {
            self.close_window();
            return Err(e);
        }

        self.refocus_timer.cancel_last();

        self.started_at.set(Some(std::time::Instant::now()));

        // Force show the window to steal focus after it has been moved:
        // self.focus_timer.notify_after(Duration::from_millis(10));
        self.on_focus_tick(); // Seems like we can do this immediately?

        // Don't close the window immediately since that would cancel the window focus change.
        self.close_timer.notify_after(Duration::from_millis(125));
        Ok(())
    }
    fn on_focus_tick(&self) {
        tracing::info!(
            after = ?self.started_at.get().unwrap().elapsed(),
            "InvisibleWindow::on_focus_tick()",
        );
        let guard = self.invisible_window.borrow();
        guard.window.set_visible(true);
        guard.set_foreground();
        guard.window.set_focus();
    }
    fn on_close_tick(&self) {
        {
            tracing::info!(
                after = ?self.started_at.get().unwrap().elapsed(),
                "InvisibleWindow::on_close_tick()",
            );
            self.close_window();
        }

        // Refocus last window (usually works without this, but this might help):
        // self.refocus_last_window();
    }
}

/// A window that attempts to be as invisible as possible while still allowing
/// focus so that it can be focused in order to move to another virtual desktop.
pub struct CustomInvisibleWindow(windows::Win32::Foundation::HWND);
#[allow(dead_code)]
impl CustomInvisibleWindow {
    const CLASS_NAME_UTF8: &'static str = "CustomInvisibleWindow";

    /// Class name in utf16 with trailing nul byte.
    fn class_name() -> &'static [u16] {
        static CLASS_NAME_UTF16: OnceLock<Vec<u16>> = OnceLock::new();
        CLASS_NAME_UTF16.get_or_init(|| to_utf16(Self::CLASS_NAME_UTF8))
    }
    /// Create a window class.
    ///
    /// Adapted from [`native_windows_gui::win32::window::build_sysclass`].
    fn create_window_class() -> Result<(), windows::core::Error> {
        use windows::{
            core::PCWSTR,
            Win32::{
                Foundation::{
                    GetLastError, ERROR_CLASS_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM, LRESULT,
                    WPARAM,
                },
                Graphics::Gdi::{COLOR_WINDOW, HBRUSH},
                System::LibraryLoader::GetModuleHandleW,
                UI::WindowsAndMessaging::{
                    DefWindowProcW, LoadCursorW, RegisterClassExW, ShowWindow, CS_HREDRAW,
                    CS_VREDRAW, HICON, IDC_ARROW, SW_HIDE, WM_CLOSE, WM_CREATE, WNDCLASSEXW,
                },
            },
        };
        /// A blank system procedure used when creating new window class.
        ///
        /// Adapted from `blank_window_proc` in [`native_windows_gui::win32::window`].
        unsafe extern "system" fn blank_window_proc(
            hwnd: HWND,
            msg: u32,
            w: WPARAM,
            l: LPARAM,
        ) -> LRESULT {
            let handled = match msg {
                WM_CREATE => true,
                WM_CLOSE => {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                    true
                }
                _ => false,
            };

            if handled {
                LRESULT(0)
            } else {
                DefWindowProcW(hwnd, msg, w, l)
            }
        }

        let module = unsafe { GetModuleHandleW(PCWSTR::null())? };
        let class_name = Self::class_name();

        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(blank_window_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: module.into(),
            hIcon: HICON(null_mut()),
            hCursor: unsafe { LoadCursorW(HINSTANCE(null_mut()), IDC_ARROW) }?,
            hbrBackground: HBRUSH(COLOR_WINDOW.0 as *mut _),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: PCWSTR::from_raw(class_name.as_ptr()),
            hIconSm: HICON(null_mut()),
        };

        let class_token = unsafe { RegisterClassExW(&class) };
        if class_token == 0 && unsafe { GetLastError() } != ERROR_CLASS_ALREADY_EXISTS {
            Err(windows::core::Error::from_win32())
        } else {
            Ok(())
        }
    }
    fn lazy_create_window_class() -> windows::core::Result<()> {
        use std::sync::atomic::{AtomicBool, Ordering};
        static HAS_INIT: AtomicBool = AtomicBool::new(false);
        if HAS_INIT.load(Ordering::Acquire) {
            return Ok(());
        }
        if let Err(e) = Self::create_window_class() {
            tracing::error!(
                error =? e,
                class_name = Self::CLASS_NAME_UTF8,
                "Failed to create window class for invisible window"
            );
            Err(e)
        } else {
            HAS_INIT.store(true, Ordering::Release);
            Ok(())
        }
    }
    pub fn create() -> Result<Self, windows::core::Error> {
        unsafe {
            use windows::{
                core::PCWSTR,
                Win32::{
                    Foundation::HWND,
                    System::LibraryLoader::GetModuleHandleW,
                    UI::WindowsAndMessaging::{
                        CreateWindowExW, CW_USEDEFAULT, HMENU, WINDOW_EX_STYLE, WINDOW_STYLE,
                        WS_POPUP, WS_VISIBLE,
                    },
                },
            };
            Self::lazy_create_window_class()?;
            let module = GetModuleHandleW(PCWSTR::null())?;
            let title = [0];
            let class_name = Self::class_name();
            let handle = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR::from_raw(class_name.as_ptr()),
                PCWSTR::from_raw(title.as_ptr()),
                WINDOW_STYLE(0) | WS_POPUP | WS_VISIBLE,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                0,
                0,
                HWND(null_mut()),
                HMENU(null_mut()),
                module,
                None,
            )?;
            if handle.0.is_null() {
                return Err(windows::core::Error::from_win32());
            }
            Ok(Self(handle))
        }
    }
    pub fn set_foreground(&self) {
        unsafe {
            let _ = SetForegroundWindow(self.0);
        }
    }
    pub fn set_focus(&self) {
        unsafe {
            _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(self.0);
        }
    }
}
impl Drop for CustomInvisibleWindow {
    fn drop(&mut self) {
        if let Err(e) = unsafe { windows::Win32::UI::WindowsAndMessaging::DestroyWindow(self.0) } {
            tracing::warn!(error = ?e, "Failed to destroy window");
        }
    }
}

#[allow(dead_code)]
pub fn switch_desktop_with_invisible_window(
    desktop: vd::Desktop,
    parent: Option<nwg::ControlHandle>,
) -> Result<(), Box<dyn std::error::Error>> {
    //{
    //    let custom = CustomInvisibleWindow::create()?;
    //    std::thread::sleep(Duration::from_millis(500));
    //    if let Err(e) = vd::move_window_to_desktop(desktop, &custom.0) {
    //        tracing::warn!(error = ?e, "Failed to move custom window");
    //    }
    //    std::thread::sleep(Duration::from_millis(500));
    //    custom.set_foreground();
    //    custom.set_focus();
    //
    //    std::thread::sleep(Duration::from_millis(1000));
    //    panic!("testing");
    //}

    let mut empty_parent;
    let parent = if let Some(parent) = parent {
        Some(parent)
    } else {
        empty_parent = nwg::MessageWindow::default();
        nwg::MessageWindow::builder()
            .build(&mut empty_parent)
            .ok()
            .map(|()| empty_parent.handle)
    };
    let ui = InvisibleWindow::build_ui(InvisibleWindow {
        parent,
        ex_flags: 0,
        window: Default::default(),
    })
    .expect("Failed to create invisible window");

    // Move the window to the wanted virtual desktop:
    let try_move =
        || vd::move_window_to_desktop(desktop, &HWND(ui.window.handle.hwnd().unwrap().cast()));
    if let Err(_e) = try_move() {
        // Sometimes winvd doesn't find the created window. (not often, but still)
        tracing::error!("Failed to find the created window: {_e:?}");
        std::thread::sleep(Duration::from_millis(100));
        try_move()?;
    }

    /// Don't close the window immediately since that would cancel the window focus change.
    struct Guard<'a>(&'a InvisibleWindow);
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            std::thread::sleep(Duration::from_millis(100));
            self.0.window.close();
        }
    }
    let _ui_guard = Guard(&ui);

    // Then force show the window to steal focus after it has been moved:
    // std::thread::sleep(Duration::from_millis(25));
    ui.window.set_visible(true);
    ui.window.restore();
    ui.set_foreground();
    ui.window.set_focus();
    Ok(())

    // OLD CODE that manually created window:

    //use windows::Win32::UI::WindowsAndMessaging::WS_EX_TRANSPARENT;
    //
    //let mut window = core::mem::ManuallyDrop::new(Default::default());
    //nwg::Window::builder()
    //    .flags(nwg::WindowFlags::VISIBLE | nwg::WindowFlags::POPUP)
    //    .ex_flags(WS_EX_TRANSPARENT.0)
    //    .size((5, 5))
    //    .title("")
    //    .build(&mut window)
    //    .expect("Failed to build invisible window");
    //
    //vd::move_window_to_desktop(
    //    desktop,
    //    &(windows::Win32::Foundation::HWND(window.handle.hwnd().unwrap() as isize)),
    //)?;
    //Ok(())
} //
