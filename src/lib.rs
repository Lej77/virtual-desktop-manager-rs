//! This library implements various ideas for helping with Windows Virtual
//! Desktops. For similar projects check out [Virtual Desktop Manager · Issue
//! #343 ·
//! microsoft/PowerToys](https://github.com/microsoft/PowerToys/issues/343).

// Note: can't do this renaming in Cargo.toml since the derive macros rely on
// the package name being `native_windows_gui`.
extern crate native_windows_derive as nwd;
extern crate native_windows_gui as nwg;

#[cfg(feature = "auto_start")]
mod auto_start;
pub mod block_on;
#[cfg(feature = "admin_startup")]
mod change_elevation;
mod config_window;
pub mod dynamic_gui;
mod invisible_window;
pub mod nwg_ext;
mod quick_switch;
mod settings;
mod tray;
mod tray_icons;
pub mod vd;
mod window_filter;
pub mod window_info;
#[cfg(all(feature = "logging", debug_assertions))]
mod wm_msg_to_string;
mod tray_plugins {
    pub mod apply_filters;
    pub mod desktop_events;
    pub mod desktop_events_dynamic;
    pub mod hotkeys;
    pub mod menus;
    pub mod panic_notifier;
}

/// Get a reference to the executable's embedded icon.
fn exe_icon() -> Option<std::rc::Rc<nwg::Icon>> {
    use std::{cell::OnceCell, rc::Rc};

    thread_local! {
        static CACHE: OnceCell<Option<Rc<nwg::Icon>>> = const { OnceCell::new() };
    }
    CACHE.with(|cache| {
        cache
            .get_or_init(|| {
                nwg::EmbedResource::load(None)
                    .unwrap()
                    .icon(1, None)
                    .map(Rc::new)
            })
            .as_ref()
            .cloned()
    })
}

#[cfg(all(feature = "logging", debug_assertions))]
fn setup_logging() {
    // Set the global logger for the `log` crate:
    ::tracing_log::LogTracer::init().expect("setting global logger");

    let my_subscriber = ::tracing_subscriber::fmt::SubscriberBuilder::default()
        .pretty()
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stdout()))
        .with_max_level(tracing::Level::TRACE)
        .finish();
    tracing::subscriber::set_global_default(my_subscriber).expect("setting tracing default failed");

    tracing::debug!("Configured global logger");

    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        prev(info);
        tracing::error!("Panic: {}", info);
    }));
}

fn register_panic_hook_that_writes_to_file() {
    static CREATED_LOG: std::sync::Mutex<bool> = std::sync::Mutex::new(false);
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        prev(info);
        let Ok(exe_path) = std::env::current_exe() else {
            return;
        };
        let log_file = exe_path.with_extension("panic-log.txt");
        let Ok(mut created_log_guard) = CREATED_LOG.lock() else {
            return;
        };
        let mut open_options = std::fs::OpenOptions::new();
        let has_previous_panic = *created_log_guard;
        if has_previous_panic {
            open_options.create(true).append(true);
        } else {
            open_options.create(true).write(true).truncate(true);
        }
        let Ok(mut file) = open_options.open(log_file) else {
            return;
        };
        *created_log_guard = true;

        use std::io::Write;
        let _ = write!(
            file,
            "{}{}",
            if has_previous_panic { "\n\n\n\n" } else { "" },
            info
        );
    }));
}

#[cfg(feature = "cli_commands")]
#[derive(clap::Parser, Debug)]
#[command(version, about)]
enum Args {
    /// Switch to another virtual desktop.
    Switch {
        /// The index of the desktop to switch to.
        #[clap(required_unless_present_any(["next", "back"]))]
        target: Option<u32>,

        /// Switch to the desktop with an index one more than the current one.
        #[clap(long)]
        next: bool,

        /// Switch to the desktop with an index one less than the current one.
        #[clap(long)]
        back: bool,

        /// Smooth desktop switching using an animation instead of instant
        /// switch.
        #[clap(long)]
        smooth: bool,
    },
}

fn desktop_event_plugin() -> Box<dyn tray::TrayPlugin> {
    #[cfg(feature = "winvd_dynamic")]
    {
        if vd::has_loaded_dynamic_library_successfully() {
            tracing::info!("Using dynamic library to get virtual desktop events");
            return Box::<tray_plugins::desktop_events_dynamic::DynamicVirtualDesktopEventManager>::default(
            );
        }
    }
    #[cfg(feature = "winvd_static")]
    {
        tracing::info!("Using static library to get virtual desktop events");
        return Box::<tray_plugins::desktop_events::VirtualDesktopEventManager>::default();
    }
    #[allow(unreachable_code)]
    {
        panic!("Could not listen to virtual desktop events since no dynamic library was loaded");
    }
}

