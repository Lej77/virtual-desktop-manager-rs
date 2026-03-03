extern crate native_windows_gui as nwg;

use crate::config_window::WinsafeSettingsWindow;
use nwg::{ControlHandle, Event, EventData, NwgError};
use raw_window_handle::{
    DisplayHandle, HandleError, RawWindowHandle, Win32WindowHandle, WindowHandle,
};
use std::any::TypeId;
use std::cell::RefCell;
use std::num::NonZeroIsize;
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use virtual_desktop_manager_core::dynamic_gui::{DynamicUiHooks, PartialUiDyn};
use virtual_desktop_manager_core::settings::{
    UiSettings, UiSettingsChange, UiSettingsChangeDebouncer,
};
use virtual_desktop_manager_core::tray::{SystemTray, SystemTrayRef, TrayPlugin, TrayRoot};
use virtual_desktop_manager_core::ConfigWindowGui;
use winsafe::msg::WndMsg;
use winsafe::{gui, prelude::*};

mod config_window;
mod filter_options;
pub mod layout;
mod program_settings;

mod custom_msg {
    //! Custom messages.
    //!
    //! WM_APP (0x8000) through 0xBFFF - Messages available for use by applications.
    //!
    //! # References
    //!
    //! - https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-registerwindowmessagea
    //! - https://learn.microsoft.com/en-us/windows/win32/winmsg/wm-user

    use winsafe::co;

    /// [`crate::SharedStateMut`] was changed.
    pub const STATE_CHANGED: co::WM = unsafe { co::WM::from_raw(0x9040) };

    /// [`crate::BackgroundThread`] has new data in its receiver.
    pub const WINDOW_INFO_AVAILABLE: co::WM = unsafe { co::WM::from_raw(0x9041) };

    pub const DELAYED_MAXIMIZE: co::WM = unsafe { co::WM::from_raw(0x9042) };

    /// Indicates that the UI changed a filter value. This message is received after the change has been applied.
    pub const WM_FILTER_CHANGED: co::WM = unsafe { co::WM::from_raw(0xB040) };
    pub const WM_FILTER_ID_CHANGED: co::WM = unsafe { co::WM::from_raw(0xB041) };

    /// Indicates that the UI changed a global setting value.
    ///This message is received after the change has been applied.
    pub const WM_SETTING_CHANGED: co::WM = unsafe { co::WM::from_raw(0xB140) };
}

pub trait GuiParentWithEvents: GuiParent + Clone + 'static {
    fn on(&self) -> &impl GuiEventsParent;
}
macro_rules! impl_gui_parent_with_events {
    ($($type:ty),* $(,)?) => {
        $(
            impl GuiParentWithEvents for $type {
                fn on(&self) -> &impl GuiEventsParent {
                    self.on()
                }
            }
        )*
    }
}
impl_gui_parent_with_events!(
    gui::PropSheetPage,
    gui::TabPage,
    gui::WindowControl,
    gui::WindowMain,
    gui::WindowMessageOnly,
    gui::WindowModal,
    gui::WindowModeless,
);

/// Get a [`native_windows_gui::ControlHandle`] from a GUI type in the [`winsafe`] crate.
pub trait NativeWindowHandle {
    fn native_handle(&self) -> ControlHandle;
}
macro_rules! impl_native_window_handle {
    ($($type:ty),* $(,)?) => {
        $(
            impl NativeWindowHandle for $type  {
                fn native_handle(&self) -> ControlHandle {
                    ControlHandle::Hwnd(self.hwnd().ptr() as _)
                }
            }
        )*
    }
}
impl_native_window_handle!(
    gui::Button,
    gui::CheckBox,
    gui::Edit,
    gui::ComboBox,
    gui::Label,
);

/// Exposes a `winsafe::HWND` handle to libraries that expect a [`raw_window_handle::WindowHandle`].
struct WinsafeHandleToRawHandle<'a>(&'a winsafe::HWND);
impl raw_window_handle::HasWindowHandle for WinsafeHandleToRawHandle<'_> {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let raw = RawWindowHandle::Win32(Win32WindowHandle::new(
            NonZeroIsize::new(self.0.ptr() as isize).ok_or(HandleError::Unavailable)?,
        ));
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}
impl raw_window_handle::HasDisplayHandle for WinsafeHandleToRawHandle<'_> {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        Ok(DisplayHandle::windows())
    }
}

