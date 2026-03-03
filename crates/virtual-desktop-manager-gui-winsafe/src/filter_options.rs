use crate::layout::LayoutArea;
use crate::{custom_msg, GuiParentWithEvents};
use std::rc::Rc;
use virtual_desktop_manager_core::window_filter::{
    FilterAction, IntegerRange, TextPattern, WindowFilter,
};
use winsafe::co;
use winsafe::gui;
use winsafe::msg::wm::SetFont;
use winsafe::msg::WndMsg;
use winsafe::prelude::*;

/// Be notified when the [`FilterOptionsPanel`] requests that a new filter be created/loaded or if
/// the existing filter should be modified, deleted or reordered.
pub trait FilterOptionsHooks: Clone + 'static {
    /// An option for the selected filter has been modified.
    fn on_option_change(&self, change: FilterChange);
    /// A new filter index has been selected and should therefore be loaded.
    fn on_index_change(&self);
    /// Reorder the selected filter earlier in the filter list.
    fn on_move_up(&self);
    /// Reorder the selected filter later in the filter list.
    fn on_move_down(&self);
    /// Create a new filter and select it.
    fn on_create_new(&self);
    /// Delete the currently selected filter.
    fn on_delete(&self);
}

/// Information about a change, contains the previous value for the changed option.
#[derive(Debug, Clone)]
pub enum FilterChange {
    WindowRange = 0,
    DesktopRange = 1,
    WindowTitle = 2,
    ProcessName = 3,
    Action = 4,
    TargetDesktop = 5,
}
impl TryFrom<i64> for FilterChange {
    type Error = ();
    fn try_from(value: i64) -> Result<Self, ()> {
        Ok(match value {
            0 => FilterChange::WindowRange,
            1 => FilterChange::DesktopRange,
            2 => FilterChange::WindowTitle,
            3 => FilterChange::ProcessName,
            4 => FilterChange::Action,
            5 => FilterChange::TargetDesktop,
            _ => return Err(()),
        })
    }
}

#[derive(Clone)]
pub struct FilterOptionsPanel {
    selected_filter_index_label: gui::Label,
    selected_filter_index_input: gui::Edit,
    /// one based index, so 0 means no filter selected.
    selected_filter_index_up_down: gui::UpDown,
    btn_create: gui::Button,
    btn_delete: gui::Button,
    btn_move_up: gui::Button,
    btn_move_down: gui::Button,
    window_index_label: gui::Label,
    window_index_range: Rc<RangeControl>,
    virtual_desktop_index_label: gui::Label,
    virtual_desktop_index_range: Rc<RangeControl>,
    window_title_label: gui::Label,
    window_title_input: gui::Edit,
    process_name_label: gui::Label,
    process_name_input: gui::Edit,
    action_label: gui::Label,
    action: gui::ComboBox,
    target_desktop_label: gui::Label,
    target_desktop_input: gui::Edit,
    target_desktop_up_down: gui::UpDown,
}
/// GUI concerns.
impl FilterOptionsPanel {
    const CTRL_ID_SELECTED_FILTER: u16 = 1010;
    const CTRL_ID_WINDOW_INDEX_LOWER: u16 = 1020;
    const CTRL_ID_WINDOW_INDEX_UPPER: u16 = 1021;
    const CTRL_ID_DESKTOP_INDEX_LOWER: u16 = 1030;
    const CTRL_ID_DESKTOP_INDEX_UPPER: u16 = 1031;
    const CTRL_ID_TARGET_DESKTOP_INDEX: u16 = 1040;