/// Start the GUI main loop and show the tray icon.
pub fn run_gui() {
    #[cfg(all(feature = "logging", debug_assertions))]
    setup_logging();
    register_panic_hook_that_writes_to_file();

    // Safety: "VirtualDesktopAccessor.dll" is well-behaved if it exists.
    if let Err(e) = unsafe { vd::load_dynamic_library() } {
        if nwg::init().is_ok() {
            nwg::error_message(
                "VirtualDesktopManager - Failed to load dynamic library",
                &e.to_string(),
            );
        }
        std::process::exit(3);
    }

    #[cfg(feature = "cli_commands")]
    if let Some(cmd) = std::env::args().nth(1) {
        use clap::{Parser, Subcommand};

        if Args::has_subcommand(&cmd) || cmd.contains("help") {
            let args = Args::try_parse().unwrap_or_else(|e| {
                if nwg::init().is_ok() {
                    nwg::error_message(
                        "Virtual Desktop Manager - Invalid CLI arguments",
                        &format!("{e}"),
                    );
                }
                std::process::exit(2);
            });
            std::thread::Builder::new()
                .name("CLI Command Executor".to_owned())
                .spawn(move || {
                    struct ExitGuard;
                    impl Drop for ExitGuard {
                        fn drop(&mut self) {
                            std::process::exit(1);
                        }
                    }
                    let _exit_guard = ExitGuard;

                    // Old .dll files might not call `CoInitialize` and then not work,
                    // so to be safe we make sure to do that:
                    if let Err(e) = unsafe { windows::Win32::System::Com::CoInitialize(None) }.ok()
                    {
                        tracing::warn!(
                            error = e.to_string(),
                            "Failed to call CoInitialize on CLI Command Executor thread"
                        );
                    }

                    match args {
                        Args::Switch {
                            target,
                            next,
                            back,
                            smooth,
                        } => {
                            let target = if let Some(target) = target {
                                // Ensure WinVD is initialized:
                                let _ = vd::get_current_desktop();
                                target
                            } else if next {
                                let count =
                                    vd::get_desktop_count().expect("Failed to get desktop count");
                                let current = vd::get_current_desktop()
                                    .expect("Failed to get current desktop");
                                let index = current
                                    .get_index()
                                    .expect("Failed to get index of current desktop");
                                (index + 1).min(count - 1)
                            } else if back {
                                let current = vd::get_current_desktop()
                                    .expect("Failed to get current desktop");
                                let index: u32 = current
                                    .get_index()
                                    .expect("Failed to get index of current desktop");
                                index.saturating_sub(1)
                            } else {
                                unreachable!("Clap should ensure a switch target is specified");
                            };
                            tracing::event!(
                                tracing::Level::INFO,
                                "Switching to desktop index {target}"
                            );
                            if smooth {
                                nwg::init().expect("Failed to init Native Windows GUI");
                                invisible_window::switch_desktop_with_invisible_window(
                                    vd::get_desktop(target),
                                    None,
                                )
                                .expect("Failed to smoothly switch desktop");
                            } else {
                                vd::switch_desktop(vd::Desktop::from(target))
                                    .expect("Failed to switch to target desktop");
                            }
                        }
                    }
                    std::process::exit(0);
                })
                .expect("Failed to spawn background thread to work on CLI command");

            // Start GUI event loop ASAP to prevent spinner next to mouse cursor:
            match nwg::init() {
                Ok(()) => {
                    nwg::dispatch_thread_events();
                }
                Err(e) => {
                    tracing::error!(error = ?e, "Failed to initialize gui");
                }
            }
            loop {
                std::thread::park();
            }
        }
    }

    let settings_plugin = Box::new(settings::UiSettingsPlugin::with_save_path_next_to_exe());

    #[cfg(feature = "admin_startup")]
    {
        let mut admin = change_elevation::AdminRestart;
        admin.handle_startup();
        if settings_plugin.get().request_admin_at_startup {
            if let Err(e) = change_elevation::set_elevation(&mut admin, true) {
                tracing::error!("Failed to request admin rights: {e}");
            }
        }
    }

    nwg::init().expect("Failed to init Native Windows GUI");
    nwg::Font::set_global_family("Segoe UI").expect("Failed to set default font");
    let _ui = tray::SystemTray::new(vec![
        Box::<tray_plugins::panic_notifier::PanicNotifier>::default(),
        Box::<tray_plugins::apply_filters::ApplyFilters>::default(),
        settings_plugin,
        #[cfg(feature = "global_hotkey")]
        Box::<tray_plugins::hotkeys::HotKeyPlugin>::default(),
        #[cfg(feature = "auto_start")]
        Box::<auto_start::AutoStartPlugin>::default(),
        desktop_event_plugin(),
        Box::<invisible_window::SmoothDesktopSwitcher>::default(),
        Box::<tray_plugins::menus::OpenSubmenuPlugin>::default(),
        Box::<tray_plugins::menus::TopMenuItems>::default(),
        Box::<tray_plugins::menus::BackspaceAsEscapeAlias>::default(),
        Box::<tray_plugins::menus::QuickSwitchTopMenu>::default(),
        Box::<tray_plugins::menus::QuickSwitchMenuUiAdapter>::default(),
        Box::<tray_plugins::menus::FlatSwitchMenu>::default(),
        Box::<tray_plugins::menus::BottomMenuItems>::default(),
        Box::<config_window::ConfigWindow>::default(),
    ])
    .build_ui()
    .expect("Failed to build UI");
    nwg::dispatch_thread_events();
}
