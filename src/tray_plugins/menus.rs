use nwg::MenuSeparator;

use crate::{
    dynamic_gui::{forward_to_dynamic_ui, DynamicUiHooks, DynamicUiWrapper},
    nwg_ext::menu_remove,
    settings::{AutoStart, QuickSwitchMenu, TrayIconType, UiSettings},
    tray::{MenuKeyPressEffect, MenuPosition, SystemTray, SystemTrayRef, TrayPlugin, TrayRoot},
    vd,
};
use std::{
    any::TypeId,
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
    rc::Rc,
    sync::Arc,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SubMenu {
    /// Handle for the [`nwg::Menu`] that represents a submenu.
    Handle(nwg::ControlHandle),
    /// ASCII character that will select the sub menu.
    AccessKey(u8),
}
impl SubMenu {
    fn open(self) {
        let Some(context_menu) = crate::nwg_ext::find_context_menu_window() else {
            tracing::warn!(wanted_sub_menu =? self, "Failed to find context menu window");
            return;
        };

        match self {
            SubMenu::Handle(control_handle) => {
                let Some(index) = crate::nwg_ext::menu_index_in_parent(control_handle) else {
                    tracing::warn!("Failed to find settings submenu");
                    return;
                };

                use windows::Win32::{
                    Foundation::{LPARAM, WPARAM},
                    UI::{
                        Input::KeyboardAndMouse::VK_RETURN,
                        WindowsAndMessaging::{PostMessageW, WM_KEYDOWN},
                    },
                };

                unsafe {
                    // Select submenu item:
                    _ = PostMessageW(
                        context_menu,
                        0x1e5,
                        WPARAM(usize::try_from(index).unwrap()),
                        LPARAM(0),
                    );
                    // Activate it:
                    _ = PostMessageW(
                        context_menu,
                        WM_KEYDOWN,
                        WPARAM(usize::from(VK_RETURN.0)),
                        LPARAM(0),
                    );
                }
            }
            SubMenu::AccessKey(key) => {
                use windows::Win32::{
                    Foundation::{LPARAM, WPARAM},
                    UI::WindowsAndMessaging::{PostMessageW, WM_KEYDOWN},
                };
                unsafe {
                    _ = PostMessageW(
                        context_menu,
                        WM_KEYDOWN,
                        WPARAM(usize::from(key)),
                        LPARAM(0),
                    );
                }
            }
        }
    }
}

/// Listens for context menu events to be able to then focus on submenu items
/// and expand them.
#[derive(Default, nwd::NwgPartial)]
pub struct OpenSubmenuPlugin {
    submenus: RefCell<VecDeque<SubMenu>>,
}
impl OpenSubmenuPlugin {
    pub fn queue_open_of(&self, submenu: impl IntoIterator<Item = SubMenu>) {
        let items = submenu.into_iter().collect::<Vec<_>>();
        self.submenus.borrow_mut().extend(items);
    }
}
impl DynamicUiHooks<SystemTray> for OpenSubmenuPlugin {
    fn before_partial_build(
        &mut self,
        tray_ui: &Rc<SystemTray>,
        _should_build: &mut bool,
    ) -> Option<(nwg::ControlHandle, TypeId)> {
        Some((tray_ui.root().tray_menu.handle, TypeId::of::<TrayRoot>()))
    }
    fn after_process_events(
        &self,
        _dynamic_ui: &Rc<SystemTray>,
        evt: nwg::Event,
        _evt_data: &nwg::EventData,
        _handle: nwg::ControlHandle,
        _window: nwg::ControlHandle,
    ) {
        if let nwg::Event::OnMenuOpen = evt {
            let Some(next) = self.submenus.borrow_mut().pop_front() else {
                return;
            };
            next.open();
        }
    }
}
impl TrayPlugin for OpenSubmenuPlugin {}

/// A submenu item (with an extra separator) that can be used as the parent of
/// the quick switch menu items.
#[derive(Default, nwd::NwgPartial)]
pub struct QuickSwitchTopMenu {
    #[nwg_control(text: "&Quick Switch")]
    tray_quick_menu: nwg::Menu,

    #[nwg_control()]
    tray_sep: nwg::MenuSeparator,

    is_built: bool,
}
impl DynamicUiHooks<SystemTray> for QuickSwitchTopMenu {
    fn before_partial_build(
        &mut self,
        tray_ui: &Rc<SystemTray>,
        should_build: &mut bool,
    ) -> Option<(nwg::ControlHandle, TypeId)> {
        let should_enable = tray_ui.settings().get().quick_switch_menu == QuickSwitchMenu::SubMenu;
        if !should_enable {
            *should_build = false;
            return None;
        }
        Some((tray_ui.root().tray_menu.handle, TypeId::of::<TrayRoot>()))
    }
    fn after_partial_build(&mut self, _dynamic_ui: &Rc<SystemTray>) {
        self.is_built = true;
    }
    fn need_rebuild(&self, tray_ui: &Rc<SystemTray>) -> bool {
        let should_enable = tray_ui.settings().get().quick_switch_menu == QuickSwitchMenu::SubMenu;
        should_enable != self.is_built
    }
    fn before_rebuild(&mut self, _dynamic_ui: &Rc<SystemTray>) {
        menu_remove(&self.tray_quick_menu);
        *self = Default::default();
    }
}
impl TrayPlugin for QuickSwitchTopMenu {
    fn on_current_desktop_changed(&self, _tray_ui: &Rc<SystemTray>, current_desktop_index: u32) {
        if !self.is_built {
            return;
        }
        crate::nwg_ext::menu_set_text(
            self.tray_quick_menu.handle,
            &format!("&Quick Switch from Desktop {}", current_desktop_index + 1),
        );
    }
}

#[derive(Default, nwd::NwgPartial)]
pub struct TopMenuItems {
    tray_ui: SystemTrayRef,

    #[nwg_control(text: "&Close Current Desktop")]
    #[nwg_events(OnMenuItemSelected: [Self::close_current_desktop])]
    tray_close_desktop: nwg::MenuItem,

    #[nwg_control(text: "&New Desktop")]
    #[nwg_events(OnMenuItemSelected: [Self::create_desktop])]
    tray_create_desktop: nwg::MenuItem,

    #[nwg_control()]
    tray_sep1: nwg::MenuSeparator,

    #[nwg_control(text: "&Smooth Desktop Switch")]
    #[nwg_events(OnMenuItemSelected: [Self::toggle_smooth_switch])]
    pub tray_smooth_switch: nwg::MenuItem,

    #[nwg_control(text: "More &Options")]
    tray_settings_menu: nwg::Menu,

    #[cfg(feature = "admin_startup")]
    #[nwg_control(text: "&Request Admin at Startup", parent: tray_settings_menu)]
    #[nwg_events(OnMenuItemSelected: [Self::toggle_request_admin_at_startup])]
    tray_request_admin_at_startup: nwg::MenuItem,

    #[nwg_control(text: "Tray &Icon", parent: tray_settings_menu)]
    tray_icon_menu: nwg::Menu,

    /// One menu item per icon type.
    tray_icon_types: Vec<nwg::MenuItem>,

    #[nwg_control(text: "&Quick Switch Menu", parent: tray_settings_menu)]
    tray_quick_switch_menu: nwg::Menu,

    /// One menu item per quick switch option.
    tray_quick_switch_items: Vec<nwg::MenuItem>,

    #[nwg_control(text: "Auto &Start", parent: tray_settings_menu)]
    tray_auto_start_menu: nwg::Menu,

    /// One menu item per auto start option.
    tray_auto_start_items: Vec<nwg::MenuItem>,

    #[nwg_control()]
    tray_sep2: nwg::MenuSeparator,
}
impl DynamicUiHooks<SystemTray> for TopMenuItems {
    fn before_partial_build(
        &mut self,
        tray_ui: &Rc<SystemTray>,
        _should_build: &mut bool,
    ) -> Option<(nwg::ControlHandle, TypeId)> {
        Some((tray_ui.root().tray_menu.handle, TypeId::of::<TrayRoot>()))
    }
    fn after_partial_build(&mut self, tray_ui: &Rc<SystemTray>) {
        self.tray_ui.set(tray_ui);
        let settings = tray_ui.settings().get();

        #[cfg(feature = "admin_startup")]
        {
            self.tray_request_admin_at_startup
                .set_checked(settings.request_admin_at_startup);
            self.update_label_for_request_admin_at_startup();
        }

        self.tray_smooth_switch
            .set_checked(settings.smooth_switch_desktops);
        self.update_label_for_smooth_switch();

        {
            let menu_items = &mut self.tray_icon_types;
            menu_items.clear();

            for tray_icon in TrayIconType::ALL {
                let mut item = Default::default();
                let res = nwg::MenuItem::builder()
                    .text(&format!("{tray_icon:?}"))
                    .parent(self.tray_icon_menu.handle)
                    .build(&mut item);
                if let Err(e) = res {
                    tracing::error!(
                        "Failed to build menu item for tray icon type {tray_icon:?}: {e}"
                    );
                }
                menu_items.push(item);
            }
        }
        self.check_selected_tray_icon(settings.tray_icon_type);

        {
            let menu_items = &mut self.tray_quick_switch_items;
            menu_items.clear();

            for option in QuickSwitchMenu::ALL {
                let mut item = Default::default();
                let res = nwg::MenuItem::builder()
                    .text(&format!("{option:?}"))
                    .parent(self.tray_quick_switch_menu.handle)
                    .build(&mut item);
                if let Err(e) = res {
                    tracing::error!(
                        "Failed to build menu item for quick switch option \"{option:?}\": {e}"
                    );
                }
                menu_items.push(item);
            }
        }
        self.check_selected_quick_switch(settings.quick_switch_menu);

        {
            let menu_items = &mut self.tray_auto_start_items;
            menu_items.clear();

            for option in AutoStart::ALL {
                let mut item = Default::default();
                let res = nwg::MenuItem::builder()
                    .text(&format!("{option:?}"))
                    .parent(self.tray_auto_start_menu.handle)
                    .build(&mut item);
                if let Err(e) = res {
                    tracing::error!(
                        "Failed to build menu item for auto start option \"{option:?}\": {e}"
                    );
                }
                menu_items.push(item);
            }
        }
        self.check_selected_auto_start(settings.auto_start);
    }
    fn before_rebuild(&mut self, tray_ui: &Rc<SystemTray>) {
        *self = Default::default();
        self.tray_ui.set(tray_ui);
    }
    fn after_process_events(
        &self,
        dynamic_ui: &Rc<SystemTray>,
        evt: nwg::Event,
        _evt_data: &nwg::EventData,
        handle: nwg::ControlHandle,
        _window: nwg::ControlHandle,
    ) {
        if let nwg::Event::OnMenuItemSelected = evt {
            let new_icon = self
                .tray_icon_types
                .iter()
                .zip(TrayIconType::ALL)
                .find(|(item, _)| item.handle == handle)
                .map(|(_, icon)| *icon);

            if let Some(new_icon) = new_icon {
                dynamic_ui.settings().update(|prev| UiSettings {
                    tray_icon_type: new_icon,
                    ..prev.clone()
                });
            }

            let wanted_quick_switch = self
                .tray_quick_switch_items
                .iter()
                .zip(QuickSwitchMenu::ALL)
                .find(|(item, _)| item.handle == handle)
                .map(|(_, option)| *option);

            if let Some(wanted_quick_switch) = wanted_quick_switch {
                dynamic_ui.settings().update(|prev| UiSettings {
                    quick_switch_menu: wanted_quick_switch,
                    ..prev.clone()
                });
            }

            let auto_start = self
                .tray_auto_start_items
                .iter()
                .zip(AutoStart::ALL)
                .find(|(item, _)| item.handle == handle)
                .map(|(_, option)| *option);

            if let Some(auto_start) = auto_start {
                dynamic_ui.settings().update(|prev| UiSettings {
                    auto_start,
                    ..prev.clone()
                });
            }
        }
    }
}
impl TrayPlugin for TopMenuItems {
    fn on_settings_changed(
        &self,
        _tray_ui: &Rc<SystemTray>,
        prev: &Arc<UiSettings>,
        settings: &Arc<UiSettings>,
    ) {
        #[cfg(feature = "admin_startup")]
        {
            if self.tray_request_admin_at_startup.checked() != settings.request_admin_at_startup {
                self.tray_request_admin_at_startup
                    .set_checked(settings.request_admin_at_startup);
                self.update_label_for_request_admin_at_startup();
            }
        }

        if self.tray_smooth_switch.checked() != settings.smooth_switch_desktops {
            self.tray_smooth_switch
                .set_checked(settings.smooth_switch_desktops);
            self.update_label_for_smooth_switch();
        }

        if prev.tray_icon_type != settings.tray_icon_type {
            self.check_selected_tray_icon(settings.tray_icon_type);
        }
        if prev.quick_switch_menu != settings.quick_switch_menu {
            self.check_selected_quick_switch(settings.quick_switch_menu);
        }
        if prev.auto_start != settings.auto_start {
            self.check_selected_auto_start(settings.auto_start);
        }
    }
}
/// Handle clicked menu items.
impl TopMenuItems {
    fn close_current_desktop(&self) {
        let Some(tray_ui) = self.tray_ui.get() else {
            return;
        };
        let result = vd::get_current_desktop().and_then(|current| {
            let ix = current.get_index()?;
            vd::remove_desktop(
                current,
                // Fallback to the left but if we are at the first then fallback
                // to the right:
                vd::Desktop::from(ix.checked_sub(1).unwrap_or(1)),
            )?;
            Ok(())
        });
        if let Err(e) = result {
            tray_ui.show_notification(
                "Virtual Desktop Manager Error",
                &format!("Failed to create a new virtual desktop with: {e:?}"),
            );
        }
    }
    fn create_desktop(&self) {
        let Some(tray_ui) = self.tray_ui.get() else {
            return;
        };
        if let Err(e) = vd::create_desktop() {
            tray_ui.show_notification(
                "Virtual Desktop Manager Error",
                &format!("Failed to create a new virtual desktop with: {e:?}"),
            );
        }
    }
    fn toggle_smooth_switch(&self) {
        let Some(tray_ui) = self.tray_ui.get() else {
            return;
        };
        let new_value = !self.tray_smooth_switch.checked();
        self.tray_smooth_switch.set_checked(new_value);
        tray_ui.settings().update(|prev| UiSettings {
            smooth_switch_desktops: new_value,
            ..prev.clone()
        });
        self.update_label_for_smooth_switch();
        tray_ui.show_menu(MenuPosition::AtPrevious);
    }
    #[cfg(feature = "admin_startup")]
    fn toggle_request_admin_at_startup(&self) {
        let Some(tray_ui) = self.tray_ui.get() else {
            return;
        };
        let new_value = !self.tray_request_admin_at_startup.checked();
        self.tray_request_admin_at_startup.set_checked(new_value);
        tray_ui.settings().update(|prev| UiSettings {
            request_admin_at_startup: new_value,
            ..prev.clone()
        });
        self.update_label_for_request_admin_at_startup();
        if let Some(plugin) = tray_ui.get_dynamic_ui().get_ui::<OpenSubmenuPlugin>() {
            plugin.queue_open_of([SubMenu::Handle(self.tray_settings_menu.handle)]);
        };
        tray_ui.show_menu(MenuPosition::AtPrevious);
    }
}
/// Helper methods.
impl TopMenuItems {
    #[cfg(feature = "admin_startup")]
    fn update_label_for_request_admin_at_startup(&self) {
        let checked = self.tray_request_admin_at_startup.checked();
        crate::nwg_ext::menu_set_text(
            self.tray_request_admin_at_startup.handle,
            &format!(
                "Request Admin at Startup ({})",
                if checked { "On" } else { "Off" }
            ),
        );
    }
    fn update_label_for_smooth_switch(&self) {
        let checked = self.tray_smooth_switch.checked();
        crate::nwg_ext::menu_set_text(
            self.tray_smooth_switch.handle,
            &format!(
                "&Smooth Desktop Switch ({})",
                if checked { "On" } else { "Off" }
            ),
        );
    }
    fn check_selected_tray_icon(&self, selected: TrayIconType) {
        let items = &self.tray_icon_types;
        for (item, icon) in items.iter().zip(TrayIconType::ALL) {
            let should_check = *icon == selected;
            if should_check != item.checked() {
                // This re-renders the item to ensure it gets updated if the context menu is open
                item.set_enabled(true);
                // Do this after `set_enabled` since that resets the checked status.
                item.set_checked(should_check);
            }
        }
    }
    fn check_selected_quick_switch(&self, selected: QuickSwitchMenu) {
        let items = &self.tray_quick_switch_items;
        for (item, option) in items.iter().zip(QuickSwitchMenu::ALL) {
            let should_check = *option == selected;
            if should_check != item.checked() {
                // This re-renders the item to ensure it gets updated if the context menu is open
                item.set_enabled(true);
                // Do this after `set_enabled` since that resets the checked status.
                item.set_checked(should_check);
            }
        }
    }
    fn check_selected_auto_start(&self, selected: AutoStart) {
        let items = &self.tray_auto_start_items;
        for (item, option) in items.iter().zip(AutoStart::ALL) {
            let should_check = *option == selected;
            if should_check != item.checked() {
                // This re-renders the item to ensure it gets updated if the context menu is open
                item.set_enabled(true);
                // Do this after `set_enabled` since that resets the checked status.
                item.set_checked(should_check);
            }
        }
    }
}

/// Context menu items to switch to another virtual desktop. Not nested under a
/// submenu but rather all flat under the root menu. These are also "checked"
/// when you are currently on that desktop.
#[derive(Default)]
pub struct FlatSwitchMenu {
    tray_ui: SystemTrayRef,

    /// Update right before UI build, so we can use this to track if we need to
    /// rebuild.
    desktop_count: u32,

    /// One menu item per open virtual desktop.
    tray_virtual_desktops: Vec<nwg::MenuItem>,
}
impl FlatSwitchMenu {
    fn check_current_desktop(&self, current_desktop_index: u32) {
        let desktops = self.tray_virtual_desktops.as_slice();
        for (i, desktop) in desktops.iter().rev().enumerate() {
            let is_current = i == current_desktop_index as usize;
            let was_checked = desktop.checked();
            if is_current != was_checked {
                // This re-renders the item to ensure it gets updated if the context menu is open
                desktop.set_enabled(true);
                // Do this after `set_enabled` since it resets the checked status.
                desktop.set_checked(is_current);
            }
        }
    }
}
impl nwg::PartialUi for FlatSwitchMenu {
    fn build_partial<W: Into<nwg::ControlHandle>>(
        data: &mut Self,
        parent: Option<W>,
    ) -> Result<(), nwg::NwgError> {
        let parent = parent.map(Into::into).ok_or_else(|| {
            nwg::NwgError::MenuCreationError("No parent defined for FlatSwitchMenu".to_string())
        })?;
        {
            let tray_desktops = &mut data.tray_virtual_desktops;
            tray_desktops.clear();

            for i in (1..=data.desktop_count.min(15)).rev() {
                let mut item = Default::default();
                nwg::MenuItem::builder()
                    .text(&format!(
                        "Virtual desktop {}{i}",
                        if i < 10 { "&" } else { "" }
                    ))
                    .parent(parent)
                    .build(&mut item)
                    .map_err(|e| {
                        nwg::NwgError::MenuCreationError(format!(
                            "Failed to build menu item for FlatSwitchMenu: {e}"
                        ))
                    })?;
                tray_desktops.push(item);
            }
        }

        // After we rebuilt the context menu, we need to mark the currently
        // active virtual desktop:
        if let Some(tray_ui) = data.tray_ui.get() {
            data.check_current_desktop(tray_ui.desktop_index.get());
        }

        Ok(())
    }
    fn process_event(
        &self,
        evt: nwg::Event,
        _evt_data: &nwg::EventData,
        handle: nwg::ControlHandle,
    ) {
        if let nwg::Event::OnMenuItemSelected = evt {
            let desktop_ix = self
                .tray_virtual_desktops
                .iter()
                .rev()
                .position(|d| d.handle == handle);
            if let Some(clicked_desktop_ix) = desktop_ix {
                if let Some(tray_ui) = self.tray_ui.get() {
                    tray_ui.switch_desktop(clicked_desktop_ix as u32);
                }
            }
        }
    }
}
impl DynamicUiHooks<SystemTray> for FlatSwitchMenu {
    fn before_partial_build(
        &mut self,
        tray_ui: &Rc<SystemTray>,
        should_build: &mut bool,
    ) -> Option<(nwg::ControlHandle, TypeId)> {
        if tray_ui.settings().get().quick_switch_menu == QuickSwitchMenu::TopMenu {
            *should_build = false;
            return None;
        }
        self.desktop_count = tray_ui.desktop_count.get();
        self.tray_ui.set(tray_ui);
        Some((tray_ui.root().tray_menu.handle, TypeId::of::<TrayRoot>()))
    }
    fn need_rebuild(&self, tray_ui: &Rc<SystemTray>) -> bool {
        if tray_ui.settings().get().quick_switch_menu == QuickSwitchMenu::TopMenu {
            self.desktop_count != 0 // Want 0 flat switch items
        } else {
            self.desktop_count != tray_ui.desktop_count.get()
        }
    }
}
impl TrayPlugin for FlatSwitchMenu {
    fn on_current_desktop_changed(&self, _tray_ui: &Rc<SystemTray>, current_desktop_index: u32) {
        self.check_current_desktop(current_desktop_index);
    }
}

/// Listens for backspace key presses and sends escape key events when they
/// occur. This allows backspace to be used to close submenus which works quite
/// intuitively with the quick switch menu.
#[derive(Default, nwd::NwgPartial)]
pub struct BackspaceAsEscapeAlias {}
impl DynamicUiHooks<SystemTray> for BackspaceAsEscapeAlias {
    fn before_partial_build(
        &mut self,
        _dynamic_ui: &Rc<SystemTray>,
        _should_build: &mut bool,
    ) -> Option<(nwg::ControlHandle, TypeId)> {
        None
    }
}
impl TrayPlugin for BackspaceAsEscapeAlias {
    fn on_menu_key_press(
        &self,
        _tray_ui: &Rc<SystemTray>,
        key_code: u32,
        _menu_handle: isize,
    ) -> Option<MenuKeyPressEffect> {
        if key_code != 8 {
            // Not backspace key
            return None;
        }
        'simulate_escape: {
            use windows::Win32::{
                Foundation::{LPARAM, WPARAM},
                UI::{
                    Input::KeyboardAndMouse::VK_ESCAPE,
                    WindowsAndMessaging::{SendMessageW, WM_KEYDOWN},
                },
            };

            let Some(context_menu_window) = crate::nwg_ext::find_context_menu_window() else {
                tracing::warn!("Unable to find context menu window");
                break 'simulate_escape;
            };
            unsafe {
                SendMessageW(
                    context_menu_window,
                    WM_KEYDOWN,
                    WPARAM(usize::from(VK_ESCAPE.0)),
                    LPARAM(0),
                );
            }
        }
        Some(MenuKeyPressEffect::SelectIndex(0))
    }
}

