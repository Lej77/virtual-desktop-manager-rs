use crate::{
    dynamic_gui::DynamicUiHooks,
    tray::{SystemTray, TrayPlugin},
    vd,
    window_filter::{FilterAction, WindowFilter},
    window_info::{VirtualDesktopInfo, WindowInfo},
};
use nwd::NwgPartial;
use std::{
    any::TypeId,
    cell::OnceCell,
    ops::Deref,
    rc::Rc,
    sync::{mpsc, Arc},
    thread::JoinHandle,
};

#[derive(Debug)]
enum BackgroundAction {
    ApplyFilters {
        filters: Arc<[WindowFilter]>,
        stop_flashing_windows: bool,
    },
    StopFlashingWindows,
}

struct ThreadInfo {
    join_handle: JoinHandle<()>,
    sender: mpsc::Sender<BackgroundAction>,
}
impl ThreadInfo {
    pub fn start() -> Self {
        let (tx, rx) = mpsc::channel::<BackgroundAction>();
        let join_handle = std::thread::Builder::new()
            .name("ApplyFiltersThread".to_owned())
            .spawn(move || Self::background_work(rx))
            .expect("should be able to spawn thread for applying window filters/rules");
        Self {
            join_handle,
            sender: tx,
        }
    }
    fn background_work(rx: mpsc::Receiver<BackgroundAction>) {
        if vd::has_loaded_dynamic_library_successfully() {
            // Old .dll files might not call `CoInitialize` and then not work,
            // so to be safe we make sure to do that:
            if let Err(e) = unsafe { windows::Win32::System::Com::CoInitialize(None) }.ok() {
                tracing::warn!(
                    error = e.to_string(),
                    "Failed to call CoInitialize on ApplyFiltersThread"
                );
            }
        }
        'outer: while let Ok(latest_action) = rx.recv() {
            let mut filters_to_apply = None;
            let mut stop_flashing = false;
            let mut stop_flashing_globally = false;
            let mut queue_action = |action| match action {
                BackgroundAction::ApplyFilters {
                    filters,
                    stop_flashing_windows,
                } => {
                    filters_to_apply = Some(filters);
                    stop_flashing |= stop_flashing_windows;
                }
                BackgroundAction::StopFlashingWindows => stop_flashing_globally = true,
            };
            queue_action(latest_action);
            loop {
                match rx.try_recv() {
                    // Only apply the latest filter list:
                    Ok(action) => queue_action(action),
                    // No more queued filter lists to apply:
                    Err(mpsc::TryRecvError::Empty) => break,
                    // The program has exited so don't apply the latest queued action:
                    Err(mpsc::TryRecvError::Disconnected) => break 'outer,
                }
            }
            let windows = WindowInfo::get_all();
            let mut windows_to_prevent_flashing =
                Vec::with_capacity(if stop_flashing || stop_flashing_globally {
                    windows.len()
                } else {
                    0
                });
            for (ix, window) in windows.into_iter().enumerate() {
                if stop_flashing_globally {
                    windows_to_prevent_flashing.push((
                        window.handle,
                        if let VirtualDesktopInfo::AtDesktop { desktop, .. } =
                            window.virtual_desktop
                        {
                            Some(desktop)
                        } else {
                            None
                        },
                    ))
                }
                let Some(filter_list) = &filters_to_apply else {
                    continue;
                };
                let Some(action_info) =
                    WindowFilter::find_first_action(filter_list, ix as i32, &window)
                else {
                    continue;
                };

                if window.virtual_desktop.is_app_pinned() {
                    // Don't interact with process that have all of their windows pinned.
                    continue;
                }

                let mut move_to_target_desktop = || {
                    let Ok(target_desktop_zero_based) = u32::try_from(action_info.target_desktop)
                    else {
                        tracing::error!(info =? action_info, "Tried to target a desktop outside the range of u32");
                        return;
                    };
                    if let VirtualDesktopInfo::AtDesktop { index, .. } = window.virtual_desktop {
                        let target = vd::get_desktop(target_desktop_zero_based);
                        if stop_flashing_globally {
                            windows_to_prevent_flashing.last_mut().unwrap().1 = Some(target);
                        } else if index == target_desktop_zero_based {
                            // Already at wanted desktop
                        } else if stop_flashing {
                            windows_to_prevent_flashing.push((window.handle, Some(target)));
                        } else if let Err(e) = vd::move_window_to_desktop(target, &window.handle) {
                            tracing::warn!(error = ?e, "Failed to move window to target desktop");
                        }
                    }
                };
                let unpin_window = || {
                    if window.virtual_desktop.is_window_pinned() {
                        if let Err(e) = vd::unpin_window(window.handle) {
                            tracing::warn!(error = ?e, "Failed to unpin window");
                            return false;
                        }
                    }
                    true
                };
                let stop_flashing_without_move = |windows_to_prevent_flashing: &mut Vec<(_, _)>| {
                    if stop_flashing_globally {
                        windows_to_prevent_flashing.last_mut().unwrap().1 = None;
                    } else if stop_flashing {
                        windows_to_prevent_flashing.push((window.handle, None));
                    }
                };

                match action_info.action {
                    FilterAction::Move => move_to_target_desktop(),
                    FilterAction::UnpinAndMove => {
                        if unpin_window() {
                            move_to_target_desktop();
                        }
                    }
                    FilterAction::Unpin => {
                        unpin_window();
                        stop_flashing_without_move(&mut windows_to_prevent_flashing);
                    }
                    FilterAction::Pin => {
                        if window.virtual_desktop.is_at_desktop() {
                            if let Err(e) = vd::pin_window(window.handle) {
                                tracing::warn!(error = ?e, "Failed to pin window");
                            }
                        }
                        stop_flashing_without_move(&mut windows_to_prevent_flashing);
                    }
                    FilterAction::Nothing | FilterAction::Disabled => {}
                }
            }

            if let Err(e) = vd::stop_flashing_windows_blocking(windows_to_prevent_flashing) {
                tracing::error!(
                    error = e.to_string(),
                    globally = stop_flashing_globally,
                    "Failed to prevent windows from flashing"
                );
            }
        }
        tracing::info!("ApplyFilters thread exited since the original was dropped");
    }
}
#[derive(Default)]
struct LazyThreadInfo(OnceCell<ThreadInfo>);
impl Drop for LazyThreadInfo {
    fn drop(&mut self) {
        let Some(inner) = self.0.take() else {
            return;
        };
        // Notify background thread to exit:
        drop(inner.sender);
        // Wait for background thread:
        let _ = inner.join_handle.join();
    }
}
impl Deref for LazyThreadInfo {
    type Target = ThreadInfo;

    fn deref(&self) -> &Self::Target {
        self.0.get_or_init(ThreadInfo::start)
    }
}

/// Apply filters on a background thread.
#[derive(Default, NwgPartial)]
pub struct ApplyFilters {
    background: LazyThreadInfo,
}
impl DynamicUiHooks<SystemTray> for ApplyFilters {
    fn before_partial_build(
        &mut self,
        _tray_ui: &Rc<SystemTray>,
        _should_build: &mut bool,
    ) -> Option<(nwg::ControlHandle, TypeId)> {
        None
    }
}
impl TrayPlugin for ApplyFilters {}
impl ApplyFilters {
    pub fn apply_filters(&self, filters: Arc<[WindowFilter]>, stop_flashing_windows: bool) {
        self.background
            .sender
            .send(BackgroundAction::ApplyFilters {
                filters,
                stop_flashing_windows,
            })
            .expect("send work to ApplyFilter thread");
    }
    pub fn stop_all_flashing_windows(&self) {
        self.background
            .sender
            .send(BackgroundAction::StopFlashingWindows)
            .expect("send work to ApplyFilter thread");
    }
}