#[derive(Clone, Copy, PartialOrd, PartialEq, Eq, Ord, Debug, Default)]
pub enum WindowState {
    Open,
    Refocus,
    #[default]
    Closed,
}
impl WindowState {
    pub fn is_available(self) -> bool {
        self != WindowState::Closed
    }
}

pub struct SharedStateMut {
    /// The current target state of the config window.
    pub state: WindowState,
    /// The program settings. `old` refers to the latest known state for the program while `new`
    /// refers to the desired settings that should be set/published when possible.
    ///
    /// - Wake main thread if `new` changes should be published.
    /// - Wake config window thread if `old` changed and `new` changes should be rolled back.
    pub tracked_settings: UiSettingsChange,
    /// A handle to the config window.
    pub window: Option<gui::WindowMain>,
    pub wake_settings_change: nwg::NoticeSender,
    pub wake_apply_filters: nwg::NoticeSender,
}
impl SharedStateMut {
    pub fn notify_window_of_change(this: MutexGuard<'_, SharedStateMut>) {
        let Some(window) = this.window.clone() else {
            return;
        };
        drop(this);

        // Safety: we send a message in the range 0x8000 through 0xBFFF so it won't be interpreted by other controls.
        _ = unsafe {
            window.hwnd().PostMessage(WndMsg {
                msg_id: custom_msg::STATE_CHANGED,
                wparam: 0,
                lparam: 0,
            })
        };
    }
    pub fn notify_main_of_settings_change(this: MutexGuard<'_, SharedStateMut>) {
        let wake_main = this.wake_settings_change.clone();
        drop(this);
        wake_main.notice();
    }
    pub fn notify_main_to_apply_filters(this: MutexGuard<'_, SharedStateMut>) {
        let wake_main = this.wake_apply_filters.clone();
        drop(this);
        wake_main.notice();
    }
}

pub struct SharedState {
    pub mutex: Mutex<SharedStateMut>,
}
impl SharedState {
    pub fn new(
        wanted_state: WindowState,
        settings: Arc<UiSettings>,
        wake_settings_change: nwg::NoticeSender,
        wake_apply_filters: nwg::NoticeSender,
    ) -> Self {
        Self {
            mutex: Mutex::new(SharedStateMut {
                state: wanted_state,
                tracked_settings: UiSettingsChange {
                    old: settings.clone(),
                    new: settings,
                },
                window: None,
                wake_settings_change,
                wake_apply_filters,
            }),
        }
    }
}