/// Create quick switch menu that makes use of keyboard access keys to allow for
/// fast navigation (Note: you can use Win+B to select the toolbar and then the
/// Enter key to open the context menu, after that you can press `Q` to open the
/// quick switch menu):
#[derive(Default)]
pub struct QuickSwitchMenuUiAdapter {
    tray_ui: SystemTrayRef,

    /// Update right before UI build, so we can use this to track if we need to
    /// rebuild.
    desktop_count: u32,

    /// Extra separators when inside top menu.
    extra_separators: Option<(nwg::MenuSeparator, nwg::MenuSeparator)>,

    parent: nwg::ControlHandle,

    tray_quick_menu_state: crate::quick_switch::QuickSwitchMenu,
}
impl nwg::PartialUi for QuickSwitchMenuUiAdapter {
    fn build_partial<W: Into<nwg::ControlHandle>>(
        data: &mut Self,
        parent: Option<W>,
    ) -> Result<(), nwg::NwgError> {
        let parent = parent.map(Into::into).ok_or_else(|| {
            nwg::NwgError::MenuCreationError("No parent defined for quick switch menu".to_string())
        })?;
        data.parent = parent;
        if let Some((first, _)) = &mut data.extra_separators {
            MenuSeparator::builder().parent(parent).build(first)?;
        }

        let quick = &mut data.tray_quick_menu_state;
        quick.clear();
        quick.create_quick_switch_menu(parent, data.desktop_count + 1);

        if let Some((_, last)) = &mut data.extra_separators {
            MenuSeparator::builder().parent(parent).build(last)?;
        }
        Ok(())
    }
    fn process_event(
        &self,
        evt: nwg::Event,
        _evt_data: &nwg::EventData,
        handle: nwg::ControlHandle,
    ) {
        if let nwg::Event::OnMenuItemSelected = evt {
            let desktop_ix = self.tray_quick_menu_state.get_clicked_desktop_index(handle);
            if let Some(clicked_desktop_ix) = desktop_ix {
                if let Some(tray_ui) = self.tray_ui.get() {
                    tray_ui.switch_desktop(clicked_desktop_ix as u32);
                }
            }
        }
    }
}
impl DynamicUiHooks<SystemTray> for QuickSwitchMenuUiAdapter {
    fn before_partial_build(
        &mut self,
        tray_ui: &Rc<SystemTray>,
        should_build: &mut bool,
    ) -> Option<(nwg::ControlHandle, TypeId)> {
        let settings = tray_ui.settings().get();
        self.tray_quick_menu_state.shortcuts =
            BTreeMap::clone(&settings.quick_switch_menu_shortcuts);
        self.tray_quick_menu_state.shortcuts_only_in_root =
            settings.quick_switch_menu_shortcuts_only_in_root;
        if settings.quick_switch_menu == QuickSwitchMenu::Disabled {
            *should_build = false;
            return None;
        }
        self.tray_ui.set(tray_ui);
        let (parent_handle, parent_id) = if let Some(menu) = tray_ui
            .dynamic_ui
            .get_ui::<QuickSwitchTopMenu>()
            .filter(|top| top.is_built)
        {
            (
                menu.tray_quick_menu.handle,
                TypeId::of::<QuickSwitchTopMenu>(),
            )
        } else {
            tracing::info!(
                "No QuickSwitchTopMenu so quick switch menu will be inlined in the root context menu"
            );
            self.extra_separators = Some(Default::default());
            (tray_ui.root().tray_menu.handle, TypeId::of::<TrayRoot>())
        };
        self.desktop_count = tray_ui.desktop_count.get();
        Some((parent_handle, parent_id))
    }
    fn need_rebuild(&self, tray_ui: &Rc<SystemTray>) -> bool {
        let settings = tray_ui.settings().get();
        let has_moved_menu = if settings.quick_switch_menu == QuickSwitchMenu::Disabled {
            self.desktop_count != 0
        } else {
            self.desktop_count != tray_ui.desktop_count.get()
        };
        let has_changed_shortcuts = settings.quick_switch_menu_shortcuts_only_in_root
            != self.tray_quick_menu_state.shortcuts_only_in_root
            || *settings.quick_switch_menu_shortcuts != self.tray_quick_menu_state.shortcuts;
        has_moved_menu || has_changed_shortcuts
    }
    fn before_rebuild(&mut self, _tray_ui: &Rc<SystemTray>) {
        // Reuse quick menu internal capacity:
        let quick = std::mem::take(&mut self.tray_quick_menu_state);
        *self = Default::default();
        self.tray_quick_menu_state = quick;
        self.tray_quick_menu_state.clear();
    }
}
impl TrayPlugin for QuickSwitchMenuUiAdapter {
    fn on_menu_key_press(
        &self,
        tray_ui: &Rc<SystemTray>,
        key_code: u32,
        menu_handle: isize,
    ) -> Option<MenuKeyPressEffect> {
        let key = char::from_u32(key_code)?;
        if key == 'q' || key == 'Q' {
            let parent = match self.parent {
                nwg::ControlHandle::Menu(_, h) => h as isize,
                nwg::ControlHandle::PopMenu(_, h) => h as isize,
                _ => {
                    tracing::error!("Parent to quick switch menu wasn't a menu");
                    return None;
                }
            };
            if parent != menu_handle {
                return None; // Not inside same menu as quick switch items
            }
            let item = self
                .tray_quick_menu_state
                .first_item_in_submenu(menu_handle)?;
            return Some(MenuKeyPressEffect::Select(item));
        }
        if key != ' ' {
            return None;
        }
        let Some(wanted_ix) = self
            .tray_quick_menu_state
            .get_desktop_index_so_far(menu_handle)
        else {
            tracing::debug!("Could not find quick switch submenu when pressing space");
            return None;
        };
        tracing::info!(
            "Pressed space while inside a quick switch context submenu that \
            would have been opened by pressing the access keys corresponding \
            to the desktop with the one-based index {}",
            wanted_ix + 1
        );
        tray_ui.switch_desktop(wanted_ix as u32);
        Some(MenuKeyPressEffect::Close)
    }
}

