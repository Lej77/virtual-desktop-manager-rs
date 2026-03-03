extern crate native_windows_gui as nwg;

use eframe::{egui, UserEvent};
use nwg::{ControlHandle, NwgError};
use std::any::TypeId;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex};
use virtual_desktop_manager_core::dynamic_gui::{DynamicUiHooks, PartialUiDyn};
use virtual_desktop_manager_core::settings::UiSettings;
use virtual_desktop_manager_core::tray::{SystemTray, TrayPlugin};
use virtual_desktop_manager_core::ConfigWindowGui;
use winit::event_loop::EventLoop;
use winit::platform::run_on_demand::EventLoopExtRunOnDemand;
use winit::platform::windows::EventLoopBuilderExtWindows;

#[derive(Default)]
struct MyEguiApp {
    shared: Arc<Mutex<SharedState>>,
}

impl MyEguiApp {
    fn new(cc: &eframe::CreationContext<'_>, shared: Arc<Mutex<SharedState>>) -> Self {
        // Customize egui here with cc.egui_ctx.set_fonts and cc.egui_ctx.set_visuals.
        // Restore app state using cc.storage (requires the "persistence" feature).
        // Use the cc.gl (a glow::Context) to create graphics shaders and buffers that you can use
        // for e.g. egui::PaintCallback.
        shared.lock().unwrap().window = Some(cc.egui_ctx.clone());
        cc.egui_ctx.send_viewport_cmd(egui::ViewportCommand::Focus);

        Self { shared }
    }
}

impl eframe::App for MyEguiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        {
            let mut guard = self.shared.lock().unwrap();
            match guard.state {
                State::Open => {}
                State::Refocus => {
                    guard.state = State::Open;
                    drop(guard);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                State::Closed | State::Quit => {
                    drop(guard);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Hello World!");
        });
    }
    fn on_exit(&mut self, _ctx: Option<&eframe::glow::Context>) {
        let mut guard = self.shared.lock().unwrap();
        if guard.state != State::Quit {
            guard.state = State::Closed;
        }
    }
}

#[derive(Clone, Copy, PartialOrd, PartialEq, Eq, Ord, Debug, Default)]
pub enum State {
    Open,
    Refocus,
    Closed,
    #[default]
    Quit,
}
impl State {
    pub fn is_available(self) -> bool {
        self != State::Quit
    }
}

#[derive(Default)]
pub struct SharedState {
    pub state: State,
    pub window: Option<egui::Context>,
}
impl SharedState {
    pub fn notify_window_of_change(&self) {
        let Some(window) = &self.window else { return };
        window.request_repaint();
    }
}

#[derive(Default)]
pub struct ConfigWindow {
    pub active_window: RefCell<(Arc<Mutex<SharedState>>, Arc<Condvar>)>,
}
impl PartialUiDyn for ConfigWindow {
    fn build_partial_dyn(&mut self, _parent: Option<ControlHandle>) -> Result<(), NwgError> {
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
        let (current_window, current_condvar) = self.active_window.borrow().clone();
        let mut guard = current_window.lock().unwrap();
        if !guard.state.is_available() {
            drop(guard);
            let mut shared = SharedState::default();
            shared.state = State::Open;
            let shared = Arc::new(Mutex::new(shared));
            let condvar = Arc::new(Condvar::new());

            std::thread::Builder::new()
                .name("Config Window Thread (egui)".to_owned())
                .spawn({
                    let shared = shared.clone();
                    let condvar = condvar.clone();
                    move || {
                        let result = EventLoop::<UserEvent>::with_user_event()
                            .with_any_thread(true)
                            .build()
                            .and_then(|mut event_loop| loop {
                                {
                                    let native_options = eframe::NativeOptions {
                                        viewport: egui::ViewportBuilder {
                                            title: Some("Virtual Desktop Manager".to_owned()),
                                            // https://github.com/emilk/egui/discussions/1574
                                            icon: Some(Arc::new(
                                                eframe::icon_data::from_png_bytes(include_bytes!(
                                                    "../../../Icons/edges - transparent with white.png"
                                                )).expect("The icon data must be valid"),
                                            )),
                                            // icon: Some(Arc::new(egui::IconData::default())),
                                            active: Some(true),
                                            ..Default::default()
                                        },
                                        ..Default::default()
                                    };
                                    let mut app = eframe::create_native(
                                        "Virtual Desktop Manager",
                                        native_options,
                                        Box::new(|cc| Ok(Box::new(MyEguiApp::new(cc, shared.clone())))),
                                        &event_loop,
                                    );
                                    event_loop.run_app_on_demand(&mut app)?;
                                }

                                let guard = condvar
                                    .wait_while(shared.lock().unwrap(), |shared| {
                                        shared.state == State::Closed
                                    })
                                    .unwrap();
                                if guard.state == State::Quit {
                                    break Ok(());
                                }
                            });
                        if let Err(e) = result {
                            tracing::error!("Failed to run egui based config window: {}", e);
                        }
                    }
                })
                .unwrap();
            _ = self.active_window.replace((shared, condvar));
        } else {
            guard.state = if guard.state == State::Closed {
                State::Open
            } else if refocus {
                State::Refocus
            } else {
                State::Closed
            };
            guard.notify_window_of_change();
            current_condvar.notify_all();
        }
    }
}
impl Drop for ConfigWindow {
    fn drop(&mut self) {
        let (shared, condvar) = self.active_window.get_mut();
        let mut guard = shared.lock().unwrap();
        guard.state = State::Closed;
        SharedState::notify_window_of_change(&guard);
        condvar.notify_all();
    }
}
