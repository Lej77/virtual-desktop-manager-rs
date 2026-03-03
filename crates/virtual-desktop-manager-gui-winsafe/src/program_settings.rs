use crate::layout::LayoutArea;
use crate::{custom_msg, GuiParentWithEvents, NativeWindowHandle};
use std::cell::{Cell, OnceCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::Arc;
use virtual_desktop_manager_core::settings::{
    AutoStart, QuickSwitchMenu, TrayClickAction, TrayIconType, UiSettings,
};
use winsafe::co;
use winsafe::gui;
use winsafe::msg::wm::SetFont;
use winsafe::msg::WndMsg;
use winsafe::prelude::*;

/// Be notified when the [`ProgramSettingsPanel`] changes.
pub trait ProgramSettingsHooks: Clone + 'static {
    /// An option for the selected filter has been modified.
    fn on_setting_change(&self);
}

pub struct ProgramSettingsPanel {
    start_as_admin_checkbox: gui::CheckBox,
    auto_start_label: gui::Label,
    auto_start_combobox: gui::ComboBox,
    prevent_flashing_checkbox: gui::CheckBox,
    smooth_switch_checkbox: gui::CheckBox,
    tray_icon_label: gui::Label,
    tray_icon_combobox: gui::ComboBox,
    quick_switch_label: gui::Label,
    quick_switch_combobox: gui::ComboBox,
    quick_switch_shortcuts_label: gui::Label,
    quick_switch_shortcuts_input: gui::Edit,
    quick_switch_shortcuts_recursively_checkbox: gui::CheckBox,
    quick_switch_hotkey_label: gui::Label,
    quick_switch_hotkey_input: gui::Edit,
    quick_switch_hotkey_status: gui::Edit,
    tray_left_click_label: gui::Label,
    tray_left_click_combobox: gui::ComboBox,
    tray_middle_click_label: gui::Label,
    tray_middle_click_combobox: gui::ComboBox,
    menu_at_mouse_pos_hotkey_label_layout: gui::Label,
    menu_at_mouse_pos_hotkey_input: gui::Edit,
    menu_at_mouse_pos_hotkey_status: gui::Edit,
    tooltips: OnceCell<nwg::Tooltip>,
    is_manually_setting: Cell<u32>,
}
impl ProgramSettingsPanel {
    pub fn new<P, H>(parent: &P, layout: &mut LayoutArea, hooks: H) -> Rc<Self>
    where
        P: GuiParentWithEvents,
        H: ProgramSettingsHooks,
    {
        let label_height = 20;
        let input_height = 30;
        let multi_input_height = 90;
        let button_height = 30;
        let checkbox_height = 22;

        let start_as_admin_layout = layout.take_top(checkbox_height);
        let start_as_admin_checkbox = gui::CheckBox::new(
            parent,
            gui::CheckBoxOpts {
                text: "Start program with admin rights",
                position: start_as_admin_layout.dpi_pos(),
                size: start_as_admin_layout.dpi_size(),
                ..Default::default()
            },
        );

        let auto_start_label_layout = layout.take_top(label_height);
        let auto_start_label = gui::Label::new(
            parent,
            gui::LabelOpts {
                text: "Auto start with Windows:",
                position: auto_start_label_layout.dpi_pos(),
                size: auto_start_label_layout.dpi_size(),
                ..Default::default()
            },
        );

        let auto_start_combobox_layout =
            layout.take_top_with_margin(button_height, layout.margin + layout.margin / 2);
        let auto_start_combobox = gui::ComboBox::new(
            parent,
            gui::ComboBoxOpts {
                position: auto_start_combobox_layout.dpi_pos(),
                width: auto_start_combobox_layout.dpi_width(),
                items: &AutoStart::ALL
                    .into_iter()
                    .map(|action| action.as_str())
                    .collect::<Vec<_>>(),
                selected_item: Some(0),
                ..Default::default()
            },
        );

        let prevent_flashing_layout =
            layout.take_top_with_margin(checkbox_height, layout.margin / 2);
        let prevent_flashing_checkbox = gui::CheckBox::new(
            parent,
            gui::CheckBoxOpts {
                text: "Prevent flashing windows",
                position: prevent_flashing_layout.dpi_pos(),
                size: prevent_flashing_layout.dpi_size(),
                ..Default::default()
            },
        );

        // Extra space from previous controls (new grouping)
        layout.take_top_with_margin(layout.margin, 0);

        let smooth_switch_layout = layout.take_top(checkbox_height);
        let smooth_switch_checkbox = gui::CheckBox::new(
            parent,
            gui::CheckBoxOpts {
                text: "Smoothly switch virtual desktop",
                position: smooth_switch_layout.dpi_pos(),
                size: smooth_switch_layout.dpi_size(),
                ..Default::default()
            },
        );

        let tray_icon_label_layout = layout.take_top(label_height);
        let tray_icon_label = gui::Label::new(
            parent,
            gui::LabelOpts {
                text: "Tray icon:",
                position: tray_icon_label_layout.dpi_pos(),
                size: tray_icon_label_layout.dpi_size(),
                ..Default::default()
            },
        );

        let tray_icon_combobox_layout = layout.take_top(button_height);
        let tray_icon_combobox = gui::ComboBox::new(
            parent,
            gui::ComboBoxOpts {
                position: tray_icon_combobox_layout.dpi_pos(),
                width: tray_icon_combobox_layout.dpi_width(),
                items: &TrayIconType::ALL
                    .into_iter()
                    .map(|action| action.as_str())
                    .collect::<Vec<_>>(),
                selected_item: Some(0),
                ..Default::default()
            },
        );

        let quick_switch_label_layout = layout.take_top(label_height);
        let quick_switch_label = gui::Label::new(
            parent,
            gui::LabelOpts {
                text: "Quick switch context menu:",
                position: quick_switch_label_layout.dpi_pos(),
                size: quick_switch_label_layout.dpi_size(),
                ..Default::default()
            },
        );

        let quick_switch_combobox_layout = layout.take_top(button_height);
        let quick_switch_combobox = gui::ComboBox::new(
            parent,
            gui::ComboBoxOpts {
                position: quick_switch_combobox_layout.dpi_pos(),
                width: quick_switch_combobox_layout.dpi_width(),
                items: &QuickSwitchMenu::ALL
                    .into_iter()
                    .map(|action| action.as_str())
                    .collect::<Vec<_>>(),
                selected_item: Some(0),
                ..Default::default()
            },
        );

        layout.take_top_with_margin(layout.margin / 2, 0);
        let quick_switch_shortcuts_label_layout = layout.take_top(label_height);
        let quick_switch_shortcuts_label = gui::Label::new(
            parent,
            gui::LabelOpts {
                text: "Quick switch menu shortcuts:",
                position: quick_switch_shortcuts_label_layout.dpi_pos(),
                size: quick_switch_shortcuts_label_layout.dpi_size(),
                ..Default::default()
            },
        );

        let quick_switch_shortcuts_layout =
            layout.take_top_with_margin(multi_input_height, layout.margin * 3 / 2);
        let quick_switch_shortcuts_input = gui::Edit::new(
            parent,
            gui::EditOpts {
                position: quick_switch_shortcuts_layout.dpi_pos(),
                height: quick_switch_shortcuts_layout.dpi_height(),
                width: quick_switch_shortcuts_layout.dpi_width(),
                control_style: co::ES::MULTILINE
                    | co::ES::WANTRETURN
                    | co::ES::AUTOVSCROLL
                    | co::ES::AUTOHSCROLL
                    | co::ES::NOHIDESEL,
                window_style: gui::EditOpts::default().window_style
                    | co::WS::VSCROLL
                    | co::WS::HSCROLL,
                ..Default::default()
            },
        );

        let quick_switch_shortcuts_recursively_layout = layout.take_top(checkbox_height);
        let quick_switch_shortcuts_recursively_checkbox = gui::CheckBox::new(
            parent,
            gui::CheckBoxOpts {
                text: "Quick shortcuts in submenus",
                position: quick_switch_shortcuts_recursively_layout.dpi_pos(),
                size: quick_switch_shortcuts_recursively_layout.dpi_size(),
                ..Default::default()
            },
        );

        layout.take_top_with_margin(layout.margin, 0); // extra space

        let quick_switch_hotkey_label_layout = layout.take_top(label_height);
        let quick_switch_hotkey_label = gui::Label::new(
            parent,
            gui::LabelOpts {
                text: "Global hotkey for quick switch:",
                position: quick_switch_hotkey_label_layout.dpi_pos(),
                size: quick_switch_hotkey_label_layout.dpi_size(),
                ..Default::default()
            },
        );

        let quick_switch_hotkey_input_layout = layout.take_top(input_height);
        let quick_switch_hotkey_input = gui::Edit::new(
            parent,
            gui::EditOpts {
                position: quick_switch_hotkey_input_layout.dpi_pos(),
                width: quick_switch_hotkey_input_layout.dpi_width(),
                height: quick_switch_hotkey_input_layout.dpi_height(),
                ..Default::default()
            },
        );

        let quick_switch_hotkey_status_layout = layout.take_top(input_height + /* scrollbar: */ 15);
        let quick_switch_hotkey_status = gui::Edit::new(
            parent,
            gui::EditOpts {
                position: quick_switch_hotkey_status_layout.dpi_pos(),
                width: quick_switch_hotkey_status_layout.dpi_width(),
                height: quick_switch_hotkey_status_layout.dpi_height(),
                control_style: gui::EditOpts::default().control_style | co::ES::READONLY,
                window_style: gui::EditOpts::default().window_style | co::WS::HSCROLL,
                ..Default::default()
            },
        );

        layout.take_top_with_margin(layout.margin, 0); // extra space

        let tray_left_click_label_layout = layout.take_top(label_height);
        let tray_left_click_label = gui::Label::new(
            parent,
            gui::LabelOpts {
                text: "Left click on tray icon:",
                position: tray_left_click_label_layout.dpi_pos(),
                size: tray_left_click_label_layout.dpi_size(),
                ..Default::default()
            },
        );

        let tray_left_click_combobox_layout = layout.take_top(button_height);
        let tray_left_click_combobox = gui::ComboBox::new(
            parent,
            gui::ComboBoxOpts {
                position: tray_left_click_combobox_layout.dpi_pos(),
                width: tray_left_click_combobox_layout.dpi_width(),
                items: &TrayClickAction::ALL
                    .into_iter()
                    .map(|action| action.as_str())
                    .collect::<Vec<_>>(),
                selected_item: TrayClickAction::ALL
                    .iter()
                    .position(|&i| i == TrayClickAction::ToggleConfigurationWindow)
                    .map(|i| i as _),
                ..Default::default()
            },
        );

        let tray_middle_click_label_layout = layout.take_top(label_height);
        let tray_middle_click_label = gui::Label::new(
            parent,
            gui::LabelOpts {
                text: "Middle click on tray icon:",
                position: tray_middle_click_label_layout.dpi_pos(),
                size: tray_middle_click_label_layout.dpi_size(),
                ..Default::default()
            },
        );

        let tray_middle_click_combobox_layout = layout.take_top(button_height);
        let tray_middle_click_combobox = gui::ComboBox::new(
            parent,
            gui::ComboBoxOpts {
                position: tray_middle_click_combobox_layout.dpi_pos(),
                width: tray_middle_click_combobox_layout.dpi_width(),
                items: &TrayClickAction::ALL
                    .into_iter()
                    .map(|action| action.as_str())
                    .collect::<Vec<_>>(),
                selected_item: TrayClickAction::ALL
                    .iter()
                    .position(|&i| i == TrayClickAction::ApplyFilters)
                    .map(|i| i as _),
                ..Default::default()
            },
        );

        layout.take_top_with_margin(layout.margin, 0); // extra space

        let menu_at_mouse_pos_hotkey_label_layout = layout.take_top(label_height * 2);
        let menu_at_mouse_pos_hotkey_label_layout = gui::Label::new(
            parent,
            gui::LabelOpts {
                text: "Global hotkey to open context\r\nmenu at current mouse position:",
                position: menu_at_mouse_pos_hotkey_label_layout.dpi_pos(),
                size: menu_at_mouse_pos_hotkey_label_layout.dpi_size(),
                ..Default::default()
            },
        );

        let menu_at_mouse_pos_hotkey_input_layout = layout.take_top(input_height);
        let menu_at_mouse_pos_hotkey_input = gui::Edit::new(
            parent,
            gui::EditOpts {
                position: menu_at_mouse_pos_hotkey_input_layout.dpi_pos(),
                width: menu_at_mouse_pos_hotkey_input_layout.dpi_width(),
                height: menu_at_mouse_pos_hotkey_input_layout.dpi_height(),
                ..Default::default()
            },
        );

        let menu_at_mouse_pos_hotkey_status_layout =
            layout.take_top(input_height + /* scrollbar: */ 15);
        let menu_at_mouse_pos_hotkey_status = gui::Edit::new(
            parent,
            gui::EditOpts {
                position: menu_at_mouse_pos_hotkey_status_layout.dpi_pos(),
                width: menu_at_mouse_pos_hotkey_status_layout.dpi_width(),
                height: menu_at_mouse_pos_hotkey_status_layout.dpi_height(),
                control_style: gui::EditOpts::default().control_style | co::ES::READONLY,
                window_style: gui::EditOpts::default().window_style | co::WS::HSCROLL,
                ..Default::default()
            },
        );

        let new_self = Rc::new(Self {
            start_as_admin_checkbox,
            auto_start_label,
            auto_start_combobox,
            prevent_flashing_checkbox,
            smooth_switch_checkbox,
            tray_icon_label,
            tray_icon_combobox,
            quick_switch_label,
            quick_switch_combobox,
            quick_switch_shortcuts_label,
            quick_switch_shortcuts_input,
            quick_switch_shortcuts_recursively_checkbox,
            quick_switch_hotkey_label,
            quick_switch_hotkey_input,
            quick_switch_hotkey_status,
            tray_left_click_label,
            tray_left_click_combobox,
            tray_middle_click_label,
            tray_middle_click_combobox,
            menu_at_mouse_pos_hotkey_label_layout,
            menu_at_mouse_pos_hotkey_input,
            menu_at_mouse_pos_hotkey_status,
            tooltips: OnceCell::new(),
            is_manually_setting: Cell::new(0),
        });
        new_self.events(parent, hooks);
        new_self
    }

