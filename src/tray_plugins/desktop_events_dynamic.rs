//! Tray plugin that registers to Virtual Desktop events using the dynamic
//! library `VirtualDesktopAccessor.dll`.

#![cfg(feature = "winvd_dynamic")]

use windows::Win32::Foundation::HWND;

use crate::{
    dynamic_gui::DynamicUiHooks,
    nwg_ext::FastTimerControl,
    tray::{SystemTray, SystemTrayRef, TrayPlugin, TrayRoot},
    vd,
};
use std::{any::TypeId, cell::Cell, cmp::Ordering, rc::Rc, time::Duration};

/// Any value between WM_USER (0x0400 = 1024) and 0x7FFF (32767) can be used
/// according to
/// <https://learn.microsoft.com/en-us/windows/win32/winmsg/wm-user>. But don't
/// use any already used by nwg (see top of its `window_helper.rs` file).
///
/// Current value was suggested in AutoHotkey example script inside `winvd`
/// repository.
const MESSAGE_OFFSET: u32 = 0x1400;

#[derive(nwd::NwgPartial, Default)]
pub struct DynamicVirtualDesktopEventManager {
    tray_ref: SystemTrayRef,
    #[nwg_control(interval: Duration::from_millis(1000))]
    #[nwg_events(OnNotice: [Self::on_poll_timer])]
    poll_timer: FastTimerControl,
    registered_at: Cell<Option<HWND>>,
    prev_window_count: Cell<u32>,
}
impl DynamicVirtualDesktopEventManager {
    fn on_poll_timer(&self) {
        let Some(tray) = self.tray_ref.get() else {
            return;
        };
        let Some(Ok(_)) = vd::dynamic::get_loaded_symbols() else {
            return;
        };
        let new_count = match vd::get_desktop_count() {
            Ok(count) => count,
            Err(e) => {
                tracing::warn!("Failed to get desktop count from the dynamic library: {e:?}");
                return;
            }
        };

        match new_count.cmp(&self.prev_window_count.get()) {
            Ordering::Equal => return,
            Ordering::Less => {
                tray.notify_desktop_event(vd::DesktopEvent::DesktopDestroyed {
                    destroyed: vd::get_desktop(self.prev_window_count.get() - 1),
                    fallback: match vd::get_current_desktop() {
                        Ok(desk) => desk,
                        Err(e) => {
                            tracing::warn!(
                                "Failed to get current desktop from the dynamic library: {e:?}"
                            );
                            return;
                        }
                    },
                });
            }
            Ordering::Greater => {
                tray.notify_desktop_event(vd::DesktopEvent::DesktopCreated(vd::get_desktop(
                    new_count - 1,
                )));
            }
        }

        self.prev_window_count.set(new_count);
    }
}
impl DynamicUiHooks<SystemTray> for DynamicVirtualDesktopEventManager {
    fn before_partial_build(
        &mut self,
        tray: &Rc<SystemTray>,
        _should_build: &mut bool,
    ) -> Option<(nwg::ControlHandle, TypeId)> {
        self.tray_ref.set(tray);
        Some((tray.root().window.handle, TypeId::of::<TrayRoot>()))
    }
    fn after_partial_build(&mut self, tray_ui: &Rc<SystemTray>) {
        let Some(Ok(symbols)) = vd::dynamic::get_loaded_symbols() else {
            self.poll_timer.cancel_last();
            return;
        };
        let handle = tray_ui.root().window.handle;
        let handle = HWND(
            handle
                .hwnd()
                .expect("Root window should have a valid handle") as isize,
        );

        let res = unsafe { symbols.RegisterPostMessageHook(handle, MESSAGE_OFFSET) };
        if let Err(e) = res {
            tracing::error!("Failed to register post message hook for virtual desktop events from the dynamic library: {e:?}");
            tray_ui.show_notification(
                "Virtual Desktop Manager Error",
                &format!("Failed to start listening for virtual desktop events: {e:?}"),
            );
        } else {
            self.registered_at.set(Some(handle));
        }
    }
    fn before_rebuild(&mut self, _dynamic_ui: &Rc<SystemTray>) {
        let mut old = std::mem::take(self);
        let Some(Ok(symbols)) = vd::dynamic::get_loaded_symbols() else {
            return;
        };

        let Some(hwnd) = old.registered_at.get_mut().take() else {
            return;
        };

        if let Err(e) = unsafe { symbols.UnregisterPostMessageHook(hwnd) } {
            tracing::warn!("Failed to unregister post message hook for virtual desktop events from the dynamic library: {e:?}");
        }
    }
    fn process_raw_event(
        &self,
        dynamic_ui: &Rc<SystemTray>,
        hwnd: isize,
        msg: u32,
        w: usize,
        l: isize,
        _window: nwg::ControlHandle,
    ) -> Option<isize> {
        if Some(HWND(hwnd)) != self.registered_at.get() {
            return None;
        }
        if msg != MESSAGE_OFFSET {
            return None;
        }
        dynamic_ui.notify_desktop_event(vd::DesktopEvent::DesktopChanged {
            old: vd::get_desktop(w as u32),
            new: vd::get_desktop(l as u32),
        });
        None
    }
}
impl TrayPlugin for DynamicVirtualDesktopEventManager {}
