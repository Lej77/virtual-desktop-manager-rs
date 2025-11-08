use crate::{
    dynamic_gui::DynamicUiHooks,
    tray::{SystemTray, SystemTrayRef, TrayPlugin, TrayRoot},
    window_filter::WindowFilter,
};
#[cfg(feature = "persist_settings")]
use serde::{Deserialize, Deserializer, Serialize};
use std::{
    any::TypeId,
    cell::Cell,
    collections::BTreeMap,
    fmt,
    ops::Deref,
    path::Path,
    rc::Rc,
    sync::{Arc, Condvar, Mutex},
};
#[cfg(feature = "persist_settings")]
use std::{
    cell::OnceCell,
    io::{ErrorKind::NotFound, Write},
    sync::{mpsc, MutexGuard},
    time::Duration,
};

/// Use a default value if serialization fails for a field.
///
/// # References
///
/// Inspired by:
///
/// [\[Solved\] Serde deserialization on_error use default values? - help - The
/// Rust Programming Language
/// Forum](https://users.rust-lang.org/t/solved-serde-deserialization-on-error-use-default-values/6681)
#[cfg(feature = "persist_settings")]
fn ok_or_none<'de, T, D>(deserializer: D) -> Result<Option<T>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    let v: serde_json::Value = Deserialize::deserialize(deserializer)?;
    Ok(T::deserialize(v).ok())
}

/// Provide a deserialization for `UiSettings` that can handle malformed fields.
macro_rules! default_deserialize {
    (@inner
        $(#[$ty_attr:meta])*
        $ty_vis:vis struct $name:ident { $(
            $(#[$field_attr:meta])*
            $field_vis:vis $field_name:ident: $field_ty:ty
        ,)* $(,)? }
    ) => {
        #[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug)]
        #[cfg_attr(feature = "persist_settings", derive(Serialize, Deserialize))]
        pub struct UiSettingsFallback { $(
            $(#[$field_attr])*
            #[cfg_attr(feature = "persist_settings", serde(deserialize_with = "ok_or_none"))] // None if deserialization failed
            #[cfg_attr(feature = "persist_settings", serde(default))] // None if field isn't present
            $field_vis $field_name: Option<$field_ty>,
        )* }
        impl UiSettingsFallback {
            /// `true` if all fields have values and so all errors have been fixed.
            pub fn has_all_fields(&self) -> bool {
                $(
                    self.$field_name.is_some()
                )&&*
            }
        }
        impl From<UiSettingsFallback> for $name {
            fn from(value: UiSettingsFallback) -> Self {
                let mut this = <Self as Default>::default();
                $(
                    if let Some($field_name) = value.$field_name {
                        this.$field_name = $field_name;
                    }
                )*
                this
            }
        }
    };
    ($($token:tt)*) => {
        $($token)*
        default_deserialize! { @inner $($token)* }
    };
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Default, Debug)]
#[cfg_attr(feature = "persist_settings", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "persist_settings", serde(rename_all = "lowercase"))]
#[allow(dead_code)]
pub enum AutoStart {
    #[default]
    Disabled,
    Enabled,
    Elevated,
}
impl AutoStart {
    pub const ALL: &'static [Self] = &[
        Self::Disabled,
        // TODO: Add support for auto start without admin rights
        // Self::Enabled,
        Self::Elevated,
    ];
}
/// Used to display options in config window.
impl fmt::Display for AutoStart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match *self {
            AutoStart::Disabled => "No",
            AutoStart::Enabled => "Yes",
            AutoStart::Elevated => "Yes, with admin rights",
        };
        f.write_str(text)
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Default, Debug)]
#[cfg_attr(feature = "persist_settings", derive(Serialize, Deserialize))]
#[allow(dead_code)]
pub enum QuickSwitchMenu {
    Disabled,
    TopMenu,
    #[default]
    SubMenu,
}
impl QuickSwitchMenu {
    pub const ALL: &'static [Self] = &[Self::Disabled, Self::TopMenu, Self::SubMenu];
}
/// Used to display options in config window.
impl fmt::Display for QuickSwitchMenu {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match *self {
            QuickSwitchMenu::Disabled => "Off",
            QuickSwitchMenu::TopMenu => "Inside the main context menu",
            QuickSwitchMenu::SubMenu => "Inside a submenu",
        };
        f.write_str(text)
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Default, Debug)]
#[cfg_attr(feature = "persist_settings", derive(Serialize, Deserialize))]
#[allow(dead_code)]
pub enum TrayIconType {
    /// Show an icon that has a frame around the desktop index.
    ///
    /// This allows using hardcoded icons for low desktop indexes if available
    /// which should have higher quality and be faster to load.
    #[default]
    WithBackground,
    /// Show an icon that has a frame around the desktop index, but don't use
    /// hardcoded icons for low desktop indexes.
    WithBackgroundNoHardcoded,
    /// Show an icon with only a desktop index and no frame or anything else.
    /// The desktop index is rendered using the `imageproc` crate.
    NoBackground,
    /// Show an icon with only a desktop index and no frame or anything else.
    /// The desktop index is rendered using the `text_to_png` crate.
    NoBackground2,
    /// Show the same icon as the executable.
    AppIcon,
}
impl TrayIconType {
    pub const ALL: &'static [Self] = &[
        #[cfg(feature = "tray_icon_hardcoded")]
        Self::WithBackground,
        #[cfg(feature = "tray_icon_with_background")]
        Self::WithBackgroundNoHardcoded,
        #[cfg(feature = "tray_icon_text_only")]
        Self::NoBackground,
        #[cfg(feature = "tray_icon_text_only_alt")]
        Self::NoBackground2,
        Self::AppIcon,
    ];
}
/// Used to display options in config window.
impl fmt::Display for TrayIconType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match *self {
            TrayIconType::WithBackground => "Hardcoded number inside icon",
            TrayIconType::WithBackgroundNoHardcoded => "Generated number inside icon",
            TrayIconType::NoBackground => "Only black and white number",
            TrayIconType::NoBackground2 => "Only purple number",
            TrayIconType::AppIcon => "Only program icon, no number",
        };
        f.write_str(text)
    }
}

