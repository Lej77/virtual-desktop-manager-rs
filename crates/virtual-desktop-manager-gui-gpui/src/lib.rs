use gpui::*;
use gpui_component::{button::*, *};
use nwg::{ControlHandle, NwgError};
use std::any::TypeId;
use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};
use virtual_desktop_manager_core::dynamic_gui::{DynamicUiHooks, PartialUiDyn};
use virtual_desktop_manager_core::settings::UiSettings;
use virtual_desktop_manager_core::tray::{SystemTray, TrayPlugin};
use virtual_desktop_manager_core::ConfigWindowGui;

extern crate native_windows_gui as nwg;

pub struct ConfigView;

impl Render for ConfigView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .v_flex()
            .gap_2()
            .size_full()
            .items_center()
            .justify_center()
            .child("Hello, World!")
            .child(
                Button::new("ok")
                    .primary()
                    .label("Let's Go!")
                    .on_click(|_, _, _| println!("Clicked!")),
            )
    }
}

#[derive(Clone, Copy, PartialOrd, PartialEq, Eq, Ord, Debug, Default)]
enum State {
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

#[derive(Default, Debug)]
struct SharedState {
    state: State,
    waker: Option<Waker>,
}

struct WaitForChange<'a> {
    state: &'a Mutex<SharedState>,
    current_state: State,
}
impl Future for WaitForChange<'_> {
    type Output = State;

    fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let mut guard = this.state.lock().unwrap();
        if guard.state != this.current_state {
            if guard.state == State::Refocus {
                guard.state = State::Open;
            }
            Poll::Ready(guard.state)
        } else {
            guard.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

#[derive(Default)]
pub struct ConfigWindow {
    active_window: RefCell<Arc<Mutex<SharedState>>>,
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
        let mut ref_guard = self.active_window.borrow_mut();
        let mut mutex_guard = ref_guard.lock().unwrap();
        if !mutex_guard.state.is_available() {
            drop(mutex_guard);
            let app = create_app();
            *ref_guard = app;
        } else {
            mutex_guard.state = if mutex_guard.state == State::Closed {
                State::Open
            } else if refocus {
                State::Refocus
            } else {
                State::Closed
            };
            if let Some(waker) = mutex_guard.waker.take() {
                waker.wake();
            }
        }
    }
}

fn create_app() -> Arc<Mutex<SharedState>> {
    let mut state = SharedState::default();
    state.state = State::Open;
    let state = Arc::new(Mutex::new(state));

    let state2 = state.clone();
    std::thread::Builder::new()
        .name("Config Window Thread (GPUI)".to_owned())
        .spawn(|| {
            struct OnExit(Arc<Mutex<SharedState>>);
            impl Drop for OnExit {
                fn drop(&mut self) {
                    if let Ok(mut guard) = self.0.lock() {
                        guard.state = State::Quit;
                        guard.waker = None;
                    }
                }
            }
            let on_exit = OnExit(state.clone());

            let app = Application::with_platform(gpui_platform::current_platform(false))
                // https://github.com/longbridge/gpui-component/discussions/1646
                .with_quit_mode(QuitMode::Explicit)
                // GPUI Component icons:
                .with_assets(gpui_component_assets::Assets);

            app.run(move |cx| {
                // This must be called before using any GPUI Component features.
                gpui_component::init(cx);

                fn open_window(cx: &mut AsyncApp) -> gpui::Result<WindowHandle<Root>> {
                    cx.open_window(WindowOptions::default(), |window, cx| {
                        let view = cx.new(|_| ConfigView);
                        // This first level on the window, should be a Root.
                        cx.new(|cx| Root::new(view, window, cx))
                    })
                }

                cx.spawn(async move |cx: &mut AsyncApp| {
                    let mut window = Some(open_window(cx)?);

                    let mut current_state = State::Open;
                    loop {
                        let wanted_state: State = WaitForChange {
                            state: &state,
                            current_state,
                        }
                        .await;
                        if window.is_none() && matches!(wanted_state, State::Open | State::Refocus)
                        {
                            window = Some(open_window(cx)?);
                        }
                        current_state = match wanted_state {
                            State::Open => State::Open,
                            State::Refocus => {
                                _ = cx.update_window(
                                    **window.as_ref().unwrap(),
                                    |_view, window, _cx| {
                                        window.activate_window();
                                    },
                                );
                                State::Open
                            }
                            State::Closed => {
                                if let Some(window) = window.take() {
                                    _ = cx.update_window(*window, |_view, window, _cx| {
                                        window.remove_window();
                                    });
                                }
                                State::Closed
                            }
                            State::Quit => {
                                _ = cx.update(|cx| {
                                    cx.quit();
                                });
                                break;
                            }
                        }
                    }

                    Ok::<_, Box<dyn std::error::Error>>(())
                })
                .detach();
            });
            drop(on_exit);
        })
        .unwrap();
    state2
}
