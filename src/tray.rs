//! Defines the system tray and global program state.

use std::{
    any::TypeId,
    cell::{Cell, Ref, RefCell},
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use windows::Win32::Foundation::HWND;

use crate::{
    config_window::ConfigWindow,
    dynamic_gui::{DynamicUi, DynamicUiHooks, DynamicUiOwner, DynamicUiRef, DynamicUiWrapper},
    invisible_window::SmoothDesktopSwitcher,
    nwg_ext::{
        menu_index_in_parent, menu_item_index_in_parent, tray_get_rect, tray_set_version_4,
        windows_msg_for_explorer_restart, FastTimerControl, TrayWindow,
    },
    settings::UiSettings,
    vd,
};

/// Basic state used by the program.
#[derive(Default, nwd::NwgPartial)]
pub struct TrayRoot {
    tray_ui: SystemTrayRef,

    no_parent: crate::nwg_ext::ParentCapture,

    #[nwg_control(parent: no_parent)]
    pub window: TrayWindow,

    /// The initial icon for the tray.
    #[nwg_resource(source_embed: Some(&nwg::EmbedResource::load(None).unwrap()), source_embed_id: 2)]
    //#[nwg_resource(source_bin: Some(crate::tray_icons::ICON_EMPTY))]
    pub icon: nwg::Icon,

    #[nwg_control(parent: window, icon: Some(&data.icon), tip: Some("Virtual Desktop Manager"))]
    #[nwg_events(
        MousePressLeftUp: [Self::notify_tray_left_click],
        // Handled manually in process_raw_event:
        // OnContextMenu: [Self::show_menu]
    )]
    pub tray: nwg::TrayNotification,

    last_tray_key_event: Cell<Option<Instant>>,

    last_left_click: Cell<Option<Instant>>,

    #[nwg_control(parent: window, popup: true)]
    pub tray_menu: nwg::Menu,

    selected_tray_menu_item: Cell<Option<nwg::ControlHandle>>,

    /// The location where we last showed the context menu.
    last_menu_pos: Cell<Option<(i32, i32)>>,

    /// If the app is auto started with Windows then the taskbar might not exist
    /// when the program is started and if so we need to re-register our tray
    /// icon.
    #[nwg_control(parent: window)]
    #[nwg_events(OnNotice: [Self::notify_startup_rebuild])]
    rebuild_at_startup: FastTimerControl,
    /// The program was started at approximately this time.
    first_created_at: Option<Instant>,

    need_rebuild: Cell<bool>,
}
impl TrayRoot {
    pub fn notify_that_tray_icon_exists(&self) {
        self.rebuild_at_startup.cancel_last();
    }
    fn notify_tray_left_click(&self) {
        let Some(tray_ui) = self.tray_ui.get() else {
            return;
        };

        let now = Instant::now();
        if let Some(last_left_click) = self.last_left_click.replace(Some(now)) {
            if now.duration_since(last_left_click) < Duration::from_millis(300) {
                // Double click should have the same outcome as single click so
                // we ignore the second click.
                tracing::debug!("Ignored double left click event on tray icon");
                return;
            }
        }

        tray_ui.notify_tray_left_click();
    }

    fn notify_startup_rebuild(&self) {
        tracing::info!(
            "Rebuilding tray icon incase the taskbar didn't exist when the program was started"
        );
        self.need_rebuild.set(true);
    }

    #[allow(dead_code)]
    pub fn get_selected_tray_menu_item(&self) -> Option<nwg::ControlHandle> {
        self.selected_tray_menu_item.get()
    }