#[derive(Default, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Debug)]
#[cfg_attr(feature = "persist_settings", derive(Serialize, Deserialize))]
#[allow(dead_code)]
pub enum TrayClickAction {
    #[default]
    Disabled,
    StopFlashingWindows,
    ToggleConfigurationWindow,
    ApplyFilters,
    OpenContextMenu,
}
impl TrayClickAction {
    pub const ALL: &'static [Self] = &[
        Self::Disabled,
        Self::StopFlashingWindows,
        Self::ToggleConfigurationWindow,
        Self::ApplyFilters,
        Self::OpenContextMenu,
    ];
}
/// Used to display options in config window.
impl fmt::Display for TrayClickAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match *self {
            Self::Disabled => "Disabled",
            Self::StopFlashingWindows => "Stop Flashing Windows",
            Self::ToggleConfigurationWindow => "Open/Close Config Window",
            Self::ApplyFilters => "Apply Filters",
            Self::OpenContextMenu => "Open Context Menu",
        };
        f.write_str(text)
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Debug)]
#[cfg_attr(feature = "persist_settings", derive(Serialize, Deserialize))]
pub struct ConfigWindowInfo {
    pub position: Option<(i32, i32)>,
    pub size: (u32, u32),
    pub maximized: bool,
}
impl Default for ConfigWindowInfo {
    fn default() -> Self {
        Self {
            position: None,
            size: (800, 600),
            maximized: false,
        }
    }
}

