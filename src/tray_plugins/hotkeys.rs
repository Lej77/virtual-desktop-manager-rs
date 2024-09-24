//! Registers hotkeys using the [`global_hotkey`] crate.
#![cfg(feature = "global_hotkey")]

use crate::{
    dynamic_gui::DynamicUiHooks,
    settings::UiSettings,
    tray::{SystemTray, SystemTrayRef, TrayPlugin, TrayRoot},
};
use std::{
    any::TypeId,
    cell::RefCell,
    rc::Rc,
    sync::{mpsc, Arc, Mutex},
};

#[derive(nwd::NwgPartial)]
pub struct HotKeyPlugin {
    tray: SystemTrayRef,

    hotkey_manager: global_hotkey::GlobalHotKeyManager,
    current_hotkeys: RefCell<Vec<global_hotkey::hotkey::HotKey>>,
    events: mpsc::Receiver<global_hotkey::GlobalHotKeyEvent>,

    latest_notice_sender: Arc<Mutex<Option<nwg::NoticeSender>>>,
    /// This notice will be triggered when there are new Virtual Desktop events
    /// that should be handled.
    #[nwg_control]
    #[nwg_events( OnNotice: [Self::on_background_notice] )]
    background_notice: nwg::Notice,
}
impl Default for HotKeyPlugin {
    fn default() -> Self {
        let latest_notice_sender = Arc::new(Mutex::new(None::<nwg::NoticeSender>));
        let (tx, rx) = mpsc::channel();
        _ = std::thread::Builder::new()
            .name("GlobalHotKeyListenerThread".to_owned())
            .spawn({
                let latest_notice_sender = latest_notice_sender.clone();
                move || {
                    let hotkey_rx = global_hotkey::GlobalHotKeyEvent::receiver();
                    for ev in hotkey_rx.iter() {
                        if tx.send(ev).is_err() {
                            break;
                        }
                        if let Some(sender) = *latest_notice_sender.lock().unwrap() {
                            sender.notice();
                        }
                    }
                }
            });
        Self {
            tray: Default::default(),

            hotkey_manager: global_hotkey::GlobalHotKeyManager::new()
                .expect("Failed to create global keyboard shortcut manager"),
            current_hotkeys: RefCell::new(Vec::new()),
            events: rx,

            latest_notice_sender,
            background_notice: Default::default(),
        }
    }
}
impl DynamicUiHooks<SystemTray> for HotKeyPlugin {
    fn before_partial_build(
        &mut self,
        tray: &Rc<SystemTray>,
        _should_build: &mut bool,
    ) -> Option<(nwg::ControlHandle, TypeId)> {
        self.tray.set(tray);
        Some((tray.root().window.handle, TypeId::of::<TrayRoot>()))
    }
    fn after_partial_build(&mut self, _dynamic_ui: &Rc<SystemTray>) {
        *self.latest_notice_sender.lock().unwrap() = Some(self.background_notice.sender());
        self.update_hotkeys();
    }
    fn before_rebuild(&mut self, _dynamic_ui: &Rc<SystemTray>) {
        self.background_notice = Default::default();
    }
}
impl TrayPlugin for HotKeyPlugin {
    fn on_settings_changed(
        &self,
        _tray_ui: &Rc<SystemTray>,
        prev: &Arc<UiSettings>,
        new: &Arc<UiSettings>,
    ) {
        if !Arc::ptr_eq(&prev.quick_switch_hotkey, &new.quick_switch_hotkey)
            && prev.quick_switch_hotkey != new.quick_switch_hotkey
        {
            self.update_hotkeys();
        }
    }
}
impl HotKeyPlugin {
    fn on_background_notice(&self) {
        let Some(tray) = self.tray.get() else {
            return;
        };
        for event in self.events.try_iter() {
            tracing::debug!(?event, "Received global hotkey");
            if event.state() == global_hotkey::HotKeyState::Pressed {
                tray.notify_quick_switch_hotkey();
            }
        }
    }
    pub fn update_hotkeys(&self) {
        #[cfg(feature = "global_hotkey")]
        {
            let settings = self.tray.get().unwrap().settings().get();
            let Ok(mut guard) = self.current_hotkeys.try_borrow_mut() else {
                tracing::warn!("Tried to update global hotkeys recursively");
                return;
            };
            if let Err(e) = self.hotkey_manager.unregister_all(guard.as_slice()) {
                tracing::error!(error = e.to_string(), "Failed to unregister global hotkeys");
            }
            let mut hotkeys = std::mem::take(&mut *guard);
            hotkeys.clear();

            if !settings.quick_switch_hotkey.is_empty() {
                match settings.quick_switch_hotkey.parse() {
                    Ok(hotkey) => hotkeys.push(hotkey),
                    Err(e) => {
                        tracing::warn!(error = e.to_string(), "Invalid quick switch hotkey");
                    }
                }
            }

            tracing::debug!(hotkeys =? hotkeys.as_slice(), "Registering new hotkeys");

            if hotkeys.is_empty() {
                *guard = hotkeys;
                return;
            }
            if let Err(e) = self.hotkey_manager.register_all(hotkeys.as_slice()) {
                tracing::error!(error = e.to_string(), "Failed to register global hotkeys");
            } else {
                *guard = hotkeys;
            }
        }
    }
}