    pub fn update_tray_icon(&self, tray_ui: &Rc<SystemTray>, new_ix: u32) {
        use crate::{settings::TrayIconType, tray_icons::IconType};

        let icon_type = tray_ui.settings().get().tray_icon_type;
        let icon_generator = match icon_type {
            TrayIconType::WithBackground => IconType::WithBackground {
                allow_hardcoded: true,
                light_theme: tray_ui.has_light_taskbar(),
            },
            TrayIconType::WithBackgroundNoHardcoded => IconType::WithBackground {
                allow_hardcoded: false,
                light_theme: tray_ui.has_light_taskbar(),
            },
            TrayIconType::NoBackground => IconType::NoBackground {
                light_theme: tray_ui.has_light_taskbar(),
            },
            TrayIconType::NoBackground2 => IconType::NoBackgroundAlt,
            TrayIconType::AppIcon => {
                self.tray.set_icon(&self.icon);
                return;
            }
        };
        let icon_data = icon_generator.generate_icon(new_ix + 1);
        if let Ok(icon) = nwg::Icon::from_bin(&icon_data) {
            self.tray.set_icon(&icon);
        }
    }
}
impl DynamicUiHooks<SystemTray> for TrayRoot {
    fn before_partial_build(
        &mut self,
        _dynamic_ui: &Rc<SystemTray>,
        _should_build: &mut bool,
    ) -> Option<(nwg::ControlHandle, TypeId)> {
        None
    }

    fn after_partial_build(&mut self, tray_ui: &Rc<SystemTray>) {
        tracing::debug!(
            tray_window_handle = ?self.window.handle,
            "Created new tray window"
        );

        self.tray_ui.set(tray_ui);
        self.on_current_desktop_changed(tray_ui, tray_ui.desktop_index.get());

        // Modern context menu handling:
        //
        // Note: the program need to be DPI aware (see program manifest) in
        // order to get the right tray icon coordinates when opening context
        // menu.
        tray_set_version_4(&self.tray);

        // Ensure this runs at least once, otherwise the message is never registered:
        windows_msg_for_explorer_restart();

        // Rebuild tray later since the windows taskbar might not exist right
        // now (if Windows was just started):
        let first_created_at = *self.first_created_at.get_or_insert_with(Instant::now);
        let now = Instant::now();
        let rebuild_after = [30, 60, 90];
        for delay in rebuild_after {
            let rebuild_at = first_created_at + Duration::from_secs(delay);
            if rebuild_at > now {
                self.rebuild_at_startup.notify_at(rebuild_at);
                break;
            }
        }
    }