default_deserialize!(
    #[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug)]
    #[cfg_attr(feature = "persist_settings", derive(Serialize, Deserialize))]
    pub struct UiSettings {
        pub version: u64,
        /// Autostart this program when Windows is started.
        pub auto_start: AutoStart,
        /// If this is enabled then we will attempt to switch virtual desktops
        /// using animations. This is done by opening a transparent window on a
        /// different virtual desktop and then focusing on it.
        pub smooth_switch_desktops: bool,
        /// Elevated permission is useful to move windows owned by elevated
        /// programs. If this setting is enabled the program will ask for admin
        /// rights every time it is started, making it easy to always have them.
        pub request_admin_at_startup: bool,
        /// Some windows might be trying to grab the user's attention and will
        /// be flashing in the taskbar. These taskbar items will remain visible
        /// even if the window is moved to another virtual desktop. If this
        /// setting is enabled we will attempt to stop windows from flashing in
        /// the taskbar after moving them.
        pub stop_flashing_windows_after_applying_filter: bool,
        /// The type of icon to show in the system tray.
        pub tray_icon_type: TrayIconType,
        /// Fancy context menu items that allows switching to a desktop by
        /// entering its one-based index via context menu keyboard shortcuts.
        pub quick_switch_menu: QuickSwitchMenu,
        /// Extra context menu items that have custom access keys to allow fast
        /// switching to specific desktops. Usually this is used if you have
        /// more than 9 desktops because then pressing `1` could be interpreted
        /// as the start of `10` and so it is useful to have another key that
        /// brings you to the first desktop.
        pub quick_switch_menu_shortcuts: Arc<BTreeMap<String, u32>>,
        /// Determines if the extra shortcut menu items should be shown even in
        /// submenus of the quick switch menu. Usually it is enough to only have
        /// them in the top most "quick switch" context menu.
        pub quick_switch_menu_shortcuts_only_in_root: bool,

        /// Global keyboard shortcut for opening the quick switch menu. Will be
        /// parsed as a [`global_hotkey::hotkey::HotKey`].
        pub quick_switch_hotkey: Arc<str>,

        /// Global keyboard shortcut for opening the context menu at the mouse's
        /// current position. Quite useful when the keyboard shortcut is used by
        /// a macro triggered by a mouse button.
        pub open_menu_at_mouse_pos_hotkey: Arc<str>,

        pub left_click: TrayClickAction,
        /// Middle clicks are registered as left clicks for at least some
        /// versions of Windows 11.
        pub middle_click: TrayClickAction,

        /// Info about last location of the configuration window.
        pub config_window: ConfigWindowInfo,
        /// Filters/rules that specify which windows should be moved and to what
        /// virtual desktop.
        pub filters: Arc<[WindowFilter]>,
    }
);
impl UiSettings {
    const CURRENT_VERSION: u64 = 2;

    /// Ensure settings are the newest version. Some work might have been done
    /// previously by [`UiSettingsFallback::maybe_migrate`] if initial parsing
    /// failed.
    fn migrate(&mut self) {
        // Always change the version to latest, since if we save the data this
        // is the version that will be written:
        self.version = Self::CURRENT_VERSION;
    }
}
impl UiSettingsFallback {
    /// Handle some migrations to newer setting formats. If all errors could be
    /// explained by version mismatch then returns `true`.
    fn maybe_migrate(&mut self) -> bool {
        if self.open_menu_at_mouse_pos_hotkey.is_none() && matches!(self.version, Some(v) if v <= 1) {
            self.open_menu_at_mouse_pos_hotkey = Some(Arc::from(""));
        }
        self.has_all_fields()
    }
}
impl Default for UiSettings {
    fn default() -> Self {
        Self {
            version: Self::CURRENT_VERSION,
            auto_start: AutoStart::default(),
            smooth_switch_desktops: true,
            request_admin_at_startup: false,
            stop_flashing_windows_after_applying_filter: false,
            tray_icon_type: TrayIconType::default(),
            quick_switch_menu: QuickSwitchMenu::default(),
            quick_switch_menu_shortcuts: Arc::new(BTreeMap::from([
                // Useful when using the numpad:
                (",".to_owned(), 0),
            ])),
            quick_switch_menu_shortcuts_only_in_root: false,
            quick_switch_hotkey: Arc::from(""),
            open_menu_at_mouse_pos_hotkey: Arc::from(""),

            left_click: TrayClickAction::ToggleConfigurationWindow,
            middle_click: TrayClickAction::ApplyFilters,

            config_window: ConfigWindowInfo::default(),
            filters: Arc::new([]),
        }
    }
}

#[cfg(feature = "persist_settings")]
struct UiState {
    error_notice: nwg::NoticeSender,
    error_tx: mpsc::Sender<String>,
    thread_join: std::thread::JoinHandle<()>,
}

