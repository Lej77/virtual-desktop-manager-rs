use std::{
    cell::{Cell, OnceCell, RefCell},
    cmp::Ordering,
    collections::BTreeMap,
    fs::OpenOptions,
    io::Write,
    path::PathBuf,
    rc::Rc,
    str::FromStr,
    sync::{
        atomic::{AtomicBool, Ordering as AtomicOrdering},
        mpsc, Arc,
    },
};

use crate::{
    dynamic_gui::DynamicUiHooks,
    nwg_ext::{
        list_view_enable_groups, list_view_item_get_group_id, list_view_item_set_group_id,
        list_view_set_group_info, list_view_sort_rows, window_is_valid, window_placement,
        ListViewGroupAlignment, ListViewGroupInfo, NumberSelect2, WindowPlacement,
    },
    settings::{
        AutoStart, ConfigWindowInfo, QuickSwitchMenu, TrayClickAction, TrayIconType, UiSettings,
    },
    tray::{SystemTray, SystemTrayRef, TrayPlugin},
    vd,
    window_filter::{ExportedWindowFilters, FilterAction, IntegerRange, TextPattern, WindowFilter},
    window_info::WindowInfo,
};

struct BackgroundThread {
    rx: mpsc::Receiver<WindowInfo>,
    handle: Option<std::thread::JoinHandle<()>>,
    should_exit: Arc<AtomicBool>,
}
impl Drop for BackgroundThread {
    fn drop(&mut self) {
        self.should_exit.store(true, AtomicOrdering::Release);
        let Some(handle) = self.handle.take() else {
            return;
        };
        let res = handle.join();
        if !std::thread::panicking() {
            res.unwrap();
        }
    }
}

// Stretch style
use nwg::stretch::{
    geometry::{Rect, Size},
    style::{AlignSelf, Dimension as D, FlexDirection},
};
const fn uniform_rect<D: Copy>(size: D) -> Rect<D> {
    Rect {
        start: size,
        end: size,
        top: size,
        bottom: size,
    }
}
const MARGIN: Rect<D> = uniform_rect(D::Points(5.0));
const TAB_BACKGROUND: Option<[u8; 3]> = Some([255, 255, 255]);

#[derive(nwd::NwgPartial, nwd::NwgUi, Default)]
pub struct ConfigWindow {
    tray: SystemTrayRef,

    sidebar_layout: nwg::FlexboxLayout,
    layout: nwg::FlexboxLayout,

    tooltips: nwg::Tooltip,