    fn after_handles<'a>(
        &'a self,
        _dynamic_ui: &Rc<SystemTray>,
        handles: &mut Vec<&'a nwg::ControlHandle>,
    ) {
        if handles.is_empty() {
            handles.push(&self.window.handle)
        }
    }

    fn after_process_events(
        &self,
        _dynamic_ui: &Rc<SystemTray>,
        evt: nwg::Event,
        _evt_data: &nwg::EventData,
        handle: nwg::ControlHandle,
        _window: nwg::ControlHandle,
    ) {
        match evt {
            nwg::Event::OnMenuEnter | nwg::Event::OnMenuExit | nwg::Event::OnMenuItemSelected => {
                self.selected_tray_menu_item.set(None)
            }

            // evt_data is None.
            // handle is hovered item.
            nwg::Event::OnMenuHover => self.selected_tray_menu_item.set(Some(handle)),

            _ => {}
        }
    }
    fn process_raw_event(
        &self,
        tray_ui: &Rc<SystemTray>,
        _hwnd: isize,
        msg: u32,
        w: usize,
        l: isize,
        _window: nwg::ControlHandle,
    ) -> Option<isize> {
        use windows::Win32::UI::{
            Shell::{NINF_KEY, NIN_SELECT},
            WindowsAndMessaging::{
                WM_CONTEXTMENU, WM_DPICHANGED, WM_ENTERIDLE, WM_EXITMENULOOP, WM_MBUTTONDOWN,
                WM_MENUCHAR, WM_MOUSEFIRST, WM_RBUTTONUP, WM_THEMECHANGED, WM_USER,
                WM_WININICHANGE,
            },
        };
        /// [NIN_KEYSELECT missing](https://github.com/microsoft/win32metadata/issues/1765)
        const NIN_KEYSELECT: u32 = NINF_KEY | NIN_SELECT;

        // List of messages:
        // https://wiki.winehq.org/List_Of_Windows_Messages
        // https://stackoverflow.com/questions/8824255/getting-a-windows-message-name

        /// From `nwg::win32::windows_helper`
        const NWG_TRAY: u32 = WM_USER + 102;

        // This gets tray events the same way as `nwg::win32::window::process_events`
        if msg != NWG_TRAY {
            if ![1124, 148, WM_ENTERIDLE].contains(&msg) {
                #[cfg(all(feature = "logging", debug_assertions))]
                tracing::trace!(
                    msg,
                    name = crate::wm_msg_to_string::wm_msg_to_string(msg),
                    l = l,
                    w = w,
                    handle = _hwnd,
                    "Non-tray event"
                );
            }

            if msg == WM_EXITMENULOOP {
                tray_ui.notify_tray_menu_closed();
            } else if msg == WM_MENUCHAR {
                // https://learn.microsoft.com/en-us/windows/win32/menurc/wm-menuchar
                // wParam: low order is key, high order is menu type
                // lParam: handle to active menu (not the selected/hovered item)
                let key_code = w as u32 & 0xFFFF;
                tracing::info!(
                    key = ?char::from_u32(key_code),
                    key_code = key_code,
                    menu_handle = format!("{l:x}"),
                    "Pressed key inside menu"
                );
                if let Some(effect) = tray_ui.notify_key_press_in_menu(key_code, l) {
                    tracing::debug!(
                        ?effect,
                        "Choose manual effect in response to keyboard button press"
                    );
                    match effect {
                        MenuKeyPressEffect::Ignore => return Some(0),
                        MenuKeyPressEffect::Close => {
                            // 1 in high-order word (above the first 16 bit):
                            return Some(1 << 16);
                        }
                        MenuKeyPressEffect::Execute(handle)
                        | MenuKeyPressEffect::Select(handle) => {
                            let should_execute = matches!(effect, MenuKeyPressEffect::Execute(..));
                            let item_index = match handle {
                                nwg::ControlHandle::Menu(..) => menu_index_in_parent(handle),
                                nwg::ControlHandle::MenuItem(..) => {
                                    menu_item_index_in_parent(handle)
                                }
                                _ => {
                                    tracing::error!(?handle, "Unsupported handle type");
                                    return Some(0);
                                }
                            };
                            let Some(item_index) = item_index else {
                                tracing::error!(
                                    ?handle,
                                    "Failed to find index of sub menu in its parent"
                                );
                                return Some(0);
                            };
                            let item_index = item_index as isize;
                            if item_index >= (1 << 16) {
                                tracing::error!(?item_index, "Menu item index is too large");
                                return Some(0);
                            }
                            if should_execute {
                                tracing::debug!(
                                    ?effect,
                                    "Executing menu item at index {item_index}"
                                );
                                // 2 in high-order word (above the first 16 bit):
                                return Some(2 << 16 | item_index);
                            } else {
                                tracing::debug!(
                                    ?effect,
                                    "Selecting menu item at index {item_index}"
                                );
                                // 3 in high-order word (above the first 16 bit):
                                return Some(3 << 16 | item_index);
                            }
                        }
                    }
                }
            } else if msg == WM_THEMECHANGED || msg == WM_WININICHANGE {
                tray_ui.notify_windows_mode_change();
            } else if msg == windows_msg_for_explorer_restart() {
                tray_ui.notify_explorer_restart();
            } else if msg == WM_DPICHANGED {
                // Seems this is needed to ensure we get the right coordinates for the tray icon
                // https://stackoverflow.com/questions/41649303/difference-between-notifyicon-version-and-notifyicon-version-4-used-in-notifyico#comment116492307_54639792
                tracing::info!("Rebuilding tray icon since DPI changed");
                tray_ui.root().need_rebuild.set(true);
            }
            return None;
        }
        let msg = l as u32 & 0xffff;
        // contains the icon ID:
        let _other_l = l as u32 & (!0xffff);
        let x = (w & 0xffff) as i16;
        let y = ((w >> 16) & 0xffff) as i16;

        if ![WM_MOUSEFIRST].contains(&msg) {
            #[cfg(all(feature = "logging", debug_assertions))]
            tracing::trace!(
                msg,
                name =? crate::wm_msg_to_string::wm_msg_to_string(msg),
                w = w,
                other_l = _other_l,
                l_as_pos =? (x, y),
                handle = _hwnd,
                "Tray event"
            );
        }

        match msg {
            // Left click with tray version 4:
            NIN_SELECT => {}
            // Enter or spacebar when tray is selected:
            NIN_KEYSELECT => {
                // We receive this twice when enter is pressed but once when pressing space:
                // https://github.com/openjdk/jdk/blob/72ca7bafcd49a98c1fe09da72e4e47683f052e9d/src/java.desktop/windows/native/libawt/windows/awt_TrayIcon.cpp#L449
                let now = Instant::now();
                if let Some(prev_time) = self.last_tray_key_event.replace(Some(now)) {
                    let duration = now.duration_since(prev_time);
                    if duration < Duration::from_millis(100) {
                        // Likely double event
                        tracing::debug!("Ignored double keypress event on tray icon");
                        return None;
                    }
                }

                tray_ui.notify_tray_left_click();
            }
            WM_MBUTTONDOWN => {
                tray_ui.notify_tray_middle_click();
            }
            WM_RBUTTONUP => {
                // Right mouse click on tray icon, after this we will receive a WM_CONTEXTMENU
            }
            // Only if using tray icon with version 4:
            WM_CONTEXTMENU => {
                self.notify_that_tray_icon_exists();
                tray_ui.show_menu(MenuPosition::At(i32::from(x), i32::from(y)));
            }
            _ => {}
        }

        None
    }

    fn need_rebuild(&self, _dynamic_ui: &Rc<SystemTray>) -> bool {
        self.need_rebuild.get()
    }

    fn before_rebuild(&mut self, _dynamic_ui: &Rc<SystemTray>) {
        // Tray icon doesn't disappear when program quits, so we need to remove
        // it manually:
        self.tray.set_visibility(false);

        *self = Self {
            first_created_at: self.first_created_at,
            ..Default::default()
        };
    }
}
impl TrayPlugin for TrayRoot {
    fn on_windows_mode_changed(&self, tray_ui: &Rc<SystemTray>) {
        self.update_tray_icon(tray_ui, tray_ui.desktop_index.get());
    }
    fn on_current_desktop_changed(&self, tray_ui: &Rc<SystemTray>, new_ix: u32) {
        // Change icon first since any delay in that is more visible than if the
        // tooltip isn't updated immediately:
        self.update_tray_icon(tray_ui, new_ix);
        self.tray.set_tip(&format!(
            "Virtual Desktop Manager\
            \n           [Desktop {}]{}",
            new_ix + 1,
            if let Some(name) = tray_ui.get_desktop_name(new_ix) {
                format!("\n  [{name}]")
            } else {
                "".to_string()
            }
        ));
    }
    fn on_settings_changed(
        &self,
        tray_ui: &Rc<SystemTray>,
        previous: &Arc<UiSettings>,
        new: &Arc<UiSettings>,
    ) {
        if previous.tray_icon_type != new.tray_icon_type {
            self.update_tray_icon(tray_ui, tray_ui.desktop_index.get());
        }
    }
}