struct UiSettingsPluginState {
    settings: Arc<UiSettings>,
    #[cfg(feature = "persist_settings")]
    settings_in_file: Arc<UiSettings>,
    save_path: Option<Arc<Path>>,
    temp_save_path: Option<Arc<Path>>,
    #[cfg(feature = "persist_settings")]
    should_close: bool,
    #[cfg(feature = "persist_settings")]
    ui_state: Option<UiState>,
}
impl Default for UiSettingsPluginState {
    fn default() -> Self {
        let settings = Arc::new(UiSettings::default());
        #[cfg(feature = "persist_settings")]
        let settings_in_file = Arc::clone(&settings);
        Self {
            settings,
            #[cfg(feature = "persist_settings")]
            settings_in_file,
            #[cfg(feature = "persist_settings")]
            should_close: false,
            #[cfg(feature = "persist_settings")]
            ui_state: None,
            save_path: None,
            temp_save_path: None,
        }
    }
}

#[derive(Default)]
struct UiSettingsPluginShared {
    state: Mutex<UiSettingsPluginState>,
    /// Background thread waits on this when it is allowed to save settings.
    notify_change: Condvar,
    /// Background thread waits on this when its not allowed to save settings
    /// for a while.
    #[cfg(feature = "persist_settings")]
    notify_close: Condvar,
}
#[cfg(feature = "persist_settings")]
impl UiSettingsPluginShared {
    fn close_background_thread(this: &Self, mut guard: MutexGuard<UiSettingsPluginState>) {
        guard.should_close = true;
        let ui_state = guard.ui_state.take();
        drop(guard);
        this.notify_change.notify_all();
        this.notify_close.notify_all();
        if let Some(ui_state) = ui_state {
            ui_state.thread_join.join().unwrap();
        }
    }
    fn start_background_work(
        self: &Arc<Self>,
        error_notice: nwg::NoticeSender,
        error_tx: mpsc::Sender<String>,
    ) {
        let mut guard = self.state.lock().unwrap();

        // Finish closing threads:
        while guard.should_close && guard.ui_state.is_some() {
            Self::close_background_thread(self, guard);
            guard = self.state.lock().unwrap();
        }
        guard.should_close = false;

        // If there is a running thread then don't start a new one:
        if let Some(ui_state) = &mut guard.ui_state {
            ui_state.error_notice = error_notice;
            ui_state.error_tx = error_tx;
            return;
        }

        // Start new background thread:
        let thread_join = std::thread::Builder::new()
            .name("UiSettingsSaveThread".to_owned())
            .spawn({
                let shared = Arc::clone(self);
                move || shared.background_work()
            })
            .expect("Failed to spawn thread for saving UI settings");
        guard.ui_state = Some(UiState {
            error_notice,
            error_tx,
            thread_join,
        });
    }
    fn background_work(self: Arc<Self>) {
        let mut guard = self.state.lock().unwrap();
        let mut latest_saved;
        while !guard.should_close {
            latest_saved = Arc::clone(&guard.settings);
            let result = self.save_settings_inner(guard);
            guard = self.state.lock().unwrap();
            if guard.should_close {
                return;
            }
            match result {
                Ok(true) => {
                    // Saved data => wait a while before we try saving again:
                    guard = self
                        .notify_close
                        .wait_timeout(guard, Duration::from_millis(1000))
                        .unwrap()
                        .0;
                    if guard.should_close {
                        return;
                    }
                }
                Ok(false) => {
                    // No changes => wait until new settings data is specified
                }
                Err(e) => {
                    tracing::error!(?e, "Failed to save UI settings");
                    if let Some(ui_state) = &guard.ui_state {
                        if let Err(e) = ui_state.error_tx.send(e) {
                            tracing::warn!(error = ?e, "Failed to send UiSettings save error to UI thread");
                        }
                        ui_state.error_notice.notice();
                    }
                }
            }

            // Wait until new settings data is specified:
            if Arc::ptr_eq(&latest_saved, &guard.settings) {
                guard = self.notify_change.wait(guard).unwrap();
                if guard.should_close {
                    return;
                }
            }

            // Attempt to batch save changes by waiting a little before saving:
            guard = self
                .notify_close
                .wait_timeout(guard, Duration::from_millis(50))
                .unwrap()
                .0;
        }
    }

