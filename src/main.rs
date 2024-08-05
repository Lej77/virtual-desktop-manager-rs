#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    virtual_desktop_manager::run_gui();
}