/// Effect after a user presses a keyboard shortcut while a context menu is
/// active.
///
/// # References
///
/// - <https://learn.microsoft.com/en-us/windows/win32/menurc/wm-menuchar>
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)] // <- We might want to use the other alternatives in the future.
pub enum MenuKeyPressEffect {
    /// Discard the character the user pressed and create a short beep on the
    /// system speaker
    Ignore,
    /// Close the active menu.
    Close,
    /// Choose the provided menu item and then close the menu.
    Execute(nwg::ControlHandle),
    /// Select the provided menu item.
    Select(nwg::ControlHandle),
}

/// A trait for Native GUI plugins for the system tray.
pub trait TrayPlugin: DynamicUiHooks<SystemTray> {
    /// React to Virtual Desktop events.
    fn on_desktop_event(&self, _tray_ui: &Rc<SystemTray>, _event: &vd::DesktopEvent) {}
    fn on_current_desktop_changed(&self, _tray_ui: &Rc<SystemTray>, _current_desktop_index: u32) {}
    fn on_desktop_count_changed(&self, _tray_ui: &Rc<SystemTray>, _new_desktop_count: u32) {}

    /// Handle keyboard button press on tray context menu. The first return
    /// value that is `Some` will be used; if there is no such return value then
    /// [`MenuKeyPressEffect::Ignore`] will be used.
    ///
    /// The provided `menu_handle` is the currently select parent menu.
    fn on_menu_key_press(
        &self,
        _tray_ui: &Rc<SystemTray>,
        _key_code: u32,
        _menu_handle: isize,
    ) -> Option<MenuKeyPressEffect> {
        None
    }