    fn save_settings_inner(
        &self,
        mut guard: MutexGuard<UiSettingsPluginState>,
    ) -> Result<bool, String> {
        if Arc::ptr_eq(&guard.settings, &guard.settings_in_file) {
            return Ok(false);
        }
        if guard.settings == guard.settings_in_file {
            // Ensure there is a single allocation for the UI settings:
            guard.settings_in_file = Arc::clone(&guard.settings);
            return Ok(false);
        }
        let new_data = guard.settings.clone();

        let Some(save_path) = guard.save_path.clone() else {
            tracing::warn!("Can't save settings since there was no save path");
            return Ok(false);
        };
        let Some(temp_path) = guard.temp_save_path.clone() else {
            tracing::warn!("Can't save settings since there was no temporary save path");
            return Ok(false);
        };
        // Don't hold lock during slow operations:
        drop(guard);

        tracing::trace!(?save_path, ?temp_path, ?new_data, "Saving UI settings");

        let binary_data = serde_json::to_vec_pretty(&*new_data)
            .map_err(|e| format!("Failed to serialize UI settings: {e}"))?;

        match std::fs::remove_file(&temp_path) {
            Ok(_) => {}
            Err(e) if e.kind() == NotFound => {}
            Err(e) => {
                return Err(format!("Failed to remove temp ui settings: {e}"));
            }
        }

        {
            let mut file = std::fs::OpenOptions::new()
                .create_new(true)
                .truncate(true)
                .write(true)
                .open(&temp_path)
                .map_err(|e| format!("Failed to create new UI settings file: {e}"))?;

            file.write_all(&binary_data)
                .map_err(|e| format!("Failed to write UI settings to file: {e}"))?;

            file.flush()
                .map_err(|e| format!("Failed to flush UI settings to file: {e}"))?;
        }

        std::fs::rename(&temp_path, &save_path)
            .map_err(|e| format!("Failed to rename new UI settings file: {e}"))?;

        let mut guard = self.state.lock().unwrap();
        guard.settings_in_file = new_data;

        Ok(true)
    }
}

#[derive(Default)]
struct UiSettingsPluginSharedStrong(Arc<UiSettingsPluginShared>);
#[cfg(feature = "persist_settings")]
impl Drop for UiSettingsPluginSharedStrong {
    fn drop(&mut self) {
        if let Ok(guard) = self.0.state.lock() {
            UiSettingsPluginShared::close_background_thread(&self.0, guard);
        }
    }
}
impl Deref for UiSettingsPluginSharedStrong {
    type Target = Arc<UiSettingsPluginShared>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// This plugin tracks UI settings.
#[derive(nwd::NwgPartial, Default)]
pub struct UiSettingsPlugin {
    tray_ui: SystemTrayRef,
    #[cfg(feature = "persist_settings")]
    #[nwg_control]
    #[nwg_events(OnNotice: [Self::on_background_error])]
    error_notice: nwg::Notice,
    #[cfg(feature = "persist_settings")]
    error_rx: OnceCell<mpsc::Receiver<String>>,
    load_error: Cell<Option<String>>,
    shared: UiSettingsPluginSharedStrong,
}
impl UiSettingsPlugin {
    pub fn get(&self) -> Arc<UiSettings> {
        Arc::clone(&self.shared.state.lock().unwrap().settings)
    }
    pub fn set(&self, value: UiSettings) {
        let new;
        let prev = {
            let mut state = self.shared.state.lock().unwrap();
            if *state.settings == value {
                return;
            }
            new = Arc::new(value);
            let prev = std::mem::replace(&mut state.settings, Arc::clone(&new));
            self.shared.notify_change.notify_all();
            prev
        };
        if let Some(tray) = self.tray_ui.get() {
            tray.notify_settings_changed(&prev, &new);
        }
    }
    pub fn update(&self, f: impl FnOnce(&UiSettings) -> UiSettings) {
        let current = self.get();
        let new = f(&current);
        drop(current);
        self.set(new);
    }