    #[nwg_control(
        size: data.create_window_with_size(),
        position: data.create_window_with_position(),
        maximized: data.create_window_with_maximized(),
        title: "Virtual Desktop Manager",
        icon: crate::exe_icon().as_deref(),
    )]
    #[nwg_events(
        OnWindowClose: [Self::on_close],
        OnInit: [Self::on_init],
        OnResizeEnd: [Self::on_resize_end],
        OnMove: [Self::on_move],
        OnMinMaxInfo: [Self::on_window_min_max_info(SELF, EVT_DATA)],
    )]
    pub window: nwg::Window,

    #[nwg_control(
        item_count: 10,
        size: (500, 350),
        list_style: nwg::ListViewStyle::Detailed,
        focus: true,
        ex_flags:
            nwg::ListViewExFlags::GRID |
            nwg::ListViewExFlags::FULL_ROW_SELECT |
            nwg::ListViewExFlags::HEADER_DRAG_DROP,
    )]
    // Note: nwg_layout_item attribute info was written when layout was defined.
    #[nwg_events(
        OnListViewColumnClick: [Self::on_column_click(SELF, EVT_DATA)],
        OnListViewItemActivated: [Self::on_list_view_item_activated(SELF, EVT_DATA)],
    )]
    data_view: nwg::ListView,
    loaded_window_info: RefCell<Vec<WindowInfo>>,
    loaded_filters: RefCell<Option<Arc<[WindowFilter]>>>,

    #[nwg_control(parent: window)]
    sidebar_tab_container: nwg::TabsContainer,

    #[nwg_control(parent: sidebar_tab_container, text: "Filter options")]
    filter_tab: nwg::Tab,

    #[nwg_control(
        parent: filter_tab, position: (5, 5), size: (230, 25),
        text: "Selected filter index:",
        background_color: TAB_BACKGROUND,
    )]
    filter_select_label: nwg::Label,

    #[nwg_control(
        parent: filter_tab, position: (5, 30), size: (225, 25),
        min_int: 0, value_int: 0,
    )]
    #[nwg_events(OnNotice: [Self::on_select_filter_index_changed])]
    filter_select_index: NumberSelect2,
    selected_filter_index: Cell<Option<usize>>,

    #[nwg_control(parent: filter_tab, position: (5, 60), size: (130, 25), text: "Create new filter")]
    #[nwg_events(OnButtonClick: [Self::on_create_filter])]
    filter_create_button: nwg::Button,

    #[nwg_control(parent: filter_tab, position: (140, 60), size: (90, 25), text: "Delete filter")]
    #[nwg_events(OnButtonClick: [Self::on_delete_current_filter])]
    filter_delete_button: nwg::Button,

    #[nwg_control(
        parent: filter_tab, position: (5, 95), size: (230, 25),
        text: "Window index:",
        background_color: TAB_BACKGROUND,
    )]
    filter_window_index_label: nwg::Label,

    #[nwg_control(
        parent: filter_tab, position: (5, 115), size: (110, 25),
        text: "Lower bound",
        background_color: TAB_BACKGROUND,
    )]
    #[nwg_events(OnButtonClick: [Self::on_filter_config_ui_changed])]
    filter_window_index_lower_checkbox: nwg::CheckBox,

    #[nwg_control(
        parent: filter_tab, position: (125, 115), size: (110, 25),
        text: "Upper bound",
        background_color: TAB_BACKGROUND,
    )]
    #[nwg_events(OnButtonClick: [Self::on_filter_config_ui_changed])]
    filter_window_index_upper_checkbox: nwg::CheckBox,

    #[nwg_control(
        parent: filter_tab, position: (5, 140), size: (110, 25),
        min_int: 1, value_int: 1,
    )]
    #[nwg_events(OnNotice: [Self::on_filter_config_ui_changed])]
    filter_window_index_lower: NumberSelect2,

    #[nwg_control(
        parent: filter_tab, position: (125, 140), size: (110, 25),
        min_int: 1, value_int: 1,
    )]
    #[nwg_events(OnNotice: [Self::on_filter_config_ui_changed])]
    filter_window_index_upper: NumberSelect2,

    #[nwg_control(
        parent: filter_tab, position: (5, 175), size: (230, 25),
        text: "Virtual desktop index:",
        background_color: TAB_BACKGROUND,
    )]
    filter_desktop_index_label: nwg::Label,

    #[nwg_control(
        parent: filter_tab, position: (5, 195), size: (110, 25),
        text: "Lower bound",
        background_color: TAB_BACKGROUND,
    )]
    #[nwg_events(OnButtonClick: [Self::on_filter_config_ui_changed])]
    filter_desktop_index_lower_checkbox: nwg::CheckBox,

    #[nwg_control(
        parent: filter_tab, position: (125, 195), size: (110, 25),
        text: "Upper bound",
        background_color: TAB_BACKGROUND,
    )]
    #[nwg_events(OnButtonClick: [Self::on_filter_config_ui_changed])]
    filter_desktop_index_upper_checkbox: nwg::CheckBox,

    #[nwg_control(
        parent: filter_tab, position: (5, 225), size: (110, 25),
        min_int: 1, value_int: 1,
    )]
    #[nwg_events(OnNotice: [Self::on_filter_config_ui_changed])]
    filter_desktop_index_lower: NumberSelect2,

    #[nwg_control(
        parent: filter_tab, position: (125, 225), size: (110, 25),
        min_int: 1, value_int: 1,
    )]
    #[nwg_events(OnNotice: [Self::on_filter_config_ui_changed])]
    filter_desktop_index_upper: NumberSelect2,

    #[nwg_control(
        parent: filter_tab, position: (5, 260), size: (230, 25),
        text: "Window title:",
        background_color: TAB_BACKGROUND,
    )]
    filter_title_label: nwg::Label,

    #[nwg_control(parent: filter_tab, position: (5, 285), size: (230, 85))]
    #[nwg_events(OnTextInput: [Self::on_filter_config_ui_changed])]
    filter_title: nwg::TextBox,

    #[nwg_control(
        parent: filter_tab, position: (5, 375), size: (230, 25),
        text: "Process name:",
        background_color: TAB_BACKGROUND,
    )]
    filter_process_label: nwg::Label,

    #[nwg_control(parent: filter_tab, position: (5, 400), size: (230, 85))]
    #[nwg_events(OnTextInput: [Self::on_filter_config_ui_changed])]
    filter_process: nwg::TextBox,

    #[nwg_control(
        parent: filter_tab, position: (5, 495), size: (230, 25),
        text: "Virtual desktop action to apply:",
        background_color: TAB_BACKGROUND,
    )]
    filter_action_label: nwg::Label,

    #[nwg_control(
        parent: filter_tab, position: (5, 520), size: (230, 25),
        collection: vec![FilterAction::Move, FilterAction::UnpinAndMove, FilterAction::Unpin, FilterAction::Pin, FilterAction::Nothing, FilterAction::Disabled],
        selected_index: Some(5),
    )]
    #[nwg_events(OnComboxBoxSelection: [Self::on_filter_config_ui_changed])]
    filter_action: nwg::ComboBox<FilterAction>,

    #[nwg_control(
        parent: filter_tab, position: (5, 555), size: (230, 25),
        text: "Move to virtual desktop at index:",
        background_color: TAB_BACKGROUND,
    )]
    filter_target_desktop_label: nwg::Label,

    #[nwg_control(
        parent: filter_tab, position: (5, 580), size: (225, 25),
        min_int: 1, value_int: 1,
    )]
    #[nwg_events(OnNotice: [Self::on_filter_config_ui_changed])]
    filter_target_desktop: NumberSelect2,

    #[nwg_control(parent: sidebar_tab_container, text: "Program settings")]
    settings_tab: nwg::Tab,

    #[nwg_control(
        parent: settings_tab, position: (5, 5), size: (240, 25),
        text: "Start program with admin rights",
        background_color: TAB_BACKGROUND,
    )]
    #[nwg_events(OnButtonClick: [Self::on_settings_ui_changed])]
    settings_start_as_admin: nwg::CheckBox,

    #[nwg_control(
        parent: settings_tab, position: (5, 35), size: (240, 25),
        text: "Auto start with Windows:",
        background_color: TAB_BACKGROUND,
    )]
    settings_auto_start_label: nwg::Label,

    #[nwg_control(
        parent: settings_tab, position: (5, 60), size: (240, 25),
        collection: AutoStart::ALL.to_vec(),
        selected_index: Some(0),
    )]
    #[nwg_events(OnComboxBoxSelection: [Self::on_settings_ui_changed])]
    settings_auto_start: nwg::ComboBox<AutoStart>,

    #[nwg_control(
        parent: settings_tab, position: (5, 95), size: (240, 25),
        text: "Prevent flashing windows",
        background_color: TAB_BACKGROUND,
    )]
    #[nwg_events(OnButtonClick: [Self::on_settings_ui_changed])]
    settings_prevent_flashing_windows: nwg::CheckBox,

    #[nwg_control(
        parent: settings_tab, position: (5, 125), size: (240, 25),
        text: "Smoothly switch virtual desktop",
        background_color: TAB_BACKGROUND,
    )]
    #[nwg_events(OnButtonClick: [Self::on_settings_ui_changed])]
    settings_smooth_switch_desktop: nwg::CheckBox,

    #[nwg_control(
        parent: settings_tab, position: (5, 155), size: (240, 25),
        text: "Tray icon:",
        background_color: TAB_BACKGROUND,
    )]
    settings_tray_icon_label: nwg::Label,

    #[nwg_control(
        parent: settings_tab, position: (5, 180), size: (240, 25),
        collection: TrayIconType::ALL.to_vec(),
        selected_index: Some(0),
    )]
    #[nwg_events(OnComboxBoxSelection: [Self::on_settings_ui_changed])]
    settings_tray_icon: nwg::ComboBox<TrayIconType>,

    #[nwg_control(
        parent: settings_tab, position: (5, 215), size: (240, 25),
        text: "Quick switch context menu:",
        background_color: TAB_BACKGROUND,
    )]
    settings_quick_menu_label: nwg::Label,

    #[nwg_control(
        parent: settings_tab, position: (5, 240), size: (240, 25),
        collection: QuickSwitchMenu::ALL.to_vec(),
        selected_index: Some(0),
    )]
    #[nwg_events(OnComboxBoxSelection: [Self::on_settings_ui_changed])]
    settings_quick_menu: nwg::ComboBox<QuickSwitchMenu>,

    #[nwg_control(
        parent: settings_tab, position: (5, 275), size: (240, 25),
        text: "Quick switch menu shortcuts:",
        background_color: TAB_BACKGROUND,
    )]
    settings_quick_menu_shortcuts_label: nwg::Label,

    #[nwg_control(parent: settings_tab, position: (5, 300), size: (240, 85))]
    #[nwg_events(OnTextInput: [Self::on_settings_ui_changed])]
    settings_quick_menu_shortcuts: nwg::TextBox,

    #[nwg_control(
        parent: settings_tab, position: (5, 395), size: (240, 25),
        text: "Quick shortcuts in submenus",
        background_color: TAB_BACKGROUND,
    )]
    #[nwg_events(OnButtonClick: [Self::on_settings_ui_changed])]
    settings_quick_menu_shortcuts_in_submenus: nwg::CheckBox,

    #[nwg_control(
        parent: settings_tab, position: (5, 430), size: (240, 25),
        text: "Global hotkey for quick switch:",
        background_color: TAB_BACKGROUND,
    )]
    settings_quick_menu_hotkey_label: nwg::Label,

    #[nwg_control(parent: settings_tab, position: (5, 455), size: (240, 28))]
    #[nwg_events(OnTextInput: [Self::on_settings_ui_changed])]
    settings_quick_menu_hotkey: nwg::TextInput,

    #[nwg_control(parent: settings_tab,
        position: (5, 490), size: (240, 46),
        readonly: true,
        flags: "HSCROLL | AUTOHSCROLL | TAB_STOP | VISIBLE",
    )]
    settings_quick_menu_hotkey_error: nwg::TextBox,

    #[nwg_control(
        parent: settings_tab, position: (5, 550), size: (240, 25),
        text: "Left click on tray icon:",
        background_color: TAB_BACKGROUND,
    )]
    settings_left_click_label: nwg::Label,

    #[nwg_control(
        parent: settings_tab, position: (5, 575), size: (240, 25),
        collection: TrayClickAction::ALL.to_vec(),
        selected_index: Some(0),
    )]
    #[nwg_events(OnComboxBoxSelection: [Self::on_settings_ui_changed])]
    settings_left_click: nwg::ComboBox<TrayClickAction>,

    #[nwg_control(
        parent: settings_tab, position: (5, 610), size: (240, 25),
        text: "Middle click on tray icon:",
        background_color: TAB_BACKGROUND,
    )]
    settings_middle_click_label: nwg::Label,

    #[nwg_control(
        parent: settings_tab, position: (5, 635), size: (240, 25),
        collection: TrayClickAction::ALL.to_vec(),
        selected_index: Some(0),
    )]
    #[nwg_events(OnComboxBoxSelection: [Self::on_settings_ui_changed])]
    settings_middle_click: nwg::ComboBox<TrayClickAction>,

    #[nwg_control(
        parent: settings_tab, position: (5, 680), size: (240, 40),
        text: "Global hotkey to open context\r\nmenu at current mouse position:",
        background_color: TAB_BACKGROUND,
    )]
    settings_open_menu_at_mouse_pos_hotkey_label: nwg::Label,

    #[nwg_control(parent: settings_tab, position: (5, 680 + 50), size: (240, 28))]
    #[nwg_events(OnTextInput: [Self::on_settings_ui_changed])]
    settings_open_menu_at_mouse_pos_hotkey: nwg::TextInput,

    #[nwg_control(parent: settings_tab,
        position: (5, 680 + 50 + 35), size: (240, 46),
        readonly: true,
        flags: "HSCROLL | AUTOHSCROLL | TAB_STOP | VISIBLE",
    )]
    settings_open_menu_at_mouse_pos_hotkey_error: nwg::TextBox,

    #[nwg_control(parent: window, flags: "VISIBLE")]
    utils_frame: nwg::Frame,

    #[nwg_control(parent: utils_frame, position: (0, 5), size: (125, 30), text: "Import filters")]
    #[nwg_events(OnButtonClick: [Self::on_import_filters])]
    utils_import: nwg::Button,

    #[nwg_control(parent: utils_frame, position: (130, 5), size: (130, 30), text: "Export filters")]
    #[nwg_events(OnButtonClick: [Self::on_export_filters])]
    utils_export: nwg::Button,

    #[nwg_control(parent: utils_frame, position: (0, 45), size: (100, 55), text: "Refresh info")]
    #[nwg_events(OnButtonClick: [Self::on_refresh_info])]
    utils_refresh: nwg::Button,

    #[nwg_control(parent: utils_frame, position: (105, 45), size: (155, 55), text: "Apply filters")]
    #[nwg_events(OnButtonClick: [Self::on_apply_filters])]
    utils_apply_filters: nwg::Button,

    background_thread: RefCell<Option<BackgroundThread>>,
    has_queued_refresh: Cell<bool>,
    is_data_sorted: Cell<bool>,

    #[nwg_control(parent: window)]
    #[nwg_events(OnNotice: [Self::on_data])]
    data_notice: nwg::Notice,

    is_closed: Cell<bool>,
    pub open_soon: Cell<bool>,

    export_dialog: OnceCell<nwg::FileDialog>,
    import_dialog: OnceCell<nwg::FileDialog>,
}
/// Setup code
impl ConfigWindow {
    const GROUP_WINDOWS: i32 = 1;
    const GROUP_FILTERS: i32 = 2;