    fn on_windows_mode_changed(&self, _tray_ui: &Rc<SystemTray>) {}

    fn on_settings_changed(
        &self,
        _tray_ui: &Rc<SystemTray>,
        _prev: &Arc<UiSettings>,
        _new: &Arc<UiSettings>,
    ) {
    }
}

#[allow(dead_code)] // <- we might want to use some variants in the future
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuPosition {
    At(i32, i32),
    AtPrevious,
    AtTrayIcon,
    AtMouseCursor,
}

pub type SystemTrayRef = DynamicUiRef<SystemTray>;

/// Common handle used by tray plugins, usually behind an [`Rc`] stored inside
/// [`SystemTrayRef`].
#[derive(Debug)]
pub struct SystemTray {
    /// The total number of virtual desktops.
    pub desktop_count: Cell<u32>,
    /// The 0-based index of the currently active virtual desktop.
    pub desktop_index: Cell<u32>,
    /// Windows has separate modes for Windows itself and other apps. This
    /// tracks whether the taskbar and Windows uses light colors.
    has_light_taskbar: Cell<bool>,

    desktop_names: RefCell<Vec<Option<Rc<str>>>>,

    pub dynamic_ui: DynamicUi<Self>,
}
impl DynamicUiWrapper for SystemTray {
    type Hooks = dyn TrayPlugin;

    fn get_dynamic_ui(&self) -> &DynamicUi<Self> {
        &self.dynamic_ui
    }