#[derive(Default, nwd::NwgPartial)]
pub struct BottomMenuItems {
    tray_ui: SystemTrayRef,

    #[nwg_control]
    tray_sep1: nwg::MenuSeparator,

    #[nwg_control(text: "Stop &Flashing Windows")]
    #[nwg_events(OnMenuItemSelected: [Self::stop_flashing_windows])]
    tray_stop_flashing: nwg::MenuItem,

    #[nwg_control(text: "Configure Filters")]
    #[nwg_events(OnMenuItemSelected: [Self::open_filter_config])]
    tray_configure_filters: nwg::MenuItem,

    #[nwg_control(text: "Apply Filters")]
    #[nwg_events(OnMenuItemSelected: [Self::apply_filters])]
    tray_apply_filters: nwg::MenuItem,

    #[nwg_control]
    tray_sep2: nwg::MenuSeparator,

    #[nwg_control(text: "Exit")]
    #[nwg_events(OnMenuItemSelected: [Self::exit])]
    tray_exit: nwg::MenuItem,
}
/// Handle menu clicks.
impl BottomMenuItems {
    forward_to_dynamic_ui!(tray_ui => apply_filters, stop_flashing_windows, exit);

    fn open_filter_config(&self) {
        let Some(tray_ui) = self.tray_ui.get() else {
            return;
        };
        tray_ui.configure_filters(true);
    }
}
impl DynamicUiHooks<SystemTray> for BottomMenuItems {
    fn before_partial_build(
        &mut self,
        tray_ui: &Rc<SystemTray>,
        _should_build: &mut bool,
    ) -> Option<(nwg::ControlHandle, TypeId)> {
        self.tray_ui.set(tray_ui);
        Some((tray_ui.root().tray_menu.handle, TypeId::of::<TrayRoot>()))
    }
}
impl TrayPlugin for BottomMenuItems {}
