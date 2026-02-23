#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

cfg_if::cfg_if! {
    if #[cfg(feature="config_window_gpui")] {
        type ConfigWindow = vdm_gui_gpui::ConfigWindow;
    } else if #[cfg(feature="config_window_egui")] {
        type ConfigWindow = vdm_gui_egui::ConfigWindow;
    } else if #[cfg(feature="config_window_winsafe")] {
        type ConfigWindow = vdm_gui_winsafe::ConfigWindow;
    } else if #[cfg(feature = "config_window_native")] {
        type ConfigWindow = vdm_gui::ConfigWindow;
    } else {
        compile_error!("Must enable at least one of the \"config_window_\" features");
    }
}

fn main() {
    vdm_core::run_gui::<ConfigWindow>();
}