    fn get_dynamic_ui_mut(&mut self) -> &mut DynamicUi<Self> {
        &mut self.dynamic_ui
    }
}
/// Plugins.
impl SystemTray {
    pub fn new(mut plugins: Vec<Box<dyn TrayPlugin>>) -> Rc<Self> {
        plugins.insert(0, Box::<TrayRoot>::default());
        let has_light_taskbar = Self::check_if_light_taskbar();
        tracing::debug!(
            ?has_light_taskbar,
            "Detected Windows mode (affects taskbar color)"
        );
        let dynamic_ui = DynamicUi::new(plugins);
        dynamic_ui.set_prevent_recursive_events(true);
        Rc::new(Self {
            desktop_count: Cell::new(vd::get_desktop_count().unwrap_or(1)),
            desktop_index: Cell::new(
                vd::get_current_desktop()
                    .and_then(|d| d.get_index())
                    .unwrap_or(1),
            ),
            desktop_names: RefCell::new(
                vd::get_desktops()
                    .and_then(|ds| {
                        ds.into_iter()
                            .map(|d| d.get_name().map(Rc::from).map(Some))
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .inspect_err(|e| {
                        tracing::warn!("Failed to get desktop names: {e:?}");
                    })
                    .unwrap_or_default(),
            ),
            has_light_taskbar: Cell::new(has_light_taskbar),
            dynamic_ui,
        })
    }
    /// # References
    ///
    /// - <https://stackoverflow.com/questions/56865923/windows-10-taskbar-color-detection-for-tray-icon>
    /// - We use this function: [RegGetValueW in windows::Win32::System::Registry - Rust](https://microsoft.github.io/windows-docs-rs/doc/windows/Win32/System/Registry/fn.RegGetValueW.html)
    ///   - Function docs: [RegGetValueW function (winreg.h) - Win32 apps | Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/api/winreg/nf-winreg-reggetvaluew)
    ///   - StackOverflow usage example: [windows - RegGetValueW(), how to do it right - Stack Overflow](https://stackoverflow.com/questions/78224404/reggetvaluew-how-to-do-it-right)
    fn check_if_light_taskbar() -> bool {
        use windows::{
            core::w,
            Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD},
        };

        let mut buffer: [u8; 4] = [0; 4];
        let mut cb_data = buffer.len() as u32;
        let res = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                w!(r#"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"#),
                w!("SystemUsesLightTheme"),
                RRF_RT_REG_DWORD,
                Some(std::ptr::null_mut()),
                Some(buffer.as_mut_ptr() as _),
                Some(&mut cb_data as *mut u32),
            )
        };
        if res.is_err() {
            tracing::error!(
                "Failed to read Windows mode from the registry: {:?}",
                windows::core::Error::from(res.to_hresult())
            );
            return false;
        }

        // REG_DWORD is signed 32-bit, using little endian
        let windows_light_mode = i32::from_le_bytes(buffer);
        if ![0, 1].contains(&windows_light_mode) {
            tracing::error!(
                "Windows mode read from the registry was not 0 or 1 \
                ({windows_light_mode}), ignoring read value"
            );
            return false;
        }
        windows_light_mode == 1
    }
    pub fn build_ui(self: Rc<Self>) -> Result<DynamicUiOwner<Self>, nwg::NwgError> {
        <Rc<Self> as nwg::NativeUi<DynamicUiOwner<_>>>::build_ui(self)
    }
    pub fn root(&self) -> Ref<'_, TrayRoot> {
        self.dynamic_ui
            .get_ui::<TrayRoot>()
            .expect("Accessed TrayRoot while it was being rebuilt")
    }
    pub fn settings(&self) -> Ref<'_, crate::settings::UiSettingsPlugin> {
        self.dynamic_ui
            .get_ui::<crate::settings::UiSettingsPlugin>()
            .expect("Accessed UiSettingsPlugin while it was being rebuilt")
    }
    pub fn get_desktop_name(&self, index: u32) -> Option<Rc<str>> {
        if vd::has_loaded_dynamic_library_successfully() {
            // Don't get change events for desktop names, so just reload it
            // every time we change desktop:
            vd::get_desktop(index).get_name().ok().map(Rc::from)
        } else {
            self.desktop_names
                .borrow()
                .get(index as usize)
                .cloned()
                .flatten()
        }
    }
    /// Windows has separate modes for Windows itself and other apps. This
    /// tracks whether the taskbar and Windows uses light colors.
    pub fn has_light_taskbar(&self) -> bool {
        self.has_light_taskbar.get()
    }
}
/// Events.
impl SystemTray {
    pub fn notify_settings_changed(self: &Rc<Self>, prev: &Arc<UiSettings>, new: &Arc<UiSettings>) {
        self.dynamic_ui
            .for_each_ui(|plugin| plugin.on_settings_changed(self, prev, new));
    }
    fn notify_windows_mode_change(self: &Rc<Self>) {
        let is_light = Self::check_if_light_taskbar();
        let was_light = self.has_light_taskbar.replace(is_light);
        tracing::info!(
            ?was_light,
            ?is_light,
            "Windows changed its color mode (affects taskbar color)"
        );
        if is_light == was_light {
            return;
        }
        self.dynamic_ui
            .for_each_ui(|plugin| plugin.on_windows_mode_changed(self));
    }
    fn notify_explorer_restart(&self) {
        tracing::warn!(
            "Detected that Windows explorer.exe was restarted, attempting to re-register tray icon"
        );
        self.root().need_rebuild.set(true);
    }
    pub fn notify_desktop_event(self: &Rc<Self>, event: vd::DesktopEvent) {
        // Note: this will run inside an OnNotice event handler, so dynamic_ui
        // will check for rebuilding afterwards.

        use vd::DesktopEvent::*;

        tracing::trace!("Desktop event: {:?}", event);

        match &event {
            DesktopCreated { .. } | DesktopDestroyed { .. } => match vd::get_desktop_count() {
                Ok(count) => {
                    self.desktop_count.set(count);
                    {
                        let len = self.desktop_names.borrow().len() as u32;
                        match len.cmp(&count) {
                            std::cmp::Ordering::Less => {
                                let range = len..count;
                                let new_names: Vec<_> = range.map(
                                    |ix| match vd::get_desktop(ix).get_name() {
                                        Err(e) => {
                                            tracing::warn!(
                                                "Failed to get virtual desktop name for desktop {}: {e:?}",
                                                ix + 1
                                            );
                                            None
                                        }
                                        Ok(name) => Some(Rc::from(name)),
                                    },
                                ).collect();
                                self.desktop_names.borrow_mut().extend(new_names);
                            }
                            std::cmp::Ordering::Greater => {
                                self.desktop_names.borrow_mut().truncate(count as usize)
                            }
                            std::cmp::Ordering::Equal => {}
                        }
                    }
                    self.dynamic_ui
                        .for_each_ui(|plugin| plugin.on_desktop_count_changed(self, count));
                }
                Err(e) => tracing::error!("Failed to get virtual desktop count: {e:?}"),
            },
            DesktopNameChanged(d, new_name) => match d.get_index() {
                Err(e) => {
                    tracing::warn!("Failed to get virtual desktop index after name change: {e:?}");
                }
                Ok(ix) => {
                    let mut names = self.desktop_names.borrow_mut();
                    if let Some(name) = names.get_mut(ix as usize) {
                        *name = Some(Rc::from(&**new_name));
                    }
                }
            },
            DesktopChanged { new, .. } => {
                if let Ok(new_ix) = new.get_index() {
                    self.desktop_index.set(new_ix);
                    self.dynamic_ui
                        .for_each_ui(|plugin| plugin.on_current_desktop_changed(self, new_ix));
                }
            }
            _ => {}
        }

        self.dynamic_ui
            .for_each_ui(|plugin| plugin.on_desktop_event(self, &event));
    }
    fn notify_tray_left_click(&self) {
        self.root().notify_that_tray_icon_exists();
        self.configure_filters(false);
    }
    fn notify_tray_middle_click(&self) {
        self.root().notify_that_tray_icon_exists();
        self.apply_filters();
    }
    fn notify_tray_menu_closed(&self) {
        // Attempt to give focus back to the most recent window:
        self.hide_menu();
        if let Some(plugin) = self.dynamic_ui.get_ui::<SmoothDesktopSwitcher>() {
            plugin.refocus_last_window();
        }
    }
    /// Note: this isn't run inside a `SystemTray::handle_action` callback and
    /// so we might handle events while something is being rebuilt.
    fn notify_key_press_in_menu(
        self: &Rc<Self>,
        key_code: u32,
        active_menu_handle: isize,
    ) -> Option<MenuKeyPressEffect> {
        let mut first_res = None;
        self.dynamic_ui.for_each_ui(|t| {
            if let Some(res) = t.on_menu_key_press(self, key_code, active_menu_handle) {
                if first_res.is_none() {
                    first_res = Some(res);
                }
            }
        });
        first_res
    }
}
/// Commands.
impl SystemTray {
    pub fn switch_desktop(&self, desktop_ix: u32) {
        tracing::info!("SystemTray::switch_desktop({})", desktop_ix);
        // TODO: store this in settings:
        let smooth = self
            .dynamic_ui
            .get_ui::<crate::tray_plugins::menus::TopMenuItems>()
            .map_or_else(
                || {
                    tracing::warn!("No TopMenuItems: can't check if smooth scroll is enabled");
                    false
                },
                |top| top.tray_smooth_switch.checked(),
            );

        let desktop = vd::get_desktop(desktop_ix);
        let res = 'result: {
            if smooth {
                if let Some(plugin) = self.dynamic_ui.get_ui::<SmoothDesktopSwitcher>() {
                    // Attempt to hide menu since its closing animation doesn't
                    // look nice when smoothly switching desktop:
                    self.hide_menu();
                    //crate::invisible_window::switch_desktop_with_invisible_window(desktop, Some(self.window.handle))
                    break 'result plugin.switch_desktop_to(desktop);
                }
                tracing::warn!("No SmoothDesktopSwitcher: can't execute smooth scroll");
            }
            vd::switch_desktop(desktop)
        };
        if let Err(e) = res {
            self.show_notification(
                "Virtual Desktop Manager Error",
                &format!(
                    "Failed switch to Virtual Desktop {}: {e:?}",
                    desktop_ix.saturating_add(1)
                ),
            );
        }
    }
    /// This doesn't seem to actually do anything, needs to be changed to
    /// actually work.
    fn hide_menu(&self) {
        tracing::info!("SystemTray::hide_menu()");
        use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
        unsafe {
            // https://stackoverflow.com/questions/19226173/how-to-close-a-context-menu-after-a-timeout
            // https://learn.microsoft.com/sv-se/windows/win32/winmsg/wm-cancelmode?redirectedfrom=MSDN
            let _ = ShowWindow(
                HWND(self.root().tray_menu.handle.pop_hmenu().unwrap().1 as isize),
                SW_HIDE,
            );
        }
    }
    pub fn show_menu(&self, position: MenuPosition) {
        let root = self.root();
        let (x, y) = match position {
            MenuPosition::At(x, y) => (x, y),
            MenuPosition::AtPrevious => root
                .last_menu_pos
                .get()
                .unwrap_or_else(nwg::GlobalCursor::position),
            MenuPosition::AtTrayIcon => match tray_get_rect(&root.tray) {
                Ok(rect) => ((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2),
                Err(e) => {
                    tracing::error!("Failed to get tray location: {e}");
                    nwg::GlobalCursor::position()
                }
            },
            MenuPosition::AtMouseCursor => nwg::GlobalCursor::position(),
        };
        tracing::info!(
            actual_position = ?(x, y),
            requested_position = ?position,
            tray_location = ?tray_get_rect(&root.tray),
            cursor_position = ?nwg::GlobalCursor::position(),
            previous_pos = ?root.last_menu_pos.get(),
            "SystemTray::show_menu()"
        );

        if let Some(plugin) = self.dynamic_ui.get_ui::<SmoothDesktopSwitcher>() {
            plugin.cancel_refocus();
        }
        root.last_menu_pos.set(Some((x, y)));
        root.tray_menu.popup(x, y);
    }
    pub fn show_notification(&self, title: &str, text: &str) {
        let flags = nwg::TrayNotificationFlags::USER_ICON | nwg::TrayNotificationFlags::LARGE_ICON;
        self.root()
            .tray
            .show(text, Some(title), Some(flags), Some(&self.root().icon));
    }
    pub fn apply_filters(&self) {
        tracing::info!("SystemTray::apply_filters()");
        if let Some(apply_filters) = self
            .get_dynamic_ui()
            .get_ui::<crate::tray_plugins::apply_filters::ApplyFilters>()
        {
            let settings = self.settings().get();
            let filters = settings.filters.clone();
            apply_filters.apply_filters(
                filters,
                settings.stop_flashing_windows_after_applying_filter,
            );
        } else {
            self.show_notification(
                "Virtual Desktop Manager Warning",
                "Applying filters is not supported",
            );
        }
    }
    pub fn configure_filters(&self, refocus: bool) {
        tracing::info!("SystemTray::configure_filters()");
        if let Some(config_window) = self.dynamic_ui.get_ui::<ConfigWindow>() {
            if config_window.is_closed() {
                config_window.open_soon.set(true);
            } else if refocus {
                config_window.set_as_foreground_window();
            } else {
                config_window.window.close();
            }
        }
    }

    pub fn exit(&self) {
        tracing::info!("SystemTray::exit()");
        nwg::stop_thread_dispatch();
    }
}