    const COLUMN_WINDOWS_INDEX: usize = 0;
    const COLUMN_FILTERS_INDEX: usize = 4;
    const COLUMN_TARGET_DESKTOP: usize = 5;

    fn create_window_with_size(&self) -> (i32, i32) {
        let (x, y) = self
            .tray
            .get()
            .map(|tray| tray.settings().get().config_window)
            .unwrap_or_default()
            .size;
        let (min_x, min_y) = Self::MIN_SIZE;
        ((x as i32).max(min_x), (y as i32).max(min_y))
    }
    fn create_window_with_position(&self) -> (i32, i32) {
        self.tray
            .get()
            .and_then(|tray| tray.settings().get().config_window.position)
            .unwrap_or((300, 300))
    }
    fn create_window_with_maximized(&self) -> bool {
        self.tray
            .get()
            .map(|tray| tray.settings().get().config_window)
            .unwrap_or_default()
            .maximized
    }

    fn build_layout(&self) -> Result<(), nwg::NwgError> {
        let ui = self;

        // layout for the sidebar on the right side of the window:
        let mut sidebar_layout = nwg::FlexboxLayout::builder()
            .parent(&ui.window)
            .flex_direction(FlexDirection::Column);
        // First we have the "configuration" area with different tabs:
        sidebar_layout = sidebar_layout
            .child(&ui.sidebar_tab_container)
            .child_margin(MARGIN)
            .child_align_self(AlignSelf::Stretch)
            .child_flex_grow(1.0)
            .child_size(Size {
                width: D::Points(260.0),
                height: D::Auto,
            });
        // Then we have an area for buttons affecting the data table to the left:
        sidebar_layout = sidebar_layout
            .child(&ui.utils_frame)
            .child_margin(MARGIN)
            .child_align_self(AlignSelf::Stretch)
            .child_size(Size {
                width: D::Points(260.0),
                height: D::Points(100.0),
            });
        // Note: use build_partial here since it is a child layout
        sidebar_layout.build_partial(&ui.sidebar_layout)?;

        // Top-most layout of window (uses build, not build_partial):
        let mut main_layout = nwg::FlexboxLayout::builder()
            .parent(&ui.window)
            .flex_direction(FlexDirection::Row)
            .padding(uniform_rect(D::Points(5.0)));
        // The table with windows and filters comes first and fills most of the space:
        main_layout = main_layout
            .child(&ui.data_view)
            .child_margin(MARGIN)
            .child_flex_grow(1.0)
            .child_size(Size {
                width: D::Auto,
                height: D::Auto,
            });
        // Then we register the sidebar sub-layout that should be 250px wide:
        main_layout = main_layout
            .child_layout(&ui.sidebar_layout)
            .child_size(Size {
                width: D::Points(270.0),
                height: D::Auto,
            })
            .child_align_self(AlignSelf::Stretch);
        main_layout.build(&ui.layout)?;
        Ok(())
    }
    fn build_tooltip(&mut self) -> Result<(), nwg::NwgError> {
        nwg::Tooltip::builder()
            .register(
                self.settings_start_as_admin.handle,
                "This is useful in order to move windows owned by other \
                programs that have admin rights.",
            )
            .register(
                self.settings_prevent_flashing_windows.handle,
                "Some windows can try to grab attention by flashing their \
                icon in the taskbar, this option suppresses such flashing right \
                after window filters are applied.",
            )
            .register(
                self.settings_smooth_switch_desktop.handle,
                "Enable for this program to use animations when changing \
                the current virtual desktop.",
            )
            .register(
                self.utils_import.handle,
                "Add new filters by loading them from a selected file.",
            )
            .register(self.utils_export.handle, "Save all filters to a file")
            .register(
                self.utils_refresh.handle,
                "Reload info about all open windows",
            )
            .register(
                self.utils_apply_filters.handle,
                "Use the configured filters to move windows to specific virtual desktops",
            )
            .register(
                &self.settings_quick_menu_shortcuts_label,
                "Each line should have a letter or symbol followed by a zero-based \
                virtual desktop index. For each line an extra context menu item will \
                be created in the quick switch menu with that symbol as its access key.",
            )
            .register(
                &self.settings_quick_menu_shortcuts_in_submenus,
                "If checked then extra context menu items for quick switch shortcuts \
                will be created in each submenu of the quick switch menu when there are \
                more than 9 virtual desktops.",
            )
            .register(
                &self.settings_middle_click_label,
                "Controls the action that will be preformed when the tray icon \
                is middle clicked. On some Windows 11 versions middle clicks are \
                registered as left clicks.",
            )
            .build(&mut self.tooltips)?;
        Ok(())
    }