    pub fn set_font(&self, msg: &mut SetFont) {
        let handles = [
            self.start_as_admin_checkbox.hwnd(),
            self.auto_start_label.hwnd(),
            self.auto_start_combobox.hwnd(),
            self.prevent_flashing_checkbox.hwnd(),
            self.smooth_switch_checkbox.hwnd(),
            self.tray_icon_label.hwnd(),
            self.tray_icon_combobox.hwnd(),
            self.quick_switch_label.hwnd(),
            self.quick_switch_combobox.hwnd(),
            self.quick_switch_shortcuts_label.hwnd(),
            self.quick_switch_shortcuts_input.hwnd(),
            self.quick_switch_shortcuts_recursively_checkbox.hwnd(),
            self.quick_switch_hotkey_label.hwnd(),
            self.quick_switch_hotkey_input.hwnd(),
            self.quick_switch_hotkey_status.hwnd(),
            self.tray_left_click_label.hwnd(),
            self.tray_left_click_combobox.hwnd(),
            self.tray_middle_click_label.hwnd(),
            self.tray_middle_click_combobox.hwnd(),
            self.menu_at_mouse_pos_hotkey_label_layout.hwnd(),
            self.menu_at_mouse_pos_hotkey_input.hwnd(),
            self.menu_at_mouse_pos_hotkey_status.hwnd(),
        ];
        for handle in handles {
            unsafe { handle.SendMessage(msg.as_generic_wm()) };
        }
    }