    pub fn with_save_path_next_to_exe() -> Self {
        let mut this = Self::default();
        this.set_save_path_next_to_exe();
        this
    }
    pub fn set_save_path_next_to_exe(&mut self) {
        let exe_path = match std::env::current_exe() {
            Ok(v) => v,
            Err(e) => {
                self.load_error.set(Some(format!(
                    "Failed to find UI settings file, can't get executable's path: {e}"
                )));
                return;
            }
        };
        {
            let mut guard = self.shared.state.lock().unwrap();
            guard.save_path = Some(Arc::from(exe_path.with_extension("settings.json")));
            guard.temp_save_path = Some(Arc::from(exe_path.with_extension("settings.temp.json")));
        }
        self.load_data();
    }
    pub fn load_data(&self) {
        #[cfg(feature = "persist_settings")]
        {
            let Some(save_path) = self.shared.state.lock().unwrap().save_path.clone() else {
                return;
            };
            let (settings, load_error) = match std::fs::read_to_string(&save_path) {
                Ok(data) => {
                    let mut deserializer = serde_json::Deserializer::from_str(&data);
                    let result: Result<UiSettings, _> = {
                        #[cfg(not(feature = "serde_path_to_error"))]
                        {
                            serde::Deserialize::deserialize(&mut deserializer)
                        }
                        #[cfg(feature = "serde_path_to_error")]
                        {
                            serde_path_to_error::deserialize(&mut deserializer)
                        }
                    };
                    match result {
                        Ok(settings) => (Some(settings), None),
                        Err(e) => {
                            let mut ignore_error = false;
                            (
                            // Try to be more lenient when parsing (skip parsing for
                            // fields that fail and use default values for those):
                            serde_json::from_str::<UiSettingsFallback>(&data)
                                .ok()
                                .map(|mut fallback| {
                                    ignore_error = fallback.maybe_migrate();
                                    UiSettings::from(fallback)
                                }),
                            // Emit an error message for why the strict parsing failed:
                            Some(format!(
                                "Could not parse UI settings file as JSON: {e}: Settings file at \"{}\"",
                                save_path.display()
                            )).filter(|_| !ignore_error),
                        )
                        }
                    }
                }
                Err(e) if e.kind() == NotFound => {
                    tracing::trace!(
                        "Using default settings since no UI settings file was found at \"{}\"",
                        save_path.display()
                    );
                    (None, None)
                }
                Err(e) => (
                    None,
                    Some(format!(
                        "Failed to read UI settings file: {e}: Settings file at \"{}\"",
                        save_path.display()
                    )),
                ),
            };
            // Notify error:
            if let Some(error) = load_error {
                if let Some(tray) = self.tray_ui.get() {
                    Self::notify_load_error(&tray, &error)
                } else {
                    self.load_error.set(Some(error));
                }
            }
            // Update tracked settings:
            if let Some(mut settings) = settings {
                settings.migrate();
                let new = Arc::new(settings);
                let prev = {
                    let mut state = self.shared.state.lock().unwrap();
                    state.settings_in_file = Arc::clone(&new);
                    std::mem::replace(&mut state.settings, Arc::clone(&new))
                };
                if let Some(tray) = self.tray_ui.get() {
                    tray.notify_settings_changed(&prev, &new);
                }
            }
        }
    }
    fn notify_load_error(tray_ui: &SystemTray, error: &str) {
        tray_ui.show_notification("Virtual Desktop Manager Error", error);
    }
    #[cfg(feature = "persist_settings")]
    fn on_background_error(&self) {
        let Some(error_rx) = self.error_rx.get() else {
            return;
        };
        let Some(dynamic_ui) = self.tray_ui.get() else {
            return;
        };
        for error in error_rx.try_iter() {
            dynamic_ui.show_notification("Virtual Desktop Manager Error", &error);
        }
    }
}
impl DynamicUiHooks<SystemTray> for UiSettingsPlugin {
    fn before_partial_build(
        &mut self,
        tray_ui: &Rc<SystemTray>,
        _should_build: &mut bool,
    ) -> Option<(nwg::ControlHandle, TypeId)> {
        self.tray_ui.set(tray_ui);
        Some((tray_ui.root().window.handle, TypeId::of::<TrayRoot>()))
    }
    fn after_partial_build(&mut self, tray_ui: &Rc<SystemTray>) {
        if let Some(error) = self.load_error.take() {
            Self::notify_load_error(tray_ui, &error);
        }

        #[cfg(feature = "persist_settings")]
        {
            let (tx, rx) = mpsc::channel();
            self.shared
                .start_background_work(self.error_notice.sender(), tx);
            if self.error_rx.set(rx).is_err() {
                tracing::error!("Failed to set new error receiver for UiSettingsPlugin");
            }
        }
    }
    fn before_rebuild(&mut self, _dynamic_ui: &Rc<SystemTray>) {
        self.tray_ui = Default::default();
        #[cfg(feature = "persist_settings")]
        {
            self.error_notice = Default::default();
            self.error_rx = OnceCell::new();
        }
    }
}
impl TrayPlugin for UiSettingsPlugin {}