    fn on_init(&self) {
        let dv = &self.data_view;

        dv.set_headers_enabled(true);

        debug_assert_eq!(Self::COLUMN_WINDOWS_INDEX, dv.column_len());
        dv.insert_column(nwg::InsertListViewColumn {
            index: Some(dv.column_len() as _),
            fmt: Some(nwg::ListViewColumnFlags::LEFT),
            width: Some(100),
            text: Some("Window Index".into()),
        });

        dv.insert_column(nwg::InsertListViewColumn {
            index: Some(dv.column_len() as _),
            fmt: Some(nwg::ListViewColumnFlags::LEFT),
            width: Some(100),
            text: Some("Virtual Desktop".into()),
        });

        dv.insert_column(nwg::InsertListViewColumn {
            index: Some(dv.column_len() as _),
            fmt: Some(nwg::ListViewColumnFlags::LEFT),
            width: Some(200),
            text: Some("Window Title".into()),
        });

        dv.insert_column(nwg::InsertListViewColumn {
            index: Some(dv.column_len() as _),
            fmt: Some(nwg::ListViewColumnFlags::LEFT),
            width: Some(200),
            text: Some("Process Name".into()),
        });

        debug_assert_eq!(Self::COLUMN_FILTERS_INDEX, dv.column_len());
        dv.insert_column(nwg::InsertListViewColumn {
            index: Some(dv.column_len() as _),
            fmt: Some(nwg::ListViewColumnFlags::LEFT),
            width: Some(100),
            text: Some("Filter Index".into()),
        });

        debug_assert_eq!(Self::COLUMN_TARGET_DESKTOP, dv.column_len());
        dv.insert_column(nwg::InsertListViewColumn {
            index: Some(dv.column_len() as _),
            fmt: Some(nwg::ListViewColumnFlags::LEFT),
            width: Some(100),
            text: Some("Target Desktop".into()),
        });

        dv.set_column_sort_arrow(0, None);

        list_view_enable_groups(dv, true);
        list_view_set_group_info(
            dv,
            ListViewGroupInfo {
                create_new: true,
                group_id: Self::GROUP_WINDOWS,
                header: Some("Active Windows".into()),
                header_alignment: Some(ListViewGroupAlignment::Left),
                ..Default::default()
            },
        );
        list_view_set_group_info(
            dv,
            ListViewGroupInfo {
                create_new: true,
                group_id: Self::GROUP_FILTERS,
                header: Some("Filters / Rules".into()),
                header_alignment: Some(ListViewGroupAlignment::Left),
                ..Default::default()
            },
        );

        self.sync_filter_from_settings(None);
        self.set_selected_filter_index(Some(0));
        self.gather_window_info();
    }
}
/// Sort list view.
impl ConfigWindow {
    fn on_column_click(&self, data: &nwg::EventData) {
        let &nwg::EventData::OnListViewItemIndex { column_index, .. } = data else {
            tracing::error!(event_data = ?data, "ConfigWindow::on_column_click: got unexpected event data");
            return;
        };
        tracing::trace!(event_data = ?data, "ConfigWindow::on_column_click");

        let sort_dir = self.data_view.column_sort_arrow(column_index);
        let new_sort_dir = match sort_dir {
            Some(nwg::ListViewColumnSortArrow::Up)
                if column_index == Self::COLUMN_WINDOWS_INDEX =>
            {
                None
            }
            Some(nwg::ListViewColumnSortArrow::Up) => Some(nwg::ListViewColumnSortArrow::Down),
            Some(nwg::ListViewColumnSortArrow::Down) => Some(nwg::ListViewColumnSortArrow::Up),
            None => Some(nwg::ListViewColumnSortArrow::Down),
        };
        tracing::debug!(column_index, ?sort_dir, ?new_sort_dir, "on_column_click");
        self.data_view
            .set_column_sort_arrow(column_index, new_sort_dir);
        for i in 0..self.data_view.column_len() {
            if i == column_index {
                continue;
            }
            self.data_view.set_column_sort_arrow(i, None);
        }
        self.sort_items(
            Some(column_index).filter(|_| new_sort_dir.is_some()),
            new_sort_dir,
        );
    }
    fn get_sort_info(&self) -> (Option<usize>, Option<nwg::ListViewColumnSortArrow>) {
        for i in 0..self.data_view.column_len() {
            let sort_dir = self.data_view.column_sort_arrow(i);
            if sort_dir.is_some() {
                return (Some(i), sort_dir);
            }
        }
        (None, None)
    }
    fn resort_items(&self) {
        let (index, sort_dir) = self.get_sort_info();
        self.sort_items(index, sort_dir);
    }
    fn sort_items(
        &self,
        column_index: Option<usize>,
        sort_dir: Option<nwg::ListViewColumnSortArrow>,
    ) {
        list_view_sort_rows(&self.data_view, |a_ix, b_ix| {
            let a_group = list_view_item_get_group_id(&self.data_view, a_ix);
            let b_group = list_view_item_get_group_id(&self.data_view, b_ix);
            let group_cmp = a_group.cmp(&b_group);
            if group_cmp.is_ne() {
                // Sort by group first:
                return group_cmp;
            }

            let mut using_fallback = column_index.is_none();
            let result = loop {
                let current_column_index = if using_fallback {
                    if a_group == Self::GROUP_FILTERS {
                        Self::COLUMN_FILTERS_INDEX
                    } else if a_group == Self::GROUP_WINDOWS {
                        Self::COLUMN_WINDOWS_INDEX
                    } else {
                        tracing::warn!("Tried to sort row that was neither a window or a filter");
                        column_index.unwrap_or_default()
                    }
                } else {
                    column_index.unwrap_or_default()
                };
                let a = self.data_view.item(a_ix, current_column_index, 4096);
                let b = self.data_view.item(b_ix, current_column_index, 4096);
                let (a, b) = match (a, b) {
                    (Some(a), Some(b)) => (a, b),
                    (None, Some(_)) => {
                        tracing::warn!("Failed to get list item at row {}", a_ix);
                        // First item likely had too long text (so put it last):
                        return Ordering::Greater;
                    }
                    (Some(_), None) => {
                        tracing::warn!("Failed to get list item at row {}", b_ix);
                        return Ordering::Less;
                    }
                    (None, None) => {
                        tracing::warn!("Failed to get list item at row {} and row {}", a_ix, b_ix);
                        return Ordering::Equal;
                    }
                };

                let result = match a
                    .text
                    .parse::<i64>()
                    .and_then(|a| Ok((a, b.text.parse::<i64>()?)))
                {
                    Ok((a, b)) => a.cmp(&b),
                    Err(_) => a.text.cmp(&b.text),
                };
                if result.is_ne() || using_fallback {
                    break result;
                } else {
                    using_fallback = true;
                }
            };
            if using_fallback {
                result
            } else if let Some(nwg::ListViewColumnSortArrow::Up) = sort_dir {
                result.reverse()
            } else {
                result
            }
        });
        self.is_data_sorted.set(true);
    }
}
/// Manage window info inside list view.
impl ConfigWindow {
    fn clear_window_info(&self) {
        for ix in (0..self.data_view.len()).rev() {
            let group = list_view_item_get_group_id(&self.data_view, ix);
            if group == Self::GROUP_WINDOWS {
                self.data_view.remove_item(ix);
            }
        }
        self.loaded_window_info.replace(Vec::new());
    }
    fn determine_active_filter_indexes_for_window(
        &self,
        window_index: i32,
        window: &WindowInfo,
    ) -> String {
        self.loaded_filters
            .borrow()
            .as_deref()
            .unwrap_or_default()
            .iter()
            .enumerate()
            // Find filters/rules that apply to this window:
            .filter(|(_, rule)| rule.check_window(window_index, window))
            // one-based indexes:
            .map(|(ix, _)| (ix + 1).to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }
    fn add_window_info(&self, window: WindowInfo) {
        let index = {
            let mut guard = self.loaded_window_info.borrow_mut();
            let index = guard.len();
            guard.push(window.clone());
            index
        };

        let filter_indexes = self.determine_active_filter_indexes_for_window(index as i32, &window);
        let action = WindowFilter::find_first_action(
            self.loaded_filters.borrow().as_deref().unwrap_or_default(),
            index as i32,
            &window,
        )
        .map(|filter| filter.display_target_desktop().to_string());

        let WindowInfo {
            handle: _,
            title,
            process_id: _,
            process_name,
            virtual_desktop,
        } = window;

        let virtual_desktop = format!("{virtual_desktop}");
        let one_based_index = (index + 1).to_string();
        let info = [
            one_based_index.as_str(),
            virtual_desktop.as_str(),
            title.as_str(),
            &*process_name,
            filter_indexes.as_str(),
            action.as_deref().unwrap_or_default(),
        ];
        self.data_view.insert_items_row(None, &info);
        list_view_item_set_group_id(
            &self.data_view,
            self.data_view.len().saturating_sub(1),
            Some(Self::GROUP_WINDOWS),
        );
        self.is_data_sorted.set(false);
    }
    fn update_window_infos(&self) {
        for row_ix in (0..self.data_view.len()).rev() {
            let group = list_view_item_get_group_id(&self.data_view, row_ix);
            if group != Self::GROUP_WINDOWS {
                continue;
            }
            let Some(window_index_item) =
                self.data_view.item(row_ix, Self::COLUMN_WINDOWS_INDEX, 10)
            else {
                continue;
            };

            let Ok(window_index) = window_index_item.text.parse::<usize>() else {
                continue;
            };
            // UI has one-based index:
            let window_index = window_index - 1;
            let Some(window_info) = self.loaded_window_info.borrow().get(window_index).cloned()
            else {
                continue;
            };

            let filter_indexes =
                self.determine_active_filter_indexes_for_window(window_index as i32, &window_info);
            self.data_view.update_item(
                row_ix,
                nwg::InsertListViewItem {
                    index: Some(row_ix as _),
                    column_index: Self::COLUMN_FILTERS_INDEX as _,
                    text: Some(filter_indexes),
                    image: None,
                },
            );

            let action = WindowFilter::find_first_action(
                self.loaded_filters.borrow().as_deref().unwrap_or_default(),
                window_index as i32,
                &window_info,
            )
            .map(|filter| filter.display_target_desktop().to_string());
            self.data_view.update_item(
                row_ix,
                nwg::InsertListViewItem {
                    index: Some(row_ix as _),
                    column_index: Self::COLUMN_TARGET_DESKTOP as _,
                    text: Some(action.unwrap_or_default()),
                    image: None,
                },
            );
        }
    }

    fn gather_window_info(&self) {
        let mut guard = self.background_thread.borrow_mut();
        if matches!(
            &*guard,
            Some(BackgroundThread { handle: Some(handle), should_exit, .. })
            if !handle.is_finished() && !should_exit.load(AtomicOrdering::Acquire)
        ) {
            self.has_queued_refresh.set(true);
            return; // Wait for previous operation
        }
        self.clear_window_info();
        self.has_queued_refresh.set(false);

        let (tx, rx) = mpsc::channel();
        let notice_tx = self.data_notice.sender();
        let should_exit = <Arc<AtomicBool>>::default();
        let handle = std::thread::Builder::new()
                .name("ConfigWindowBackgroundThread".to_owned())
                .spawn({
                    let should_exit = Arc::clone(&should_exit);
                    move || {
                        if vd::has_loaded_dynamic_library_successfully() {
                            // Old .dll files might not call `CoInitialize` and then not work,
                            // so to be safe we make sure to do that:
                            if let Err(e) = unsafe { windows::Win32::System::Com::CoInitialize(None) }.ok() {
                                tracing::warn!(
                                    error = e.to_string(),
                                    "Failed to call CoInitialize on ConfigWindowBackgroundThread"
                                );
                            }
                        }
                        for result in WindowInfo::try_get_all() {
                            if let Ok(window) = result {
                                tracing::trace!(info = ?window, "Sending window info to config window");
                                if tx.send(window).is_err() {
                                    tracing::debug!("Canceled config window background thread since receiver was closed");
                                    return;
                                }
                                notice_tx.notice();
                            }
                            if should_exit.load(AtomicOrdering::Relaxed) {
                                tracing::debug!(
                                    "Canceled config window background thread since it was requested"
                                );
                                return;
                            }
                        }
                        should_exit.store(true, AtomicOrdering::Relaxed);
                        // Drop tx and send notice so that ui knows we are done:
                        drop(tx);
                        notice_tx.notice();
                        tracing::debug!(
                            "Config window background thread has gathered info about all windows"
                        );
                    }
                }).expect("Failed to spawn config window thread");

        // If there already was a background thread here then it will be stopped:
        *guard = Some(BackgroundThread {
            rx,
            handle: Some(handle),
            should_exit,
        });
    }
    fn on_data(&self) {
        let Ok(guard) = self.background_thread.try_borrow() else {
            tracing::warn!("Received notice from background thread while RefCell was locked, might delay a table update");
            return;
        };
        let Some(background) = &*guard else {
            tracing::warn!(
                "Received notice from background thread, but no such thread was running"
            );
            return;
        };
        tracing::trace!("ConfigWindow::on_data");
        loop {
            match background.rx.try_recv() {
                Ok(window) => {
                    tracing::trace!(info = ?window, "Received window info from background thread");
                    self.add_window_info(window);
                    continue;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    // Got all data!
                    drop(guard);
                    self.on_gathered_all_window_info();
                }
                Err(mpsc::TryRecvError::Empty) => {
                    // Will get more data later
                }
            }
            break;
        }
    }
    fn on_gathered_all_window_info(&self) {
        if !self.is_data_sorted.get() {
            self.resort_items();
        }
        if self.has_queued_refresh.get() {
            self.gather_window_info();
        }
    }
}
/// Window events and helper methods.
impl ConfigWindow {
    const MIN_SIZE: (i32, i32) = (300, 1025);

