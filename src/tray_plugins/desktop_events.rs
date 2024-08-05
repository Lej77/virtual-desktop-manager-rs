//! Tray plugin that forwards Virtual Desktop events to the tray UI using
//! [`winvd`] crate.

#![cfg(feature = "winvd_static")]
use crate::{
    dynamic_gui::DynamicUiHooks,
    tray::{SystemTray, SystemTrayRef, TrayPlugin, TrayRoot},
};
use std::{any::TypeId, cell::RefCell, rc::Rc, sync::mpsc};

#[derive(nwd::NwgPartial, Default)]
pub struct VirtualDesktopEventManager {
    tray: SystemTrayRef,

    /// State used to keep listening to Virtual Desktop events.
    background: RefCell<
        Option<(
            winvd::DesktopEventThread,
            mpsc::Receiver<winvd::DesktopEvent>,
        )>,
    >,

    /// This notice will be triggered when there are new Virtual Desktop events
    /// that should be handled.
    #[nwg_control]
    #[nwg_events( OnNotice: [Self::on_background_event] )]
    background_notice: nwg::Notice,
}
impl DynamicUiHooks<SystemTray> for VirtualDesktopEventManager {
    fn before_partial_build(
        &mut self,
        tray: &Rc<SystemTray>,
        _should_build: &mut bool,
    ) -> Option<(nwg::ControlHandle, TypeId)> {
        self.tray.set(tray);
        Some((tray.root().window.handle, TypeId::of::<TrayRoot>()))
    }
    fn after_partial_build(&mut self, tray_ui: &Rc<SystemTray>) {
        let (sender, receiver_1) = mpsc::channel::<winvd::DesktopEvent>();
        match winvd::listen_desktop_events(sender) {
            Err(e) => {
                tray_ui.show_notification(
                    "Virtual Desktop Manager Error",
                    &format!("Failed to start listening for virtual desktop events: {e:?}"),
                );
            }
            Ok(guard) => {
                let (sender, receiver_2) = mpsc::channel::<winvd::DesktopEvent>();
                let notice = self.background_notice.sender();
                std::thread::spawn(move || {
                    // Forward events and notify the main thread that there are
                    // more messages in the channel.
                    for event in receiver_1 {
                        if sender.send(event).is_err() {
                            return;
                        }
                        notice.notice();
                    }
                });
                self.background.replace(Some((guard, receiver_2)));
            }
        }
    }
    fn before_rebuild(&mut self, _dynamic_ui: &Rc<SystemTray>) {
        self.background_notice = Default::default();
    }
}
impl TrayPlugin for VirtualDesktopEventManager {}
impl VirtualDesktopEventManager {
    fn on_background_event(&self) {
        // Note: this has a shared reference to self and a rebuild can only
        // happen after a mutable reference so "self.background" can never be
        // mutably borrowed during "tray.on_desktop_event(..)".
        let background = self.background.borrow();
        let Some((_, receiver)) = background.as_ref() else {
            return;
        };
        while let Ok(event) = receiver.try_recv() {
            if let Some(tray) = self.tray.get() {
                tray.notify_desktop_event(event.into());
            }
        }
    }
}