    fn post_change(&self, parent: &impl GuiParentWithEvents) {
        if self.is_manually_setting.get() > 0 {
            return;
        }
        let result = unsafe {
            parent.hwnd().PostMessage(WndMsg {
                msg_id: custom_msg::WM_SETTING_CHANGED,
                wparam: 0,
                lparam: 0,
            })
        };
        if let Err(e) = result {
            tracing::error!(error = ?e, "Failed to post setting change message");
        }
    }

    fn suppress_events(&self) -> SuppressChangeEvents<'_> {
        SuppressChangeEvents::new(self)
    }

    fn events<P, H>(self: &Rc<Self>, parent: &P, hooks: H)
    where
        P: GuiParentWithEvents,
        H: ProgramSettingsHooks,
    {
        parent.on().wm_create({
            let this = self.clone();
            move |_| {
                this.update_hotkey_status();
                this.build_tooltips();
                Ok(0)
            }
        });

        let inputs = [
            &self.quick_switch_shortcuts_input,
            &self.menu_at_mouse_pos_hotkey_input,
            &self.quick_switch_hotkey_input,
        ];
        for input in inputs {
            let this = self.clone();
            let parent = parent.clone();
            input.on().en_change(move || {
                this.update_hotkey_status();
                Self::post_change(&this, &parent);
                Ok(())
            });
        }

        let checkboxes = [
            &self.start_as_admin_checkbox,
            &self.prevent_flashing_checkbox,
            &self.smooth_switch_checkbox,
            &self.quick_switch_shortcuts_recursively_checkbox,
        ];
        for checkbox in checkboxes {
            let this = self.clone();
            let parent = parent.clone();
            checkbox.on().bn_clicked(move || {
                Self::post_change(&this, &parent);
                Ok(())
            });
        }

        let combo_boxes = [
            &self.auto_start_combobox,
            &self.tray_icon_combobox,
            &self.quick_switch_combobox,
            &self.tray_left_click_combobox,
            &self.tray_middle_click_combobox,
        ];
        for combo in combo_boxes {
            let this = self.clone();
            let parent = parent.clone();
            combo.on().cbn_sel_change(move || {
                Self::post_change(&this, &parent);
                Ok(())
            });
        }

        // Delayed change events (so that reading values from controls will get the latest values):
        parent.on().wm(custom_msg::WM_SETTING_CHANGED, {
            let hooks = hooks.clone();
            move |_msg| {
                hooks.on_setting_change();
                Ok(0)
            }
        });
    }

    fn update_hotkey_status(&self) {
        let hotkeys = [
            (
                &self.quick_switch_hotkey_input,
                &self.quick_switch_hotkey_status,
            ),
            (
                &self.menu_at_mouse_pos_hotkey_input,
                &self.menu_at_mouse_pos_hotkey_status,
            ),
        ];
        for (input, status_input) in hotkeys {
            match input.text() {
                Err(e) => {
                    tracing::error!(error = ?e, "Failed to get hotkey input text");
                }
                Ok(hotkey_text) => {
                    let status_text = if hotkey_text.is_empty() {
                        "Hotkey disabled".to_owned()
                    } else {
                        #[cfg(feature = "global_hotkey")]
                        {
                            match global_hotkey::hotkey::HotKey::from_str(&hotkey_text) {
                                Ok(_) => "Valid hotkey".to_owned(),
                                Err(e) => format!("Invalid hotkey: {e}"),
                            }
                        }
                        #[cfg(not(feature = "global_hotkey"))]
                        {
                            "Compiled without hotkey support".to_owned()
                        }
                    };
                    let result = status_input.set_text(format!("Status: {status_text}").as_str());
                    if let Err(e) = result {
                        tracing::error!(error = ?e, "Failed to update hotkey status");
                    }
                }
            }
        }
    }

    fn build_tooltips(&self) {
        if self.tooltips.get().is_some() {
            return;
        }
        let mut tooltip = nwg::Tooltip::default();
        let result = nwg::Tooltip::builder()
            .register(
                self.start_as_admin_checkbox.native_handle(),
                "This is useful in order to move windows owned by other \
                programs that have admin rights.",
            )
            .register(
                self.prevent_flashing_checkbox.native_handle(),
                "Some windows can try to grab attention by flashing their \
                icon in the taskbar, this option suppresses such flashing right \
                after window filters are applied.",
            )
            .register(
                self.smooth_switch_checkbox.native_handle(),
                "Enable for this program to use animations when changing \
                the current virtual desktop.",
            )
            .register(
                self.quick_switch_shortcuts_label.native_handle(),
                "Each line should have a letter or symbol followed by a zero-based \
                virtual desktop index. For each line an extra context menu item will \
                be created in the quick switch menu with that symbol as its access key.",
            )
            .register(
                self.quick_switch_shortcuts_recursively_checkbox
                    .native_handle(),
                "If checked then extra context menu items for quick switch shortcuts \
                will be created in each submenu of the quick switch menu when there are \
                more than 9 virtual desktops.",
            )
            .register(
                self.tray_middle_click_label.native_handle(),
                "Controls the action that will be preformed when the tray icon \
                is middle clicked. On some Windows 11 versions middle clicks are \
                registered as left clicks.",
            )
            .build(&mut tooltip);
        if let Err(e) = result {
            tracing::error!(error = ?e, "Failed to build tooltips for ProgramSettingsPanel");
        } else {
            tracing::debug!("Built tooltips for ProgramSettingsPanel");
            _ = self.tooltips.set(tooltip);
        }
    }
}
/// Retrieve data related to the selected [`UiSettings`].
impl ProgramSettingsPanel {
    pub fn get_request_admin_at_startup(&self) -> bool {
        self.start_as_admin_checkbox.is_checked()
    }
    pub fn get_auto_start(&self) -> AutoStart {
        self.auto_start_combobox
            .items()
            .selected_index()
            .map(|index| AutoStart::ALL[index as usize])
            .unwrap_or_default()
    }
    pub fn get_stop_flashing_windows_after_applying_filter(&self) -> bool {
        self.prevent_flashing_checkbox.is_checked()
    }
    pub fn get_smooth_switch_desktops(&self) -> bool {
        self.smooth_switch_checkbox.is_checked()
    }
    pub fn get_tray_icon_type(&self) -> TrayIconType {
        self.tray_icon_combobox
            .items()
            .selected_index()
            .map(|index| TrayIconType::ALL[index as usize])
            .unwrap_or_default()
    }
    pub fn get_quick_switch_menu(&self) -> QuickSwitchMenu {
        self.quick_switch_combobox
            .items()
            .selected_index()
            .map(|index| QuickSwitchMenu::ALL[index as usize])
            .unwrap_or_default()
    }
    pub fn get_quick_switch_menu_shortcuts(
        &self,
    ) -> Result<Arc<BTreeMap<String, u32>>, Arc<BTreeMap<String, u32>>> {
        let text = self.quick_switch_shortcuts_input.text().unwrap_or_else(|e| {
            tracing::error!(error = ?e, "failed to read quick switch menu shortcuts text field");
            String::new()
        });

        let mut quick_shortcuts_count = 0;
        let mut invalid_quick_shortcut_target = false;
        let parsed = Arc::new(
            text.split('\n')
                .filter_map(|text| {
                    // remove \r at end of line (might be a letter after it if
                    // the cursor was placed between the \r and \n):
                    let text = text.trim_end_matches('\r');
                    if text.contains('\r') {
                        // cursor was likely after the \r and wrote something,
                        // move the cursor back
                        invalid_quick_shortcut_target = true;
                    }
                    let text = text.replace('\r', "");
                    if text.is_empty() {
                        return None;
                    }
                    let (target, key): (String, String) =
                        text.chars().partition(char::is_ascii_digit);
                    let target = if target.is_empty() {
                        // No target number
                        invalid_quick_shortcut_target = true;
                        0
                    } else {
                        u32::try_from(
                            target
                                .parse::<i64>()
                                .unwrap_or_else(|_| {
                                    // Invalid target, maybe trailing non-digits
                                    invalid_quick_shortcut_target = true;
                                    0
                                })
                                .abs(),
                        )
                        .unwrap_or_else(|_| {
                            // Too many digits:
                            invalid_quick_shortcut_target = true;
                            u32::MAX
                        })
                    };
                    Some((key, target))
                })
                .inspect(|_| {
                    quick_shortcuts_count += 1;
                })
                .collect::<BTreeMap<_, _>>(),
        );

        if invalid_quick_shortcut_target || quick_shortcuts_count != parsed.len() {
            // Had duplicates
            tracing::info!(
                invalid_entry = invalid_quick_shortcut_target,
                line_count = quick_shortcuts_count,
                deduplicated_entries_count = parsed.len(),
                "Invalid numbers or duplicated items in quick switch shortcuts field"
            );
            Err(parsed)
        } else {
            Ok(parsed)
        }
    }
    pub fn get_quick_switch_menu_shortcuts_only_in_root(&self) -> bool {
        !self
            .quick_switch_shortcuts_recursively_checkbox
            .is_checked()
    }
    pub fn get_quick_switch_hotkey(&self) -> String {
        self.quick_switch_hotkey_input.text().unwrap_or_else(|e| {
            tracing::error!(error = ?e, "failed to read quick switch hotkey input");
            String::new()
        })
    }
    pub fn get_open_menu_at_mouse_pos_hotkey(&self) -> String {
        self.menu_at_mouse_pos_hotkey_input
            .text()
            .unwrap_or_else(|e| {
                tracing::error!(error = ?e, "failed to read menu at mouse position hotkey input");
                String::new()
            })
    }
    pub fn get_left_click(&self) -> TrayClickAction {
        self.tray_left_click_combobox
            .items()
            .selected_index()
            .map(|index| TrayClickAction::ALL[index as usize])
            .unwrap_or_default()
    }
    pub fn get_middle_click(&self) -> TrayClickAction {
        self.tray_middle_click_combobox
            .items()
            .selected_index()
            .map(|index| TrayClickAction::ALL[index as usize])
            .unwrap_or_default()
    }