    pub fn is_closed(&self) -> bool {
        self.is_closed.get() || !window_is_valid(self.window.handle)
    }
    pub fn set_as_foreground_window(&self) {
        let Some(handle) = self.window.handle.hwnd() else {
            return;
        };
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow(
                windows::Win32::Foundation::HWND(handle.cast()),
            );
        }
    }

    fn save_position_and_size(&self) {
        let Some(tray) = self.tray.get() else {
            return;
        };
        let pos = self.window.position();
        let size = self.window.size();
        let placement = window_placement(&self.window).unwrap_or(WindowPlacement::Minimized);
        let maximized = placement == WindowPlacement::Maximized;

        tracing::trace!(
            position =? pos,
            size =? size,
            ?placement,
            "Config window resized or moved"
        );
        if placement == WindowPlacement::Minimized {
            return;
        }

        tray.settings().update(|prev| UiSettings {
            config_window: if maximized {
                // Don't save size and position of maximized window:
                ConfigWindowInfo {
                    maximized,
                    ..prev.config_window
                }
            } else {
                ConfigWindowInfo {
                    position: Some(pos),
                    size,
                    maximized,
                }
            },
            ..prev.clone()
        });
    }
    fn on_resize_end(&self) {
        self.save_position_and_size();
    }
    fn on_move(&self) {
        self.save_position_and_size();
    }
    fn on_close(&self) {
        self.is_closed.set(true);
        if let Some(background) = &*self.background_thread.borrow() {
            background.should_exit.store(true, AtomicOrdering::Release);
        }
    }
    fn on_window_min_max_info(&self, data: &nwg::EventData) {
        let nwg::EventData::OnMinMaxInfo(info) = data else {
            return;
        };
        let (width, height) = Self::MIN_SIZE;
        info.set_min_size(width, height);
    }
}
/// Handle Sidebar Events
impl ConfigWindow {
    fn on_apply_filters(&self) {
        let Some(tray) = self.tray.get() else {
            return;
        };
        tray.apply_filters();
    }
    fn on_refresh_info(&self) {
        self.gather_window_info();
    }
    fn on_export_filters(&self) {
        let dialog = if let Some(dialog) = self.export_dialog.get() {
            dialog
        } else {
            let mut dialog = nwg::FileDialog::default();
            if let Err(e) = nwg::FileDialog::builder()
                .title("Export Virtual Desktop Manager Rules / Filters")
                .action(nwg::FileDialogAction::Save)
                .filters("JSON filters(*.json)|Xml legacy filters(*.xml;*.txt)|Any filter file(*.json;*.xml;*.txt)|All files(*)")
                .build(&mut dialog)
            {
                tracing::error!(error = e.to_string(), "Failed to create export dialog");
                return;
            }
            self.export_dialog.get_or_init(|| dialog)
        };
        if !dialog.run(Some(self.window.handle)) {
            return;
        }
        let Ok(mut selected) = dialog
            .get_selected_item()
            .map(PathBuf::from)
            .inspect_err(|e| {
                tracing::error!(
                    error = e.to_string(),
                    "Failed to get selected item from export dialog"
                );
            })
        else {
            return;
        };

        // The file dialog should have asked about overwriting existing file.
        let mut allow_overwrite = true;

        let is_legacy = if let Some(ext) = selected.extension() {
            ext.eq_ignore_ascii_case("xml") || ext.eq_ignore_ascii_case("txt")
        } else {
            selected.set_extension("json");
            allow_overwrite = false; // <- Since we change the path the dialog would not have warned about overwrite
            false
        };
        let Some(data) = (if is_legacy {
            #[cfg(feature = "persist_filters_xml")]
            {
                let filters = self.loaded_filters.borrow().clone();
                WindowFilter::serialize_to_xml(filters.as_deref().unwrap_or_default())
                    .inspect_err(|e| {
                        nwg::error_message(
                            "Virtual Desktop Manager - Export error",
                            &format!("Failed to convert filters to legacy XML format:\n{e}"),
                        );
                    })
                    .ok()
            }
            #[cfg(not(feature = "persist_filters_xml"))]
            {
                nwg::error_message(
                    "Virtual Desktop Manager - Export error",
                    "This program was compiled without support for legacy XML filters/rules. \
                    Recompile the program from source with the \"persist_filters_xml\" feature \
                    in order to support exporting such filter files.",
                );
                None
            }
        } else {
            #[cfg(feature = "persist_filters")]
            {
                let exported = ExportedWindowFilters {
                    filters: self
                        .loaded_filters
                        .borrow()
                        .clone()
                        .unwrap_or_default()
                        .to_vec(),
                    ..Default::default()
                };
                serde_json::to_string_pretty(&exported)
                    .inspect_err(|e| {
                        nwg::error_message(
                            "Virtual Desktop Manager - Export error",
                            &format!("Failed to convert filters to JSON:\n{e}"),
                        );
                    })
                    .ok()
            }
            #[cfg(not(feature = "persist_filters"))]
            {
                nwg::error_message(
                    "Virtual Desktop Manager - Export error",
                    "This program was compiled without support for JSON filters/rules. \
                    Recompile the program from source with the \"persist_filters\" feature \
                    in order to support exporting such filter files.",
                );
                None
            }
        }) else {
            return;
        };
        let Ok(mut file) = OpenOptions::new()
            .create(true)
            .create_new(!allow_overwrite)
            .write(true)
            .truncate(true)
            .open(selected.as_path())
            .inspect_err(|e| {
                nwg::error_message(
                    "Virtual Desktop Manager - Export error",
                    &format!("Failed to create file at \"{}\":\n{e}", selected.display()),
                );
            })
        else {
            return;
        };
        if let Err(e) = file.write_all(data.as_bytes()) {
            nwg::error_message(
                "Virtual Desktop Manager - Export error",
                &format!(
                    "Failed to write data to file at \"{}\":\n{e}",
                    selected.display()
                ),
            );
        }
    }
    fn on_import_filters(&self) {
        let dialog = if let Some(dialog) = self.import_dialog.get() {
            dialog
        } else {
            let mut dialog = nwg::FileDialog::default();
            if let Err(e) = nwg::FileDialog::builder()
                .title("Import Virtual Desktop Manager Rules / Filters")
                .action(nwg::FileDialogAction::Open)
                .filters("Any filter file(*.json;*.xml;*.txt)|JSON filters(*.json)|Xml legacy filters(*.xml;*.txt)|All files(*)")
                .build(&mut dialog)
            {
                tracing::error!(error = e.to_string(), "Failed to create import dialog");
                return;
            }
            self.import_dialog.get_or_init(|| dialog)
        };
        if !dialog.run(Some(self.window.handle)) {
            return;
        }
        let Ok(selected) = dialog
            .get_selected_item()
            .map(PathBuf::from)
            .inspect_err(|e| {
                tracing::error!(
                    error = e.to_string(),
                    "Failed to get selected item from import dialog"
                );
            })
        else {
            return;
        };
        let data = match std::fs::read_to_string(selected.as_path()) {
            Ok(v) => v,
            Err(e) => {
                nwg::error_message(
                    "Virtual Desktop Manager - Import error",
                    &format!(
                        "Error when reading file with filter/rule at \"{}\":\n\n{e}",
                        selected.display()
                    ),
                );
                return;
            }
        };
        let is_legacy = selected
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("xml") || ext.eq_ignore_ascii_case("txt"));