    pub fn new(
        parent: &(impl GuiParentWithEvents + 'static),
        layout: &mut LayoutArea,
        hooks: impl FilterOptionsHooks,
    ) -> Rc<Self> {
        let label_height = 20;
        let input_height = 25;
        let button_height = 30;

        let selected_filter_index_label = gui::Label::new(
            parent,
            gui::LabelOpts {
                text: "Selected filter index:",
                position: layout.dpi_pos(),
                size: (layout.dpi_width(), gui::dpi_y(label_height)),
                ..Default::default()
            },
        );
        layout.take_top(label_height);

        let selected_filter_index_input_layout = layout.take_top(input_height);
        let selected_filter_index_input = gui::Edit::new(
            parent,
            gui::EditOpts {
                position: selected_filter_index_input_layout.dpi_pos(),
                width: selected_filter_index_input_layout.dpi_width(),
                height: selected_filter_index_input_layout.dpi_height(),
                control_style: gui::EditOpts::default().control_style | co::ES::NUMBER,
                ..Default::default()
            },
        );
        let selected_filter_index_up_down = gui::UpDown::new(
            parent,
            gui::UpDownOpts {
                position: selected_filter_index_input_layout.dpi_pos(),
                height: selected_filter_index_input_layout.dpi_height(),
                ctrl_id: Self::CTRL_ID_SELECTED_FILTER,
                range: (0, 100_000),
                ..Default::default()
            },
        );

        let mut btn_create_layout = layout.take_top(button_height);
        let btn_delete_layout = btn_create_layout.take_right(95);

        let btn_create = gui::Button::new(
            parent,
            gui::ButtonOpts {
                text: "Create new filter",
                position: btn_create_layout.dpi_pos(),
                height: btn_create_layout.dpi_height(),
                width: btn_create_layout.dpi_width(),
                ..Default::default()
            },
        );
        let btn_delete = gui::Button::new(
            parent,
            gui::ButtonOpts {
                text: "Delete filter",
                position: btn_delete_layout.dpi_pos(),
                height: btn_delete_layout.dpi_height(),
                width: btn_delete_layout.dpi_width(),
                ..Default::default()
            },
        );

        let btn_move_up_layout = layout.take_top(button_height);
        let [btn_move_up_layout, btn_move_down_layout] = btn_move_up_layout.split_horizontal();

        let btn_move_up = gui::Button::new(
            parent,
            gui::ButtonOpts {
                text: "Move up",
                position: btn_move_up_layout.dpi_pos(),
                height: btn_move_up_layout.dpi_height(),
                width: btn_move_up_layout.dpi_width(),
                ..Default::default()
            },
        );
        let btn_move_down = gui::Button::new(
            parent,
            gui::ButtonOpts {
                text: "Move down",
                position: btn_move_down_layout.dpi_pos(),
                height: btn_move_down_layout.dpi_height(),
                width: btn_move_down_layout.dpi_width(),
                ..Default::default()
            },
        );

        // Extra space from previous controls (new grouping)
        layout.take_top_with_margin(layout.margin, 0);

        let window_index_label = gui::Label::new(
            parent,
            gui::LabelOpts {
                text: "Window index:",
                position: layout.dpi_pos(),
                size: (layout.dpi_width(), gui::dpi_y(label_height)),
                ..Default::default()
            },
        );
        layout.take_top(label_height);

        let window_index_range = RangeControl::new(
            parent,
            layout,
            RangeControlOpts {
                lower_up_down_ctrl_id: Self::CTRL_ID_WINDOW_INDEX_LOWER,
                upper_up_down_ctrl_id: Self::CTRL_ID_WINDOW_INDEX_UPPER,
                range_lower: (1, 100_000),
                range_upper: (1, 100_000),
            },
        );

        // Extra space from previous controls (new grouping)
        layout.take_top_with_margin(layout.margin, 0);

        let virtual_desktop_index_label = gui::Label::new(
            parent,
            gui::LabelOpts {
                text: "Virtual desktop index:",
                position: layout.dpi_pos(),
                size: (layout.dpi_width(), gui::dpi_y(label_height)),
                ..Default::default()
            },
        );
        layout.take_top(label_height);

        let virtual_desktop_index_range = RangeControl::new(
            parent,
            layout,
            RangeControlOpts {
                lower_up_down_ctrl_id: Self::CTRL_ID_DESKTOP_INDEX_LOWER,
                upper_up_down_ctrl_id: Self::CTRL_ID_DESKTOP_INDEX_UPPER,
                range_lower: (1, 100_000),
                range_upper: (1, 100_000),
            },
        );

        // Extra space from previous controls (new grouping)
        layout.take_top_with_margin(layout.margin, 0);

        let window_title_label = gui::Label::new(
            parent,
            gui::LabelOpts {
                text: "Window title:",
                position: layout.dpi_pos(),
                size: (layout.dpi_width(), gui::dpi_y(label_height)),
                ..Default::default()
            },
        );
        layout.take_top(label_height);

        let window_title_layout = layout.take_top(input_height * 3 + input_height / 2);
        let window_title_input = gui::Edit::new(
            parent,
            gui::EditOpts {
                position: window_title_layout.dpi_pos(),
                height: window_title_layout.dpi_height(),
                width: window_title_layout.dpi_width(),
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

        // Extra space from previous controls (new grouping)
        layout.take_top_with_margin(layout.margin, 0);

        let process_name_label = gui::Label::new(
            parent,
            gui::LabelOpts {
                text: "Process name:",
                position: layout.dpi_pos(),
                size: (layout.dpi_width(), gui::dpi_y(label_height)),
                ..Default::default()
            },
        );
        layout.take_top(label_height);

        let process_name_layout = layout.take_top(input_height * 3 + input_height / 2);
        let process_name_input = gui::Edit::new(
            parent,
            gui::EditOpts {
                position: process_name_layout.dpi_pos(),
                height: process_name_layout.dpi_height(),
                width: process_name_layout.dpi_width(),
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

        // Extra space from previous controls (new grouping)
        layout.take_top_with_margin(layout.margin, 0);

        let action_label = gui::Label::new(
            parent,
            gui::LabelOpts {
                text: "Virtual desktop action to apply:",
                position: layout.dpi_pos(),
                size: (layout.dpi_width(), gui::dpi_y(label_height)),
                ..Default::default()
            },
        );
        layout.take_top(label_height);

        let action_layout = layout.take_top(button_height);
        let action = gui::ComboBox::new(
            parent,
            gui::ComboBoxOpts {
                position: action_layout.dpi_pos(),
                width: action_layout.dpi_width(),
                items: &FilterAction::all().map(|action| action.as_str()),
                selected_item: Some(0),
                ..Default::default()
            },
        );

        // Extra space from previous controls (new grouping)
        layout.take_top_with_margin(layout.margin, 0);

        let target_desktop_label = gui::Label::new(
            parent,
            gui::LabelOpts {
                text: "Move to virtual desktop at index:",
                position: layout.dpi_pos(),
                size: (layout.dpi_width(), gui::dpi_y(label_height)),
                ..Default::default()
            },
        );
        layout.take_top(label_height);

        let target_desktop_layout = layout.take_top(input_height);
        let target_desktop_input = gui::Edit::new(
            parent,
            gui::EditOpts {
                position: target_desktop_layout.dpi_pos(),
                width: target_desktop_layout.dpi_width(),
                height: target_desktop_layout.dpi_height(),
                control_style: gui::EditOpts::default().control_style | co::ES::NUMBER,
                ..Default::default()
            },
        );
        let target_desktop_up_down = gui::UpDown::new(
            parent,
            gui::UpDownOpts {
                position: target_desktop_layout.dpi_pos(),
                height: target_desktop_layout.dpi_height(),
                ctrl_id: Self::CTRL_ID_TARGET_DESKTOP_INDEX,
                range: (1, 100_000),
                ..Default::default()
            },
        );

        let new_self = Rc::new(Self {
            selected_filter_index_label,
            selected_filter_index_input,
            selected_filter_index_up_down,
            btn_create,
            btn_delete,
            btn_move_up,
            btn_move_down,
            window_index_label,
            window_index_range,
            virtual_desktop_index_label,
            virtual_desktop_index_range,
            window_title_label,
            window_title_input,
            process_name_label,
            process_name_input,
            action_label,
            action,
            target_desktop_label,
            target_desktop_input,
            target_desktop_up_down,
        });
        new_self.events(parent, hooks);
        new_self
    }

    pub fn set_font(&self, msg: &mut SetFont) {
        tracing::trace!("FilterOptionsPanel::set_font");
        let handles = [
            self.selected_filter_index_label.hwnd(),
            self.selected_filter_index_input.hwnd(),
            self.selected_filter_index_up_down.hwnd(),
            self.btn_create.hwnd(),
            self.btn_delete.hwnd(),
            self.btn_move_up.hwnd(),
            self.btn_move_down.hwnd(),
            self.window_index_label.hwnd(),
            self.virtual_desktop_index_label.hwnd(),
            self.window_title_label.hwnd(),
            self.window_title_input.hwnd(),
            self.process_name_label.hwnd(),
            self.process_name_input.hwnd(),
            self.action_label.hwnd(),
            self.action.hwnd(),
            self.target_desktop_label.hwnd(),
            self.target_desktop_input.hwnd(),
            self.target_desktop_up_down.hwnd(),
        ];
        for handle in handles {
            unsafe { handle.SendMessage(msg.as_generic_wm()) };
        }

        self.window_index_range.set_font(msg);
        self.virtual_desktop_index_range.set_font(msg);
    }

    fn post_change(parent: &impl GuiParentWithEvents, change: FilterChange) {
        let result = unsafe {
            parent.hwnd().PostMessage(WndMsg {
                msg_id: custom_msg::WM_FILTER_CHANGED,
                wparam: 0,
                lparam: change as isize,
            })
        };
        if let Err(e) = result {
            tracing::error!(error = ?e, "Failed to post filter change message");
        }
    }
    fn post_change_id(parent: &impl GuiParentWithEvents) {
        let result = unsafe {
            parent.hwnd().PostMessage(WndMsg {
                msg_id: custom_msg::WM_FILTER_ID_CHANGED,
                wparam: 0,
                lparam: 0,
            })
        };
        if let Err(e) = result {
            tracing::error!(error = ?e, "Failed to post filter id changed message");
        }
    }

    fn events<P, H>(self: &Rc<Self>, parent: &P, hooks: H)
    where
        P: GuiParentWithEvents,
        H: FilterOptionsHooks,
    {
        parent.on().wm_create({
            let this = self.clone();
            move |_msg| {
                this.set_enabled_options(false);
                Ok(0)
            }
        });

        let button_handlers: [(&gui::Button, fn(&H, &P)); _] = [
            (&self.btn_create, |hooks, _| hooks.on_create_new()),
            (&self.btn_delete, |hooks, _| hooks.on_delete()),
            (&self.btn_move_up, |hooks, _| hooks.on_move_up()),
            (&self.btn_move_down, |hooks, _| hooks.on_move_down()),
        ];
        for (button, event_handler) in button_handlers {
            button.on().bn_clicked({
                let hooks = hooks.clone();
                let parent = parent.clone();
                move || {
                    event_handler(&hooks, &parent);
                    Ok(())
                }
            });
        }

        let checkbox_handlers: [(&gui::CheckBox, fn(&H, &P)); _] = [
            (&self.window_index_range.lower_checkbox, |_, parent| {
                Self::post_change(parent, FilterChange::WindowRange)
            }),
            (&self.window_index_range.upper_checkbox, |_, parent| {
                Self::post_change(parent, FilterChange::WindowRange)
            }),
            (
                &self.virtual_desktop_index_range.lower_checkbox,
                |_, parent| Self::post_change(parent, FilterChange::DesktopRange),
            ),
            (
                &self.virtual_desktop_index_range.upper_checkbox,
                |_, parent| Self::post_change(parent, FilterChange::DesktopRange),
            ),
        ];
        for (checkbox, event_handler) in checkbox_handlers {
            checkbox.on().bn_clicked({
                let this = self.clone();
                let hooks = hooks.clone();
                let parent = parent.clone();
                move || {
                    event_handler(&hooks, &parent);
                    this.set_enabled_ranges(); // check if some input fields should be enabled/disabled
                    Ok(())
                }
            });
        }

        let input_handlers: [(&gui::Edit, fn(&H, &P)); _] = [
            (&self.selected_filter_index_input, |_, parent| {
                Self::post_change_id(parent)
            }),
            (&self.window_index_range.lower_input, |_, parent| {
                Self::post_change(parent, FilterChange::WindowRange)
            }),
            (&self.window_index_range.upper_input, |_, parent| {
                Self::post_change(parent, FilterChange::WindowRange)
            }),
            (
                &self.virtual_desktop_index_range.lower_input,
                |_, parent| Self::post_change(parent, FilterChange::DesktopRange),
            ),
            (
                &self.virtual_desktop_index_range.upper_input,
                |_, parent| Self::post_change(parent, FilterChange::DesktopRange),
            ),
            (&self.window_title_input, |_, parent| {
                Self::post_change(parent, FilterChange::WindowTitle)
            }),
            (&self.process_name_input, |_, parent| {
                Self::post_change(parent, FilterChange::ProcessName)
            }),
            (&self.target_desktop_input, |_, parent| {
                Self::post_change(parent, FilterChange::TargetDesktop)
            }),
        ];
        for (input, event_handler) in input_handlers {
            input.on().en_change({
                let hooks = hooks.clone();
                let parent = parent.clone();
                move || {
                    event_handler(&hooks, &parent);
                    Ok(())
                }
            });
        }

        self.action.on().cbn_sel_change({
            let parent = parent.clone();
            move || {
                Self::post_change(&parent, FilterChange::Action);
                Ok(())
            }
        });

        // Delayed change events (so that reading values from controls will get the latest values):
        parent.on().wm(custom_msg::WM_FILTER_CHANGED, {
            let hooks = hooks.clone();
            move |msg| {
                let change = FilterChange::try_from(msg.lparam as i64)
                    .expect("lparam should indicate a valid change type");
                hooks.on_option_change(change);
                Ok(0)
            }
        });
        parent.on().wm(custom_msg::WM_FILTER_ID_CHANGED, {
            let hooks = hooks.clone();
            let this = self.clone();
            move |_msg| {
                hooks.on_index_change(); // <- might change the filter index again

                // check afterward if controls should be enabled/disabled:
                this.set_enabled_options(this.get_selected_filter_index().is_some());
                Ok(0)
            }
        });

        // Invert direction of up downs:
        // (Auto fixed if we just set explicit ranges for every UpDown)
        /*
        let up_down_ctrl_ids: [u16; _] = [
            Self::CTRL_ID_SELECTED_FILTER,
            Self::CTRL_ID_WINDOW_INDEX_LOWER,
            Self::CTRL_ID_WINDOW_INDEX_UPPER,
            Self::CTRL_ID_DESKTOP_INDEX_LOWER,
            Self::CTRL_ID_DESKTOP_INDEX_UPPER,
            Self::CTRL_ID_TARGET_DESKTOP_INDEX,
        ];
        for ctrl_id in up_down_ctrl_ids {
            parent
                .on()
                .wm_notify(ctrl_id, co::UDN::DELTAPOS, move |notify| {
                    let nmupdown = unsafe { notify.cast_nmhdr_mut::<NMUPDOWN>() };
                    nmupdown.iDelta = -nmupdown.iDelta;
                    tracing::trace!(
                        ctrl_id,
                        updated_delta = nmupdown.iDelta,
                        prev_pos = nmupdown.iPos,
                        "FilterOptionsPanel.parent.wm_notify(co::UDN::DELTAPOS)"
                    );
                    // Note: we don't need to notify hooks since this UpDown will in turn modify the
                    // underlying Edit control and we listen for edit events on that.
                    Ok(0)
                });
        }
        */
    }
    fn set_enabled_ranges(&self) {
        let ranges = [&self.window_index_range, &self.virtual_desktop_index_range];
        for range in ranges {
            let lower = range.lower_checkbox.is_checked();
            range.lower_input.hwnd().EnableWindow(lower);
            range.lower_up_down.hwnd().EnableWindow(lower);
            let upper = range.upper_checkbox.is_checked();
            range.upper_input.hwnd().EnableWindow(upper);
            range.upper_up_down.hwnd().EnableWindow(upper);
        }
    }
    fn set_enabled_options(&self, enabled: bool) {
        self.btn_delete.hwnd().EnableWindow(enabled);
        self.btn_move_up.hwnd().EnableWindow(enabled);
        self.btn_move_down.hwnd().EnableWindow(enabled);
        self.window_index_range.set_enabled(enabled);
        self.virtual_desktop_index_range.set_enabled(enabled);
        self.window_title_input.hwnd().EnableWindow(enabled);
        self.process_name_input.hwnd().EnableWindow(enabled);
        self.action.hwnd().EnableWindow(enabled);
        self.target_desktop_input.hwnd().EnableWindow(enabled);
        self.set_enabled_ranges();
    }
}
/// Retrieve data related to the selected [`WindowFilter`].
impl FilterOptionsPanel {
    pub fn get_selected_filter_index(&self) -> Option<usize> {
        Some(
            usize::try_from(self.selected_filter_index_up_down.pos())
                .map_err(|e| tracing::error!(error = ?e, "Invalid selected filter index"))
                .unwrap_or_default(),
        )
        .and_then(|index: usize| index.checked_sub(1))
    }
    pub fn get_window_index_range(&self) -> IntegerRange {
        self.window_index_range.get_range().from_one_based_indexes()
    }
    pub fn get_desktop_index_range(&self) -> IntegerRange {
        self.virtual_desktop_index_range
            .get_range()
            .from_one_based_indexes()
    }
    pub fn get_window_title(&self) -> TextPattern {
        self.window_title_input
            .text()
            .map(|v| TextPattern::from(v.as_str()))
            .unwrap_or_else(|e| {
                tracing::error!(error = ?e, "Failed to read text from window title input field");
                Default::default()
            })
    }
    pub fn get_process_name(&self) -> TextPattern {
        self.process_name_input
            .text()
            .map(|v| TextPattern::from(v.as_str()))
            .unwrap_or_else(|e| {
                tracing::error!(error = ?e, "Failed to read text from process name input field");
                Default::default()
            })
    }
    pub fn get_filter_action(&self) -> FilterAction {
        self.action
            .items()
            .selected_index()
            .map(|index| FilterAction::all()[index as usize])
            .unwrap_or_default()
    }
    pub fn get_target_desktop(&self) -> i64 {
        i64::from(self.target_desktop_up_down.pos().saturating_sub(1))
    }
    pub fn get_filter_data(&self) -> WindowFilter {
        WindowFilter {
            window_index: self.get_window_index_range(),
            desktop_index: self.get_desktop_index_range(),
            window_title: self.get_window_title(),
            process_name: self.get_process_name(),
            action: self.get_filter_action(),
            target_desktop: self.get_target_desktop(),
        }
    }
}
/// Set data related to the selected [`WindowFilter`].
impl FilterOptionsPanel {
    pub fn set_filters_len(&self, len: usize) {
        self.selected_filter_index_up_down.set_range(0, len as _);
    }
    pub fn set_selected_filter_index(&self, index: Option<usize>) {
        if self.get_selected_filter_index() == index {
            return;
        }
        self.selected_filter_index_up_down
            .set_pos(index.map(|index| (index + 1) as i32).unwrap_or_default());
        self.set_enabled_options(index.is_some());
    }
    pub fn set_window_index_range(&self, range: IntegerRange) {
        if self.get_window_index_range() == range {
            return;
        }
        self.window_index_range
            .set_range(range.into_one_based_indexes());
        self.set_enabled_ranges();
    }
    pub fn set_desktop_index_range(&self, index: IntegerRange) {
        if self.get_desktop_index_range() == index {
            return;
        }
        self.virtual_desktop_index_range
            .set_range(index.into_one_based_indexes());
        self.set_enabled_ranges();
    }
    pub fn set_window_title(&self, text: &TextPattern) {
        if self.get_window_title() == *text {
            return;
        }
        if let Err(e) = self.window_title_input.set_text(text.pattern()) {
            tracing::error!(error = ?e, "Failed to set window title input field");
        }
    }
    pub fn set_process_name(&self, text: &TextPattern) {
        if self.get_process_name() == *text {
            return;
        }
        if let Err(e) = self.process_name_input.set_text(text.pattern()) {
            tracing::error!(error = ?e, "Failed to set process name input field");
        }
    }
    pub fn set_filter_action(&self, filter_action: FilterAction) {
        if self.get_filter_action() == filter_action {
            return;
        }
        self.action.items().select(
            FilterAction::all()
                .iter()
                .position(|&a| a == filter_action)
                .map(|pos| pos as u32),
        )
    }
    pub fn set_target_desktop(&self, desktop_index: i64) {
        if self.get_target_desktop() == desktop_index {
            return;
        }
        self.target_desktop_up_down
            .set_pos(desktop_index.saturating_add(1) as i32);
    }
    pub fn set_filter_data(&self, filter: &WindowFilter) {
        let WindowFilter {
            window_index,
            desktop_index,
            window_title,
            process_name,
            action,
            target_desktop,
        } = filter;
        self.set_window_index_range(*window_index);
        self.set_desktop_index_range(*desktop_index);
        self.set_window_title(window_title);
        self.set_process_name(process_name);
        self.set_filter_action(*action);
        self.set_target_desktop(*target_desktop);
    }
}

pub struct RangeControlOpts {
    lower_up_down_ctrl_id: u16,
    upper_up_down_ctrl_id: u16,
    range_lower: (i32, i32),
    range_upper: (i32, i32),
}
impl Default for RangeControlOpts {
    fn default() -> Self {
        Self {
            lower_up_down_ctrl_id: 0,
            upper_up_down_ctrl_id: 0,
            range_lower: (0, 100_000),
            range_upper: (0, 100_000),
        }
    }
}

#[derive(Clone)]
pub struct RangeControl {
    pub lower_checkbox: gui::CheckBox,
    pub upper_checkbox: gui::CheckBox,
    pub lower_input: gui::Edit,
    pub lower_up_down: gui::UpDown,
    pub upper_input: gui::Edit,
    pub upper_up_down: gui::UpDown,
}
impl RangeControl {
    pub fn new(
        parent: &(impl GuiParentWithEvents + 'static),
        layout: &mut LayoutArea,
        options: RangeControlOpts,
    ) -> Rc<Self> {
        let input_height = 25;
        let checkbox_height = 20;

        let checkboxes_layout = layout.take_top(checkbox_height);
        let [lower_checkbox_layout, upper_checkbox_layout] = checkboxes_layout.split_horizontal();
        let lower_checkbox = gui::CheckBox::new(
            parent,
            gui::CheckBoxOpts {
                text: "Lower bound",
                position: lower_checkbox_layout.dpi_pos(),
                size: lower_checkbox_layout.dpi_size(),
                ..Default::default()
            },
        );
        let upper_checkbox = gui::CheckBox::new(
            parent,
            gui::CheckBoxOpts {
                text: "Upper bound",
                position: upper_checkbox_layout.dpi_pos(),
                size: upper_checkbox_layout.dpi_size(),
                ..Default::default()
            },
        );

        let input_layout = layout.take_top(input_height);
        let [lower_input_layout, upper_input_layout] = input_layout.split_horizontal();
        let lower_input = gui::Edit::new(
            parent,
            gui::EditOpts {
                position: lower_input_layout.dpi_pos(),
                width: lower_input_layout.dpi_width(),
                height: lower_input_layout.dpi_height(),
                control_style: gui::EditOpts::default().control_style | co::ES::NUMBER,
                ..Default::default()
            },
        );
        let lower_up_down = gui::UpDown::new(
            parent,
            gui::UpDownOpts {
                position: lower_input_layout.dpi_pos(),
                height: lower_input_layout.dpi_height(),
                ctrl_id: options.lower_up_down_ctrl_id,
                range: options.range_lower,
                ..Default::default()
            },
        );
        let upper_input = gui::Edit::new(
            parent,
            gui::EditOpts {
                position: upper_input_layout.dpi_pos(),
                width: upper_input_layout.dpi_width(),
                height: upper_input_layout.dpi_height(),
                control_style: gui::EditOpts::default().control_style | co::ES::NUMBER,
                ..Default::default()
            },
        );
        let upper_up_down = gui::UpDown::new(
            parent,
            gui::UpDownOpts {
                position: upper_input_layout.dpi_pos(),
                height: upper_input_layout.dpi_height(),
                ctrl_id: options.upper_up_down_ctrl_id,
                range: options.range_upper,
                ..Default::default()
            },
        );

        let new_self = Rc::new(Self {
            lower_checkbox,
            upper_checkbox,
            lower_input,
            lower_up_down,
            upper_input,
            upper_up_down,
        });
        new_self.events(parent);
        new_self
    }

    fn events(self: &Rc<Self>, _parent: &impl GuiParentWithEvents) {}

    fn handles(&self) -> [&winsafe::HWND; 6] {
        [
            self.lower_checkbox.hwnd(),
            self.upper_checkbox.hwnd(),
            self.lower_input.hwnd(),
            self.lower_up_down.hwnd(),
            self.upper_input.hwnd(),
            self.upper_up_down.hwnd(),
        ]
    }
    pub fn set_font(&self, msg: &mut SetFont) {
        for handle in self.handles() {
            unsafe { handle.SendMessage(msg.as_generic_wm()) };
        }
    }
    pub fn set_enabled(&self, enabled: bool) {
        for handle in self.handles() {
            handle.EnableWindow(enabled);
        }
    }
}
impl RangeControl {
    pub fn get_range(&self) -> IntegerRange {
        let mut range = IntegerRange::default();
        let fields = [
            (
                &mut range.lower_bound,
                &self.lower_checkbox,
                &self.lower_up_down,
            ),
            (
                &mut range.upper_bound,
                &self.upper_checkbox,
                &self.upper_up_down,
            ),
        ];
        for (out, checkbox, up_down) in fields {
            if checkbox.is_checked() {
                *out = Some(i64::from(up_down.pos()));
                /*
                match text_input.text() {
                    Ok(text) => {
                        match text.parse::<i64>() {
                            Ok(value) => range.lower_bound = Some(value),
                            Err(e) => {
                                tracing::error!(error = ?e, "Failed to parse lower bound text filed as integer (was it too large?)");
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = ?e, "Failed to read text of lower bound text field");
                    }
                }
                */
            }
        }
        range
    }
    pub fn set_range(&self, range: IntegerRange) {
        self.lower_checkbox.set_check(range.lower_bound.is_some());
        self.upper_checkbox.set_check(range.upper_bound.is_some());
        self.lower_up_down
            .set_pos(range.lower_bound.unwrap_or_default() as i32);
        self.upper_up_down
            .set_pos(range.upper_bound.unwrap_or_default() as i32);
    }
}