    pub fn get_settings_data(&self, quick_switch_menu_shortcuts_error: &mut bool) -> UiSettings {
        UiSettings {
            version: UiSettings::CURRENT_VERSION,
            auto_start: self.get_auto_start(),
            smooth_switch_desktops: self.get_smooth_switch_desktops(),
            request_admin_at_startup: self.get_request_admin_at_startup(),
            stop_flashing_windows_after_applying_filter: self
                .get_stop_flashing_windows_after_applying_filter(),
            tray_icon_type: self.get_tray_icon_type(),
            quick_switch_menu: self.get_quick_switch_menu(),
            quick_switch_menu_shortcuts: self.get_quick_switch_menu_shortcuts().unwrap_or_else(
                |deduplicated| {
                    *quick_switch_menu_shortcuts_error = true;
                    deduplicated
                },
            ),
            quick_switch_menu_shortcuts_only_in_root: self
                .get_quick_switch_menu_shortcuts_only_in_root(),
            quick_switch_hotkey: Arc::from(self.get_quick_switch_hotkey().as_str()),
            open_menu_at_mouse_pos_hotkey: Arc::from(
                self.get_open_menu_at_mouse_pos_hotkey().as_str(),
            ),
            left_click: self.get_left_click(),
            middle_click: self.get_middle_click(),
            config_window: Default::default(),
            filters: Arc::new([]),
        }
    }
}
/// Set data related to the selected [`UiSettings`].
impl ProgramSettingsPanel {
    pub fn set_request_admin_at_startup(&self, value: bool) {
        if self.get_request_admin_at_startup() == value {
            return;
        }
        let _suppress = self.suppress_events();
        self.start_as_admin_checkbox.set_check(value)
    }
    pub fn set_auto_start(&self, value: AutoStart) {
        if self.get_auto_start() == value {
            return;
        }
        let _suppress = self.suppress_events();
        self.auto_start_combobox.items().select(
            AutoStart::ALL
                .iter()
                .position(|&v| v == value)
                .map(|pos| pos as u32),
        )
    }
    pub fn set_stop_flashing_windows_after_applying_filter(&self, value: bool) {
        if self.get_stop_flashing_windows_after_applying_filter() == value {
            return;
        }
        let _suppress = self.suppress_events();
        self.prevent_flashing_checkbox.set_check(value);
    }
    pub fn set_smooth_switch_desktops(&self, value: bool) {
        if self.get_smooth_switch_desktops() == value {
            return;
        }
        let _suppress = self.suppress_events();
        self.smooth_switch_checkbox.set_check(value);
    }
    pub fn set_tray_icon_type(&self, value: TrayIconType) {
        if self.get_tray_icon_type() == value {
            return;
        }
        let _suppress = self.suppress_events();
        self.tray_icon_combobox.items().select(
            TrayIconType::ALL
                .iter()
                .position(|&v| v == value)
                .map(|pos| pos as u32),
        )
    }
    pub fn set_quick_switch_menu(&self, value: QuickSwitchMenu) {
        if self.get_quick_switch_menu() == value {
            return;
        }
        let _suppress = self.suppress_events();
        self.quick_switch_combobox.items().select(
            QuickSwitchMenu::ALL
                .iter()
                .position(|&v| v == value)
                .map(|pos| pos as u32),
        )
    }
    pub fn set_quick_switch_menu_shortcuts(&self, value: &BTreeMap<String, u32>) {
        if matches!(self.get_quick_switch_menu_shortcuts(), Ok(old) if *old == *value) {
            return;
        }

        let _suppress = self.suppress_events();

        // remember selection:
        let (mut first_index, mut past_last_index) = (0, 0);
        unsafe {
            self.quick_switch_shortcuts_input
                .hwnd()
                .SendMessage(WndMsg {
                    msg_id: co::EM::GETSEL.into(),
                    wparam: (&mut first_index as *mut u32) as usize,
                    lparam: (&mut past_last_index as *mut u32) as isize,
                })
        };

        let text = value
            .iter()
            .fold(String::with_capacity(20), |mut f, (mut key, target)| {
                // Don't write any extra newlines (could cause issues if they don't have the extra \r):
                let newlines = ['\r', '\n'];
                let new_key;
                if key.contains(newlines) {
                    new_key = key.replace(newlines, "");
                    key = &new_key;
                }

                use std::fmt::Write;
                write!(f, "{}{}\r\n", key, target)
                    .expect("should succeed at writing to in-memory string");
                f
            });
        if let Err(e) = self.quick_switch_shortcuts_input.set_text(&text) {
            tracing::error!(error =? e, "Failed to update quick_switch_shortcuts input field");
            return;
        }

        let mut selection =
            first_index.min(text.len() as u32)..past_last_index.min(text.len() as u32);

        // Previous cursor position might now be in the middle of a \r\n, so check for that:
        let Some(selected_and_prev) = text
            .as_bytes()
            .get((selection.start as usize).saturating_sub(1)..(selection.end as usize))
        else {
            tracing::warn!(
                ?selection,
                text,
                "Selection was over invalid characters so can't update it"
            );
            return;
        };
        tracing::debug!(
            new_text =? text,
            selected_and_prev =? String::from_utf8_lossy(selected_and_prev),
            selection_range =? selection,
            "Updating selection of quick switch shortcut text box"
        );
        if selected_and_prev.starts_with(b"\r") {
            selection.start = selection.start.saturating_sub(1);
            if selected_and_prev.len() == 1 {
                selection.end = selection.end.saturating_sub(1);
            }
        }
        if selected_and_prev.len() > 1 && selected_and_prev.ends_with(b"\r") {
            selection.end = selection.end.saturating_add(1).min(text.len() as u32);
        }
        self.quick_switch_shortcuts_input
            .set_selection(selection.start as i32, selection.end as i32)
    }
    pub fn set_quick_switch_menu_shortcuts_only_in_root(&self, value: bool) {
        if self.get_quick_switch_menu_shortcuts_only_in_root() == value {
            return;
        }
        let _suppress = self.suppress_events();
        self.quick_switch_shortcuts_recursively_checkbox
            .set_check(!value)
    }
    pub fn set_quick_switch_hotkey(&self, value: &str) {
        if self.get_quick_switch_hotkey() == value {
            return;
        }
        let _suppress = self.suppress_events();
        if let Err(e) = self.quick_switch_hotkey_input.set_text(value) {
            tracing::error!(error = ?e, "Failed to set quick switch hotkey input field");
        }
    }
    pub fn set_open_menu_at_mouse_pos_hotkey(&self, value: &str) {
        if self.get_open_menu_at_mouse_pos_hotkey() == value {
            return;
        }
        let _suppress = self.suppress_events();
        if let Err(e) = self.menu_at_mouse_pos_hotkey_input.set_text(value) {
            tracing::error!(error = ?e, "Failed to set \"read menu at mouse position\" hotkey input field");
        }
    }
    pub fn set_left_click(&self, value: TrayClickAction) {
        if self.get_left_click() == value {
            return;
        }
        let _suppress = self.suppress_events();
        self.tray_left_click_combobox.items().select(
            TrayClickAction::ALL
                .iter()
                .position(|&v| v == value)
                .map(|pos| pos as u32),
        )
    }
    pub fn set_middle_click(&self, value: TrayClickAction) {
        if self.get_middle_click() == value {
            return;
        }
        let _suppress = self.suppress_events();
        self.tray_middle_click_combobox.items().select(
            TrayClickAction::ALL
                .iter()
                .position(|&v| v == value)
                .map(|pos| pos as u32),
        )
    }