        let Some(imported) = (if is_legacy {
            #[cfg(feature = "persist_filters_xml")]
            {
                WindowFilter::deserialize_from_xml(&data)
                    .inspect_err(|e| {
                        nwg::error_message(
                            "Virtual Desktop Manager - Import error",
                            &format!("Failed to parse legacy XML filters/rules:\n{e}"),
                        );
                    })
                    .ok()
            }
            #[cfg(not(feature = "persist_filters_xml"))]
            {
                nwg::error_message(
                    "Virtual Desktop Manager - Import error",
                    "This program was compiled without support for legacy XML filters/rules. \
                    Recompile the program from source with the \"persist_filters_xml\" feature \
                    in order to support such filter files.",
                );
                None
            }
        } else {
            #[cfg(feature = "persist_filters")]
            {
                let mut deserializer = serde_json::Deserializer::from_str(&data);
                let result: Result<ExportedWindowFilters, _> = {
                    #[cfg(not(feature = "serde_path_to_error"))]
                    {
                        serde::Deserialize::deserialize(&mut deserializer)
                    }
                    #[cfg(feature = "serde_path_to_error")]
                    {
                        serde_path_to_error::deserialize(&mut deserializer)
                    }
                };
                result
                    .inspect_err(|e| {
                        nwg::error_message(
                            "Virtual Desktop Manager - Import error",
                            &format!("Failed to parse JSON filters/rules:\n{e}"),
                        );
                    })
                    .ok()
                    .map(|info| info.migrate_and_get_filters())
            }
            #[cfg(not(feature = "persist_filters"))]
            {
                nwg::error_message(
                    "Virtual Desktop Manager - Import error",
                    "This program was compiled without support for JSON filters/rules. \
                    Recompile the program from source with the \"persist_filters\" feature \
                    in order to support such filter files.",
                );
                None
            }
        }) else {
            return;
        };
        let Some(tray) = self.tray.get() else {
            return;
        };
        tray.settings().update(|prev| UiSettings {
            filters: prev.filters.iter().cloned().chain(imported).collect(),
            ..prev.clone()
        });
    }
}
/// Methods related to "Configure filter" tab.
impl ConfigWindow {
    fn on_list_view_item_activated(&self, data: &nwg::EventData) {
        tracing::debug!(?data, "ConfigWindow::on_list_view_item_activated");
        let &nwg::EventData::OnListViewItemIndex { row_index, .. } = data else {
            return;
        };
        let group = list_view_item_get_group_id(&self.data_view, row_index);
        if group != Self::GROUP_FILTERS {
            return;
        }
        let Some(filter_index_item) =
            self.data_view
                .item(row_index, Self::COLUMN_FILTERS_INDEX, 10)
        else {
            return;
        };
        let Ok(filter_index) = filter_index_item.text.parse::<usize>() else {
            return;
        };

        self.set_selected_filter_index(Some(filter_index - 1));
    }
    fn on_select_filter_index_changed(&self) {
        let wanted = self.get_selected_filter_index();
        if self.selected_filter_index.get() != wanted {
            self.set_selected_filter_index(wanted);
        }
    }
    fn highlight_selected_filter_in_list(&self) {
        let selected = self.get_selected_filter_index();
        for row_index in 0..self.data_view.len() {
            let group = list_view_item_get_group_id(&self.data_view, row_index);
            if group != Self::GROUP_FILTERS {
                continue;
            }
            if let Some(filter_ix) = self
                .data_view
                .item(row_index, Self::COLUMN_FILTERS_INDEX, 10)
            {
                if let Ok(filter_ix) = filter_ix.text.parse::<usize>() {
                    // UI has one-based index:
                    let filter_ix = filter_ix - 1;
                    if Some(filter_ix) == selected {
                        self.data_view.select_item(row_index, true);
                        continue;
                    }
                }
            }
            self.data_view.select_item(row_index, false);
        }
    }
    fn set_selected_filter_index(&self, index: Option<usize>) {
        self.selected_filter_index.set(index);
        let loaded_filters_len = self
            .loaded_filters
            .borrow()
            .as_deref()
            .unwrap_or_default()
            .len() as i64;
        self.filter_select_index
            .set_data(nwg::NumberSelectData::Int {
                value: index.map(|v| v + 1).unwrap_or(0) as i64,
                step: 1,
                max: loaded_filters_len,
                // 0 means no filter selected:
                min: 0,
            });
        self.highlight_selected_filter_in_list();
        self.set_filter_config_enabled(index.is_some());
        if let Some(index) = index {
            let loaded_filters = self.loaded_filters.borrow().clone();
            let loaded_filters = loaded_filters.as_deref().unwrap_or_default();
            let Some(filter) = loaded_filters.get(index) else {
                self.set_selected_filter_index(None);
                return;
            };
            self.set_filter_config_for_sidebar(filter);
        } else {
            self.set_filter_config_for_sidebar(&Default::default());
        }
    }