#[derive(Default)]
pub struct ConfigWindow {
    /// State shared with the latest config window.
    pub active_window: RefCell<Weak<SharedState>>,
    /// Wakes the main thread.
    update_settings_waker: nwg::Notice,
    /// Wakes the main thread.
    apply_filters_waker: nwg::Notice,
    /// Reference to the system tray and all plugins.
    tray: SystemTrayRef,
    /// Tracks settings changes issued by the config window. This allows us to ignore setting change
    /// events that we already know about.
    settings_debouncer: RefCell<UiSettingsChangeDebouncer>,
}
impl ConfigWindow {
    fn on_notice_settings(&self) {
        let Some(shared) = self.active_window.borrow().upgrade() else {
            return;
        };
        let Some(tray) = self.tray.get() else { return };
        let current_settings = tray.settings().get();
        let mut guard = shared.mutex.lock().unwrap();
        if guard.tracked_settings.is_unchanged() {
            return;
        }
        if !Arc::ptr_eq(&guard.tracked_settings.old, &current_settings) {
            // Someone else changed settings while our changes were pending, so ignore those changes
            tracing::warn!(
                "Ignoring setting change in config window since \
                newer changes arrived from elsewhere"
            );
            guard.tracked_settings = UiSettingsChange {
                old: current_settings.clone(),
                new: current_settings.clone(),
            };
            SharedStateMut::notify_window_of_change(guard);
            return;
        }
        let new_settings = guard.tracked_settings.new.clone();
        drop(guard);
        self.settings_debouncer
            .borrow_mut()
            .track_unpublished_version(&new_settings);
        tray.settings().set(new_settings);
    }
    fn on_notice_apply_filters(&self) {
        let Some(tray) = self.tray.get() else { return };
        tray.apply_filters();
    }
}
impl PartialUiDyn for ConfigWindow {
    fn build_partial_dyn(&mut self, parent: Option<ControlHandle>) -> Result<(), NwgError> {
        let parent_ref = parent
            .as_ref()
            .expect("ConfigWindow requires a parent control");
        nwg::Notice::builder()
            .parent(parent_ref)
            .build(&mut self.update_settings_waker)?;
        nwg::Notice::builder()
            .parent(parent_ref)
            .build(&mut self.apply_filters_waker)?;
        Ok(())
    }
    fn process_event_dyn(&self, evt: Event, _evt_data: &EventData, handle: ControlHandle) {
        match evt {
            Event::OnNotice if &handle == &self.update_settings_waker => {
                Self::on_notice_settings(&self)
            }
            Event::OnNotice if &handle == &self.apply_filters_waker => {
                Self::on_notice_apply_filters(&self)
            }
            _ => {}
        }
    }
}
impl DynamicUiHooks<SystemTray> for ConfigWindow {
    fn before_partial_build(
        &mut self,
        dynamic_ui: &Rc<SystemTray>,
        _should_build: &mut bool,
    ) -> Option<(ControlHandle, TypeId)> {
        self.tray.set(dynamic_ui);
        Some((dynamic_ui.root().window.handle, TypeId::of::<TrayRoot>()))
    }
    fn after_partial_build(&mut self, _dynamic_ui: &Rc<SystemTray>) {
        if let Some(shared) = self.active_window.borrow().upgrade() {
            let settings_waker = self.update_settings_waker.sender();
            let apply_filters_waker = self.apply_filters_waker.sender();
            let mut guard = shared.mutex.lock().unwrap();
            guard.wake_settings_change = settings_waker;
            guard.wake_apply_filters = apply_filters_waker;
        }
    }
    // Don't recreate UI event loop on background thread (would close config window if explorer.exe is restarted)
    fn before_rebuild(&mut self, _dynamic_ui: &Rc<SystemTray>) {
        self.update_settings_waker = Default::default();
        self.apply_filters_waker = Default::default();
        self.tray = Default::default();
    }
}
impl TrayPlugin for ConfigWindow {
    fn on_settings_changed(
        &self,
        _tray_ui: &Rc<SystemTray>,
        _prev: &Arc<UiSettings>,
        new: &Arc<UiSettings>,
    ) {
        // true if notified about settings change that we caused (UI is already up to date).
        let was_requested = self
            .settings_debouncer
            .borrow_mut()
            .notified_new_version(new);

        let Some(shared) = self.active_window.borrow().upgrade() else {
            return;
        };
        let mut guard = shared.mutex.lock().unwrap();
        if Arc::ptr_eq(&guard.tracked_settings.old, new) {
            // Already knew about these changes.
            return;
        }
        guard.tracked_settings.old = new.clone();
        if !was_requested {
            tracing::warn!(
                "Ignoring setting change in config window since \
                newer changes arrived from elsewhere"
            );
            // forget any changes we were trying to make.
            guard.tracked_settings.new = new.clone();

            if guard.state != WindowState::Closed {
                SharedStateMut::notify_window_of_change(guard);
            }
        }
    }
}
impl ConfigWindowGui for ConfigWindow {
    fn configure_filters(&self, refocus: bool) {
        let current_window = self.active_window.borrow().upgrade();
        let guard = current_window.as_ref().map(|v| v.mutex.lock().unwrap());
        match guard {
            Some(mut guard) if guard.state.is_available() => {
                guard.state = if guard.state == WindowState::Closed {
                    WindowState::Open
                } else if refocus {
                    WindowState::Refocus
                } else {
                    WindowState::Closed
                };
                SharedStateMut::notify_window_of_change(guard);
            }
            _ => {
                drop(guard);

                let settings = self
                    .tray
                    .get()
                    .map(|tray| tray.settings().get())
                    .unwrap_or_default();

                let mut shared = SharedState::new(
                    WindowState::Open,
                    settings,
                    self.update_settings_waker.sender(),
                    self.apply_filters_waker.sender(),
                );
                shared.mutex.get_mut().unwrap().state = WindowState::Open;
                let shared = Arc::new(shared);
                let weak = Arc::downgrade(&shared);

                std::thread::Builder::new()
                    .name("Config Window Thread (Winsafe)".to_owned())
                    .spawn(move || {
                        let window = WinsafeSettingsWindow::new(shared);
                        if let Err(e) = window.run() {
                            tracing::error!("Failed to run Config Window: {e}");
                        }
                    })
                    .unwrap();
                _ = self.active_window.replace(weak);
            }
        }
    }
}
impl Drop for ConfigWindow {
    fn drop(&mut self) {
        let Some(shared) = self.active_window.get_mut().upgrade() else {
            return;
        };
        let mut guard = shared.mutex.lock().unwrap();
        guard.state = WindowState::Closed;
        SharedStateMut::notify_window_of_change(guard);
    }
}
