extern crate native_windows_gui as nwg;

use nwg::{ControlHandle, NwgError};
use std::any::TypeId;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard};
use virtual_desktop_manager_core::dynamic_gui::{DynamicUiHooks, PartialUiDyn};
use virtual_desktop_manager_core::settings::UiSettings;
use virtual_desktop_manager_core::tray::{SystemTray, TrayPlugin};
use virtual_desktop_manager_core::ConfigWindowGui;
use winsafe::msg::WndMsg;
use winsafe::{self as w, co, gui, prelude::*};
use winsafe::gui::Icon;

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

    /// Shared state was changed.
    pub const STATE_CHANGED: co::WM = unsafe { co::WM::from_raw(0x9040) };
}

#[derive(Clone)]
pub struct MyWindow {
    wnd: gui::WindowMain,   // responsible for managing the window
    btn_hello: gui::Button, // a button
    shared: Arc<Mutex<SharedState>>,
}

impl MyWindow {
    pub fn new(shared: Arc<Mutex<SharedState>>) -> Self {
        let wnd = gui::WindowMain::new(
            // instantiate the window manager
            gui::WindowMainOpts {
                title: "Virtual Desktop Manager",
                class_icon: Icon::Id(1),
                size: gui::dpi(300, 150),
                style: gui::WindowMainOpts::default().style | co::WS::SIZEBOX | co::WS::MINIMIZEBOX | co::WS::MAXIMIZEBOX,
                ..Default::default() // leave all other options as default
            },
        );

        let btn_hello = gui::Button::new(
            &wnd, // the window manager is the parent of our button
            gui::ButtonOpts {
                text: "&Click me",
                position: gui::dpi(20, 20),
                ..Default::default()
            },
        );

        {
            let mut guard = shared.lock().unwrap();
            guard.window = Some(wnd.clone());
            guard.state = State::Open;
        }
        let new_self = Self {
            wnd,
            btn_hello,
            shared,
        };
        new_self.events(); // attach our events
        new_self
    }

    pub fn run(self) -> w::AnyResult<i32> {
        self.wnd.run_main(None) // show the main window; will block until closed
    }

    fn events(&self) {
        self.wnd.on().wm(custom_msg::STATE_CHANGED, {
            let wnd = self.wnd.clone();
            let shared = self.shared.clone();
            move |_params| {
                let mut guard = shared.lock().unwrap();
                match guard.state {
                    State::Open => {}
                    State::Refocus => {
                        guard.state = State::Open;
                        drop(guard);
                        wnd.hwnd().SetForegroundWindow();
                    }
                    State::Closed => {
                        drop(guard);
                        wnd.close();
                    }
                }
                Ok(0)
            }
        });
        self.btn_hello.on().bn_clicked({
            let wnd = self.wnd.clone();
            move || {
                wnd.hwnd().SetWindowText("Hello, world!")?; // call native Windows API
                Ok(())
            }
        });
    }
}
impl Drop for MyWindow {
    fn drop(&mut self) {
        let mut guard = self.shared.lock().unwrap();
        guard.state = State::Closed;
        guard.window = None;
    }
}

#[derive(Clone, Copy, PartialOrd, PartialEq, Eq, Ord, Debug, Default)]
pub enum State {
    Open,
    Refocus,
    #[default]
    Closed,
}
impl State {
    pub fn is_available(self) -> bool {
        self != State::Closed
    }
}

#[derive(Default)]
pub struct SharedState {
    pub state: State,
    pub window: Option<gui::WindowMain>,
}
impl SharedState {
    pub fn notify_window_of_change(this: MutexGuard<'_, SharedState>) {
        let Some(window) = this.window.clone() else {
            return;
        };
        drop(this);

        // Safety: we send a message in the range 0x8000 through 0xBFFF so it won't be interpreted by other controls.
        unsafe {
            window.hwnd().SendMessage(WndMsg {
                msg_id: custom_msg::STATE_CHANGED,
                wparam: 0,
                lparam: 0,
            })
        };
    }
}

#[derive(Default)]
pub struct ConfigWindow {
    pub active_window: RefCell<Arc<Mutex<SharedState>>>,
}
impl PartialUiDyn for ConfigWindow {
    fn build_partial_dyn(
        &mut self,
        _parent: Option<ControlHandle>,
    ) -> std::result::Result<(), NwgError> {
        Ok(())
    }
}
impl DynamicUiHooks<SystemTray> for ConfigWindow {
    fn before_partial_build(
        &mut self,
        _dynamic_ui: &Rc<SystemTray>,
        _should_build: &mut bool,
    ) -> Option<(ControlHandle, TypeId)> {
        None
    }
}
impl TrayPlugin for ConfigWindow {
    fn on_settings_changed(
        &self,
        _tray_ui: &Rc<SystemTray>,
        _prev: &Arc<UiSettings>,
        _new: &Arc<UiSettings>,
    ) {
    }
}
impl ConfigWindowGui for ConfigWindow {
    fn configure_filters(&self, refocus: bool) {
        let current_window = self.active_window.borrow().clone();
        let mut guard = current_window.lock().unwrap();
        if !guard.state.is_available() {
            drop(guard);
            let mut shared = SharedState::default();
            shared.state = State::Open;
            let shared = Arc::new(Mutex::new(shared));

            std::thread::Builder::new()
                .name("Config Window Thread (Winsafe)".to_owned())
                .spawn({
                    let shared = shared.clone();
                    move || {
                        let window = MyWindow::new(shared);
                        if let Err(e) = window.run() {
                            tracing::error!("Failed to run Config Window: {e}");
                        }
                    }
                })
                .unwrap();
            _ = self.active_window.replace(shared);
        } else {
            guard.state = if guard.state == State::Closed {
                State::Open
            } else if refocus {
                State::Refocus
            } else {
                State::Closed
            };
            SharedState::notify_window_of_change(guard);
        }
    }
}
impl Drop for ConfigWindow {
    fn drop(&mut self) {
        let mut guard = self.active_window.get_mut().lock().unwrap();
        guard.state = State::Closed;
        SharedState::notify_window_of_change(guard);
    }
}