    fn set_filter_config_enabled(&self, enabled: bool) {
        self.filter_window_index_lower.set_enabled(enabled);
        self.filter_window_index_lower_checkbox.set_enabled(enabled);
        self.filter_window_index_upper.set_enabled(enabled);
        self.filter_window_index_upper_checkbox.set_enabled(enabled);

        self.filter_desktop_index_lower.set_enabled(enabled);
        self.filter_desktop_index_lower_checkbox
            .set_enabled(enabled);
        self.filter_desktop_index_upper.set_enabled(enabled);
        self.filter_desktop_index_upper_checkbox
            .set_enabled(enabled);

        self.filter_title.set_enabled(enabled);

        self.filter_process.set_enabled(enabled);

        self.filter_action.set_enabled(enabled);

        self.filter_target_desktop.set_enabled(enabled);
    }
    fn set_filter_config_for_sidebar(&self, filter: &WindowFilter) {
        fn set_checked(check_box: &nwg::CheckBox, checked: bool) {
            check_box.set_check_state(if checked {
                nwg::CheckBoxState::Checked
            } else {
                nwg::CheckBoxState::Unchecked
            });
        }
        fn set_text(text_box: &nwg::TextBox, new_text: &str) {
            let new_text = new_text
                .chars()
                .flat_map(|c| {
                    [
                        Some('\r').filter(|_| c == '\n'),
                        Some(c).filter(|&c| c != '\r'),
                    ]
                })
                .flatten()
                .collect::<String>();
            if text_box.text() != new_text {
                text_box.set_text(&new_text);
            }
        }

        // Window Index - Lower Bound:
        set_checked(
            &self.filter_window_index_lower_checkbox,
            filter.window_index.lower_bound.is_some(),
        );
        self.filter_window_index_lower
            .set_data(nwg::NumberSelectData::Int {
                value: filter
                    .window_index
                    .lower_bound
                    .unwrap_or_default()
                    .saturating_add(1)
                    .max(1),
                step: 1,
                max: i64::MAX,
                min: 1,
            });

        // Window Index - Upper Bound:
        set_checked(
            &self.filter_window_index_upper_checkbox,
            filter.window_index.upper_bound.is_some(),
        );
        self.filter_window_index_upper
            .set_data(nwg::NumberSelectData::Int {
                value: filter
                    .window_index
                    .upper_bound
                    .unwrap_or_default()
                    .saturating_add(1)
                    .max(1),
                step: 1,
                max: i64::MAX,
                min: 1,
            });

        // Desktop Index - Lower Bound:
        set_checked(
            &self.filter_desktop_index_lower_checkbox,
            filter.desktop_index.lower_bound.is_some(),
        );
        self.filter_desktop_index_lower
            .set_data(nwg::NumberSelectData::Int {
                value: filter
                    .desktop_index
                    .lower_bound
                    .unwrap_or_default()
                    .saturating_add(1)
                    .max(1),
                step: 1,
                max: i64::MAX,
                min: 1,
            });

        // Desktop Index - Upper Bound:
        set_checked(
            &self.filter_desktop_index_upper_checkbox,
            filter.desktop_index.upper_bound.is_some(),
        );
        self.filter_desktop_index_upper
            .set_data(nwg::NumberSelectData::Int {
                value: filter
                    .desktop_index
                    .upper_bound
                    .unwrap_or_default()
                    .saturating_add(1)
                    .max(1),
                step: 1,
                max: i64::MAX,
                min: 1,
            });

        // Window Title:
        set_text(&self.filter_title, filter.window_title.pattern());

        // Process Name:
        set_text(&self.filter_process, filter.process_name.pattern());

        // Action:
        {
            let index = self
                .filter_action
                .collection()
                .iter()
                .position(|&item| item == filter.action);
            self.filter_action.set_selection(index);
        }

        // Target Desktop:
        self.filter_target_desktop
            .set_data(nwg::NumberSelectData::Int {
                value: filter.target_desktop.saturating_add(1).max(1),
                step: 1,
                max: i64::MAX,
                min: 1,
            });
    }
    fn get_filter_config_for_sidebar(&self) -> Option<WindowFilter> {
        Some(WindowFilter {
            window_index: {
                IntegerRange {
                    lower_bound: if self.filter_window_index_lower_checkbox.check_state()
                        != nwg::CheckBoxState::Checked
                    {
                        None
                    } else if let nwg::NumberSelectData::Int { value, .. } =
                        self.filter_window_index_lower.data()
                    {
                        Some(value.saturating_sub(1).max(0))
                    } else {
                        return None;
                    },
                    upper_bound: if self.filter_window_index_upper_checkbox.check_state()
                        != nwg::CheckBoxState::Checked
                    {
                        None
                    } else if let nwg::NumberSelectData::Int { value, .. } =
                        self.filter_window_index_upper.data()
                    {
                        Some(value.saturating_sub(1).max(0))
                    } else {
                        return None;
                    },
                }
            },
            desktop_index: {
                IntegerRange {
                    lower_bound: if self.filter_desktop_index_lower_checkbox.check_state()
                        != nwg::CheckBoxState::Checked
                    {
                        None
                    } else if let nwg::NumberSelectData::Int { value, .. } =
                        self.filter_desktop_index_lower.data()
                    {
                        Some(value.saturating_sub(1).max(0))
                    } else {
                        return None;
                    },
                    upper_bound: if self.filter_desktop_index_upper_checkbox.check_state()
                        != nwg::CheckBoxState::Checked
                    {
                        None
                    } else if let nwg::NumberSelectData::Int { value, .. } =
                        self.filter_desktop_index_upper.data()
                    {
                        Some(value.saturating_sub(1).max(0))
                    } else {
                        return None;
                    },
                }
            },
            window_title: TextPattern::new(Arc::from(self.filter_title.text().replace('\r', ""))),
            process_name: TextPattern::new(Arc::from(self.filter_process.text().replace('\r', ""))),
            action: 'action: {
                let Some(selected) = self.filter_action.selection() else {
                    break 'action FilterAction::default();
                };
                self.filter_action
                    .collection()
                    .get(selected)
                    .copied()
                    .unwrap_or_default()
            },
            target_desktop: if let nwg::NumberSelectData::Int { value, .. } =
                self.filter_target_desktop.data()
            {
                value.saturating_sub(1).max(0)
            } else {
                return None;
            },
        })
    }

    fn get_selected_filter_index(&self) -> Option<usize> {
        let nwg::NumberSelectData::Int { value, .. } = self.filter_select_index.data() else {
            return None;
        };
        if value < 1 {
            return None;
        }
        Some((value - 1) as usize)
    }

    /// The user changed an options in the "Configure filter" panel.
    fn on_filter_config_ui_changed(&self) {
        let Some(index) = self.get_selected_filter_index() else {
            return;
        };
        let Some(tray) = self.tray.get() else {
            return;
        };
        let Some(new_filter) = self.get_filter_config_for_sidebar() else {
            return;
        };

        tray.settings().update(|prev| UiSettings {
            filters: prev
                .filters
                .iter()
                .cloned()
                .enumerate()
                .map(move |(ix, filter)| {
                    if ix == index {
                        new_filter.clone()
                    } else {
                        filter
                    }
                })
                .collect(),
            ..prev.clone()
        });
    }
    fn on_create_filter(&self) {
        let Some(tray) = self.tray.get() else {
            return;
        };
        tray.settings().update(|prev| UiSettings {
            filters: prev
                .filters
                .iter()
                .cloned()
                .chain(Some(WindowFilter::default()))
                .collect(),
            ..prev.clone()
        });
    }
    fn on_delete_current_filter(&self) {
        let Some(index) = self.get_selected_filter_index() else {
            return;
        };
        let Some(tray) = self.tray.get() else {
            return;
        };
        tray.settings().update(|prev| UiSettings {
            filters: prev
                .filters
                .iter()
                .enumerate()
                .filter(|&(ix, _)| ix != index)
                .map(|(_, filter)| filter.clone())
                .collect(),
            ..prev.clone()
        });
    }
    fn populate_filter_list(&self, filters: &Arc<[WindowFilter]>) {
        let prev_filters = self.loaded_filters.borrow().clone();
        let prev_filters = prev_filters.as_deref().unwrap_or_default();
        let mut indexes_to_skip = Vec::with_capacity(prev_filters.len());

        tracing::trace!(
            old_filters_count = prev_filters.len(),
            new_filters_count = filters.len(),
            "ConfigWindow::populate_filter_list"
        );

        fn get_filter_columns(filter_index: usize, filter: &WindowFilter) -> [String; 6] {
            let WindowFilter {
                window_index,
                desktop_index,
                window_title,
                process_name,
                action: _,
                target_desktop: _,
            } = filter;

            [
                window_index.one_based_indexes().to_string(),
                desktop_index.one_based_indexes().to_string(),
                window_title.display_escaped_newline_glob().to_string(),
                process_name.display_escaped_newline_glob().to_string(),
                filter_index.saturating_add(1).to_string(),
                filter.display_target_desktop().to_string(),
            ]
        }

        // Update existing filter items:
        for ix in (0..self.data_view.len()).rev() {
            let group = list_view_item_get_group_id(&self.data_view, ix);
            if group != Self::GROUP_FILTERS {
                continue;
            }
            if let Some(filter_ix) = self.data_view.item(ix, Self::COLUMN_FILTERS_INDEX, 10) {
                if let Ok(filter_ix) = filter_ix.text.parse::<usize>() {
                    // UI has one-based index:
                    let filter_ix = filter_ix - 1;
                    if let Some(prev) = prev_filters.get(filter_ix) {
                        if let Some(new) = filters.get(filter_ix) {
                            if prev != new {
                                let info = get_filter_columns(filter_ix, new);
                                for (column_ix, text) in info.into_iter().enumerate() {
                                    self.data_view.update_item(
                                        ix,
                                        nwg::InsertListViewItem {
                                            index: Some(ix as _),
                                            column_index: column_ix as _,
                                            text: Some(text),
                                            image: None,
                                        },
                                    );
                                }
                            }
                            indexes_to_skip.push(filter_ix);
                            continue;
                        }
                    }
                }
            }
            self.data_view.remove_item(ix);
        }
        tracing::trace!(
            updated_filter_indexes = ?indexes_to_skip,
            "ConfigWindow::populate_filter_list updated {} items and will create {}",
            indexes_to_skip.len(),
            filters.len() - indexes_to_skip.len()
        );

        // Create new filter items:
        for (filter_index, filter) in filters.iter().enumerate() {
            if indexes_to_skip.contains(&filter_index) {
                continue;
            }
            let info = get_filter_columns(filter_index, filter);
            self.data_view.insert_items_row(None, &info);
            list_view_item_set_group_id(
                &self.data_view,
                self.data_view.len().saturating_sub(1),
                Some(Self::GROUP_FILTERS),
            );
        }
        self.loaded_filters.replace(Some(filters.clone()));
        self.is_data_sorted.set(false);

        // Windows might now be affected by different filters:
        self.update_window_infos();

        self.resort_items();

        // Update sidebar with config for selected filter:
        let selected_filter = if let Some(prev_selected) = self.get_selected_filter_index() {
            if prev_selected >= filters.len() {
                // Prev selection was removed, select last remaining:
                Some(filters.len().saturating_sub(1))
            } else if !prev_filters.is_empty() && filters.len() > prev_filters.len() {
                // New items were added to an existing list, select newest item:
                Some(filters.len().saturating_sub(1))
            } else {
                Some(prev_selected)
            }
        } else {
            None
        };
        self.set_selected_filter_index(selected_filter);
    }
    fn sync_filter_from_settings(&self, settings: Option<&Arc<UiSettings>>) {
        let settings_owned;
        let settings = match settings {
            Some(s) => s,
            None => {
                let Some(tray) = self.tray.get() else {
                    return;
                };
                settings_owned = tray.settings().get();
                &settings_owned
            }
        };
        self.populate_filter_list(&settings.filters);
    }
}
/// Methods related to "Program settings" tab.
impl ConfigWindow {
    fn on_settings_ui_changed(&self) {
        let auto_start = self
            .settings_auto_start
            .selection()
            .and_then(|ix| self.settings_auto_start.collection().get(ix).copied())
            .unwrap_or_default();
        let tray_icon_type = self
            .settings_tray_icon
            .selection()
            .and_then(|ix| self.settings_tray_icon.collection().get(ix).copied())
            .unwrap_or_default();
        let quick_switch_menu = self
            .settings_quick_menu
            .selection()
            .and_then(|ix| self.settings_quick_menu.collection().get(ix).copied())
            .unwrap_or_default();
        let left_click = self
            .settings_left_click
            .selection()
            .and_then(|ix| self.settings_left_click.collection().get(ix).copied())
            .unwrap_or_default();
        let middle_click = self
            .settings_middle_click
            .selection()
            .and_then(|ix| self.settings_middle_click.collection().get(ix).copied())
            .unwrap_or_default();
        let mut quick_shortcuts_count = 0;
        let mut invalid_quick_shortcut_target = false;
        let quick_switch_menu_shortcuts = Arc::new(
            self.settings_quick_menu_shortcuts
                .text()
                .split('\n')
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
        let quick_switch_hotkey = Arc::<str>::from(
            self.settings_quick_menu_hotkey
                .text()
                .trim_matches(['\n', '\r']),
        );
        let open_menu_at_mouse_pos_hotkey = Arc::<str>::from(
            self.settings_open_menu_at_mouse_pos_hotkey
                .text()
                .trim_matches(['\n', '\r']),
        );
        tracing::debug!(
            settings_start_as_admin = ?self.settings_start_as_admin.check_state(),
            settings_prevent_flashing_windows = ?self.settings_prevent_flashing_windows.check_state(),
            settings_smooth_switch_desktop = ?self.settings_smooth_switch_desktop.check_state(),
            ?auto_start,
            ?tray_icon_type,
            ?quick_switch_menu,
            ?quick_switch_menu_shortcuts,
            settings_quick_menu_shortcuts_in_submenus =? self.settings_quick_menu_shortcuts_in_submenus.check_state(),
            ?quick_switch_hotkey,
            ?left_click,
            ?middle_click,
            ?open_menu_at_mouse_pos_hotkey,
            "ConfigWindow::on_settings_ui_changed"
        );
        if invalid_quick_shortcut_target
            || quick_shortcuts_count != quick_switch_menu_shortcuts.len()
        {
            // Had duplicates
            tracing::debug!(
                "Invalid numbers or duplicated items in quick switch shortcuts field, \
                restoring to current settings value"
            );
            self.sync_quick_shortcuts_from(&quick_switch_menu_shortcuts);
        }
        let Some(tray) = self.tray.get() else {
            return;
        };
        tray.settings().update(|prev| UiSettings {
            request_admin_at_startup: self.settings_start_as_admin.check_state()
                == nwg::CheckBoxState::Checked,
            auto_start,
            stop_flashing_windows_after_applying_filter: self
                .settings_prevent_flashing_windows
                .check_state()
                == nwg::CheckBoxState::Checked,
            smooth_switch_desktops: self.settings_smooth_switch_desktop.check_state()
                == nwg::CheckBoxState::Checked,
            tray_icon_type,
            quick_switch_menu,
            quick_switch_menu_shortcuts,
            quick_switch_menu_shortcuts_only_in_root: self
                .settings_quick_menu_shortcuts_in_submenus
                .check_state()
                != nwg::CheckBoxState::Checked,
            quick_switch_hotkey,
            left_click,
            middle_click,
            open_menu_at_mouse_pos_hotkey,
            ..prev.clone()
        });
    }
    fn sync_program_options_from_settings(&self, settings: Option<&Arc<UiSettings>>) {
        let settings_owned;
        let settings = match settings {
            Some(s) => s,
            None => {
                let Some(tray) = self.tray.get() else {
                    return;
                };
                settings_owned = tray.settings().get();
                &settings_owned
            }
        };
        fn set_checked(check_box: &nwg::CheckBox, checked: bool) {
            check_box.set_check_state(if checked {
                nwg::CheckBoxState::Checked
            } else {
                nwg::CheckBoxState::Unchecked
            });
        }
        set_checked(
            &self.settings_start_as_admin,
            settings.request_admin_at_startup,
        );
        {
            let index = self
                .settings_auto_start
                .collection()
                .iter()
                .position(|&item| item == settings.auto_start);
            self.settings_auto_start.set_selection(index);
        }
        set_checked(
            &self.settings_prevent_flashing_windows,
            settings.stop_flashing_windows_after_applying_filter,
        );
        set_checked(
            &self.settings_smooth_switch_desktop,
            settings.smooth_switch_desktops,
        );
        {
            let index = self
                .settings_tray_icon
                .collection()
                .iter()
                .position(|&item| item == settings.tray_icon_type);
            self.settings_tray_icon.set_selection(index);
        }
        {
            let index = self
                .settings_quick_menu
                .collection()
                .iter()
                .position(|&item| item == settings.quick_switch_menu);
            self.settings_quick_menu.set_selection(index);
        }
        self.sync_quick_shortcuts_from(&settings.quick_switch_menu_shortcuts);
        set_checked(
            &self.settings_quick_menu_shortcuts_in_submenus,
            !settings.quick_switch_menu_shortcuts_only_in_root,
        );
        {
            let new_text = &*settings.quick_switch_hotkey;
            if new_text != self.settings_quick_menu_hotkey.text() {
                self.settings_quick_menu_hotkey.set_text(new_text);
            }
            self.settings_quick_menu_hotkey_error.set_text(&{
                if settings.quick_switch_hotkey.is_empty() {
                    "Hotkey disabled".to_owned()
                } else {
                    #[cfg(feature = "global_hotkey")]
                    {
                        match global_hotkey::hotkey::HotKey::from_str(&settings.quick_switch_hotkey)
                        {
                            Ok(_) => "Valid hotkey".to_owned(),
                            Err(e) => format!("Invalid hotkey: {e}"),
                        }
                    }
                    #[cfg(not(feature = "global_hotkey"))]
                    {
                        "Compiled without hotkey support".to_owned()
                    }
                }
            });
        }
        {
            let index = self
                .settings_left_click
                .collection()
                .iter()
                .position(|&item| item == settings.left_click);
            self.settings_left_click.set_selection(index);
        }
        {
            let index = self
                .settings_middle_click
                .collection()
                .iter()
                .position(|&item| item == settings.middle_click);
            self.settings_middle_click.set_selection(index);
        }
        {
            let new_text = &*settings.open_menu_at_mouse_pos_hotkey;
            if new_text != self.settings_open_menu_at_mouse_pos_hotkey.text() {
                self.settings_open_menu_at_mouse_pos_hotkey
                    .set_text(new_text);
            }
            self.settings_open_menu_at_mouse_pos_hotkey_error
                .set_text(&{
                    if settings.open_menu_at_mouse_pos_hotkey.is_empty() {
                        "Hotkey disabled".to_owned()
                    } else {
                        #[cfg(feature = "global_hotkey")]
                        {
                            match global_hotkey::hotkey::HotKey::from_str(
                                &settings.open_menu_at_mouse_pos_hotkey,
                            ) {
                                Ok(_) => "Valid hotkey".to_owned(),
                                Err(e) => format!("Invalid hotkey: {e}"),
                            }
                        }
                        #[cfg(not(feature = "global_hotkey"))]
                        {
                            "Compiled without hotkey support".to_owned()
                        }
                    }
                });
        }
    }
    fn sync_quick_shortcuts_from(&self, shortcuts: &BTreeMap<String, u32>) {
        let selection = self.settings_quick_menu_shortcuts.selection();
        let text = shortcuts.iter().fold(
            String::with_capacity(shortcuts.len() * 4),
            |mut f, (mut key, target)| {
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
            },
        );
        self.settings_quick_menu_shortcuts.set_text(&text);
        let mut selection =
            selection.start.min(text.len() as u32)..selection.end.min(text.len() as u32);

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
            selected_and_prev =? String::from_utf8_lossy(selected_and_prev),
            range =? selection,
            "Updating Quick switch shortcut text box selection"
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

        self.settings_quick_menu_shortcuts.set_selection(selection);
    }
}
impl DynamicUiHooks<SystemTray> for ConfigWindow {
    fn before_partial_build(
        &mut self,
        dynamic_ui: &Rc<SystemTray>,
        should_build: &mut bool,
    ) -> Option<(nwg::ControlHandle, std::any::TypeId)> {
        self.tray.set(dynamic_ui);
        if !self.open_soon.replace(false) {
            *should_build = false;
        }
        None
    }
    fn after_partial_build(&mut self, _dynamic_ui: &Rc<SystemTray>) {
        if let Err(e) = self.build_layout() {
            tracing::error!(
                error = e.to_string(),
                "Failed to build layout for ConfigWindow"
            );
        }
        if let Err(e) = self.build_tooltip() {
            tracing::error!(
                error = e.to_string(),
                "Failed to build tooltips for ConfigWindow"
            );
        }

        self.sync_program_options_from_settings(None);
        self.set_as_foreground_window();
    }
    fn after_handles<'a>(
        &'a self,
        _dynamic_ui: &Rc<SystemTray>,
        handles: &mut Vec<&'a nwg::ControlHandle>,
    ) {
        *handles = vec![&self.window.handle];
    }

    fn need_rebuild(&self, _dynamic_ui: &Rc<SystemTray>) -> bool {
        // Note: we should remain open even if open_soon is false.
        self.open_soon.get() && self.is_closed()
    }
    fn is_ordered_in_parent(&self) -> bool {
        false
    }
    fn before_rebuild(&mut self, _dynamic_ui: &Rc<SystemTray>) {
        let export_dialog = std::mem::take(&mut self.export_dialog);
        let import_dialog = std::mem::take(&mut self.import_dialog);
        *self = Default::default();
        self.export_dialog = export_dialog;
        self.import_dialog = import_dialog;
        // need_rebuild would only return true if open_soon was true, so
        // remember it:
        self.open_soon = Cell::new(true);
    }
}
impl TrayPlugin for ConfigWindow {
    fn on_settings_changed(
        &self,
        _tray_ui: &Rc<SystemTray>,
        _prev: &Arc<UiSettings>,
        new: &Arc<UiSettings>,
    ) {
        self.sync_program_options_from_settings(Some(new));
        let has_changed_filters =
            self.loaded_filters.borrow().as_deref().unwrap_or_default() != &*new.filters;
        if has_changed_filters {
            self.sync_filter_from_settings(Some(new));
        }
    }
}
