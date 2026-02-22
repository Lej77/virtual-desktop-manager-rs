#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(all(feature = "config_window_native", not(feature = "config_window_gpui")))]
type ConfigWindow = vdm_gui::ConfigWindow;

#[cfg(feature = "config_window_gpui")]
type ConfigWindow = vdm_gui_gpui::ConfigWindow;

fn main() {
    vdm_core::run_gui::<ConfigWindow>();
}