    pub fn set_settings_data(&self, data: &UiSettings) {
        let &UiSettings {
            version: _,
            auto_start,
            smooth_switch_desktops,
            request_admin_at_startup,
            stop_flashing_windows_after_applying_filter,
            tray_icon_type,
            quick_switch_menu,
            ref quick_switch_menu_shortcuts,
            quick_switch_menu_shortcuts_only_in_root,
            ref quick_switch_hotkey,
            ref open_menu_at_mouse_pos_hotkey,
            left_click,
            middle_click,
            config_window: _,
            filters: _,
        } = data;

        self.set_auto_start(auto_start);
        self.set_smooth_switch_desktops(smooth_switch_desktops);
        self.set_request_admin_at_startup(request_admin_at_startup);
        self.set_stop_flashing_windows_after_applying_filter(
            stop_flashing_windows_after_applying_filter,
        );
        self.set_tray_icon_type(tray_icon_type);
        self.set_quick_switch_menu(quick_switch_menu);
        self.set_quick_switch_menu_shortcuts(quick_switch_menu_shortcuts);
        self.set_quick_switch_menu_shortcuts_only_in_root(quick_switch_menu_shortcuts_only_in_root);
        self.set_quick_switch_hotkey(quick_switch_hotkey);
        self.set_open_menu_at_mouse_pos_hotkey(open_menu_at_mouse_pos_hotkey);
        self.set_left_click(left_click);
        self.set_middle_click(middle_click);
    }
}

struct SuppressChangeEvents<'a>(&'a ProgramSettingsPanel);
impl<'a> SuppressChangeEvents<'a> {
    pub fn new(settings: &'a ProgramSettingsPanel) -> SuppressChangeEvents<'a> {
        settings
            .is_manually_setting
            .update(|prev| prev.checked_add(1).unwrap());
        SuppressChangeEvents(settings)
    }
}
impl Drop for SuppressChangeEvents<'_> {
    fn drop(&mut self) {
        self.0
            .is_manually_setting
            .update(|prev| prev.saturating_sub(1));
    }
}
