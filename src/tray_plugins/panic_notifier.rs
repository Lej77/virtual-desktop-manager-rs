use crate::{
    dynamic_gui::DynamicUiHooks,
    tray::{SystemTray, TrayPlugin, TrayRoot},
};
use nwd::NwgPartial;
use std::{
    any::TypeId,
    cell::{OnceCell, RefCell},
    rc::{Rc, Weak},
    sync::{mpsc, Arc, Mutex},
};

struct ThreadLocalPanicHandler {
    tray: RefCell<Weak<SystemTray>>,
    notice: OnceCell<Arc<Mutex<nwg::NoticeSender>>>,
    panic_messages: OnceCell<mpsc::Receiver<String>>,
}
impl ThreadLocalPanicHandler {
    pub const fn new() -> Self {
        Self {
            tray: RefCell::new(Weak::new()),
            notice: OnceCell::new(),
            panic_messages: OnceCell::new(),
        }
    }
    thread_local! {
        static LOCAL: ThreadLocalPanicHandler = const { ThreadLocalPanicHandler::new() };
    }
}

/// Sets a panic hook that displays any panic as a notification.
#[derive(Default, NwgPartial)]
pub struct PanicNotifier {
    #[nwg_control]
    #[nwg_events(OnNotice: [Self::on_panic_notice])]
    panic_notice: nwg::Notice,
}
impl DynamicUiHooks<SystemTray> for PanicNotifier {
    fn before_partial_build(
        &mut self,
        tray_ui: &Rc<SystemTray>,
        _should_build: &mut bool,
    ) -> Option<(nwg::ControlHandle, TypeId)> {
        Some((tray_ui.root().window.handle, TypeId::of::<TrayRoot>()))
    }
    fn after_partial_build(&mut self, tray_ui: &Rc<SystemTray>) {
        let shared_sender =
            ThreadLocalPanicHandler::LOCAL.with(|state: &ThreadLocalPanicHandler| {
                state.tray.replace(Rc::downgrade(tray_ui));

                let sender = self.panic_notice.sender();
                if let Some(shared) = state.notice.get() {
                    // Updated existing panic hook:
                    *shared.lock().unwrap() = sender;
                    None
                } else {
                    let shared = Arc::new(Mutex::new(sender));
                    state.notice.set(shared.clone()).ok().unwrap();
                    Some(shared)
                }
            });
        let Some(shared_sender) = shared_sender else {
            return; // Already has hook
        };

        let prev = std::panic::take_hook();
        let (tx, rx) = mpsc::channel();

        ThreadLocalPanicHandler::LOCAL.with(|state: &ThreadLocalPanicHandler| {
            state
                .panic_messages
                .set(rx)
                .expect("Failed to initialize panic notifier");
        });
        std::panic::set_hook(Box::new(move |info| {
            prev(info);

            ThreadLocalPanicHandler::LOCAL.with(|shared: &ThreadLocalPanicHandler| {
                if let Some(this) = { shared.tray.borrow().upgrade() } {
                    // Panic on main thread so can display notification immediately:
                    Self::display_panic_notification(&this, &info);
                } else {
                    // Send error to main thread and notify the user:
                    if tx.send(info.to_string()).is_ok() {
                        shared_sender.lock().unwrap().notice();
                    }
                }
            });
        }));
    }
}
impl TrayPlugin for PanicNotifier {}
impl PanicNotifier {
    fn display_panic_notification(tray_ui: &SystemTray, info: &dyn std::fmt::Display) {
        tray_ui.show_notification(
            "Virtual Desktop Manager Panicked!",
            "Virtual Desktop Manager encountered a panic and might no longer work correctly, it is recommended to restart the program.",
        );
        // Show panic message in separate notification since we
        // can't fit more text in the first one:
        tray_ui.show_notification("Virtual Desktop Manager Panic Info:", &format!("{info}"));
    }
    fn on_panic_notice(&self) {
        ThreadLocalPanicHandler::LOCAL.with(|shared: &ThreadLocalPanicHandler| {
            while let Some(msg) = shared
                .panic_messages
                .get()
                .and_then(|rx| rx.try_recv().ok())
            {
                if let Some(tray_ui) = { shared.tray.borrow().upgrade() } {
                    Self::display_panic_notification(&tray_ui, &msg);
                }
            }
        });
    }
}
