use crate::filter_options::{FilterOptionsHooks, FilterOptionsPanel};
use crate::layout::LayoutArea;
use crate::program_settings::{ProgramSettingsHooks, ProgramSettingsPanel};
use crate::{
    custom_msg, filter_options, NativeWindowHandle, SharedState, SharedStateMut, WindowState,
    WinsafeHandleToRawHandle,
};
use std::cell::{Cell, OnceCell, RefCell};
use std::cmp::Ordering;
use std::error::Error;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{mpsc, Arc};
use virtual_desktop_manager_core::settings::{ConfigWindowInfo, UiSettings};
use virtual_desktop_manager_core::vd;
use virtual_desktop_manager_core::window_filter::{ExportedWindowFilters, WindowFilter};
use virtual_desktop_manager_core::window_info::WindowInfo;
use winsafe::gui::Icon;
use winsafe::msg::lvm::{EnableGroupView, SetItem};
use winsafe::msg::wm::{CtlColorStatic, SetFont};
use winsafe::msg::WndMsg;
use winsafe::prelude::*;
use winsafe::{co, gui, WString, LVGROUP, LVITEM, POINT, WINDOWPLACEMENT};

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

enum DataViewItem {
    WindowInfo(WindowInfo),
    Filter(WindowFilter),
}
impl DataViewItem {
    pub fn is_window_info(&self) -> bool {
        matches!(self, Self::WindowInfo { .. })
    }
    pub fn is_filter(&self) -> bool {
        matches!(self, Self::Filter { .. })
    }
    pub fn compare_kind(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::WindowInfo(_), Self::Filter(_)) => Ordering::Less,
            (Self::Filter(_), Self::WindowInfo(_)) => Ordering::Greater,
            _ => Ordering::Equal,
        }
    }
}

pub struct WinsafeSettingsWindow {
    /// Responsible for managing the settings window.
    wnd: gui::WindowMain,
    data_view: gui::ListView<(usize, DataViewItem)>,
    _tab_area_parent: gui::WindowControl,
    tab_area: gui::Tab,
    btn_import: gui::Button,
    btn_export: gui::Button,
    btn_refresh: gui::Button,
    btn_apply_filters: gui::Button,

    filter_options_page: gui::TabPage,
    filter_options_panel: Rc<FilterOptionsPanel>,
    program_settings_page: gui::TabPage,
    program_settings_panel: Rc<ProgramSettingsPanel>,

    tooltips: OnceCell<nwg::Tooltip>,

    shared: Arc<SharedState>,
    background_thread: RefCell<Option<BackgroundThread>>,
    has_queued_refresh: Cell<bool>,
    is_data_sorted: Cell<bool>,
    loaded_settings: RefCell<Arc<UiSettings>>,
}
impl WinsafeSettingsWindow {
    const MIN_SIZE: (i32, i32) = (400, 1000);

    const GROUP_WINDOWS: i32 = 1;
    const GROUP_FILTERS: i32 = 2;

    const COLUMN_FILTERS_INDEX: u32 = 4;
    const COLUMN_TARGET_DESKTOP: u32 = 5;

    pub fn new(shared: Arc<SharedState>) -> Rc<Self> {
        let new_self = Rc::new_cyclic(|weak_this| {
            let import_height = 30;
            let action_height = 50;
            let sidebar_width = 270;

            let settings = shared.mutex.lock().unwrap().tracked_settings.new.clone();

            let mut window_layout = LayoutArea::from_size(
                settings.config_window.size.0 as i32,
                settings.config_window.size.1 as i32,
            );

            let wnd = gui::WindowMain::new(
                // instantiate the window manager
                gui::WindowMainOpts {
                    title: "Virtual Desktop Manager",
                    class_icon: Icon::Id(1),
                    size: window_layout.dpi_size(),
                    style: gui::WindowMainOpts::default().style
                        | co::WS::SIZEBOX
                        | co::WS::MINIMIZEBOX
                        | co::WS::MAXIMIZEBOX,
                    ..Default::default() // leave all other options as default
                },
            );

            window_layout.apply_margin(); // free space at edges of window
            let mut sidebar_layout = window_layout.take_right(sidebar_width);

            let data_view = gui::ListView::new(
                &wnd,
                gui::ListViewOpts {
                    position: window_layout.dpi_pos(),
                    size: window_layout.dpi_size().into(),
                    control_style: co::LVS::REPORT | co::LVS::SHOWSELALWAYS,
                    control_ex_style: co::LVS_EX::DOUBLEBUFFER
                        | co::LVS_EX::HEADERDRAGDROP
                        | co::LVS_EX::FULLROWSELECT,
                    resize_behavior: (gui::Horz::Resize, gui::Vert::Resize),
                    columns: &[
                        ("Window Index", 120),
                        ("Virtual Desktop", 120),
                        ("Window Title", 220),
                        ("Process Name", 220),
                        ("Filter Index", 120),
                        ("Target Desktop", 120),
                    ],
                    ..Default::default()
                },
            );
            let action_layout = sidebar_layout.take_bottom(action_height);
            let import_layout = sidebar_layout.take_bottom(import_height);

            let tab_area_parent = gui::WindowControl::new(
                &wnd,
                gui::WindowControlOpts {
                    position: sidebar_layout.dpi_pos(),
                    size: sidebar_layout.dpi_size(),
                    resize_behavior: (gui::Horz::Repos, gui::Vert::Resize),
                    ex_style: gui::WindowControlOpts::default().ex_style | co::WS_EX::CONTROLPARENT,
                    ..Default::default()
                },
            );
            let mut sidebar_tab_layout = sidebar_layout.clone();
            // Panel x and y coordinates reset because new parent
            sidebar_tab_layout.x = 0;
            sidebar_tab_layout.y = 0;
            sidebar_tab_layout.height -= 30; // some space taken by the header with the tab names
            sidebar_tab_layout.width -= 10; // some width lost to TabPage
            sidebar_tab_layout.apply_margin();
            sidebar_tab_layout.margin = 5;

            let filter_options_page =
                gui::TabPage::new(&tab_area_parent, gui::TabPageOpts::default());
            let filter_options_panel = FilterOptionsPanel::new(
                &filter_options_page,
                &mut sidebar_tab_layout.clone(),
                weak_this.clone(),
            );
            let program_settings_page =
                gui::TabPage::new(&tab_area_parent, gui::TabPageOpts::default());
            let program_settings_panel = ProgramSettingsPanel::new(
                &program_settings_page,
                &mut sidebar_tab_layout.clone(),
                weak_this.clone(),
            );

            let tab_area = gui::Tab::new(
                &tab_area_parent,
                gui::TabOpts {
                    size: sidebar_layout.dpi_size(),
                    pages: &[
                        ("Filter options", filter_options_page.clone()),
                        ("Program settings", program_settings_page.clone()),
                    ],
                    resize_behavior: (gui::Horz::None, gui::Vert::Resize),
                    ..Default::default()
                },
            );

            let [import_layout, export_layout] = import_layout.split_horizontal();
            let btn_import = gui::Button::new(
                &wnd,
                gui::ButtonOpts {
                    text: "Import filters",
                    position: import_layout.dpi_pos(),
                    width: import_layout.dpi_width(),
                    height: import_layout.dpi_height(),
                    resize_behavior: (gui::Horz::Repos, gui::Vert::Repos),
                    ..Default::default()
                },
            );
            let btn_export = gui::Button::new(
                &wnd,
                gui::ButtonOpts {
                    text: "Export filters",
                    position: export_layout.dpi_pos(),
                    width: export_layout.dpi_width(),
                    height: export_layout.dpi_height(),
                    resize_behavior: (gui::Horz::Repos, gui::Vert::Repos),
                    ..Default::default()
                },
            );

            let [refresh_layout, apply_filters_layout] = action_layout.split_horizontal();
            let btn_refresh = gui::Button::new(
                &wnd,
                gui::ButtonOpts {
                    text: "&Refresh info",
                    position: refresh_layout.dpi_pos(),
                    width: refresh_layout.dpi_width(),
                    height: refresh_layout.dpi_height(),
                    resize_behavior: (gui::Horz::Repos, gui::Vert::Repos),
                    ..Default::default()
                },
            );
            let btn_apply_filters = gui::Button::new(
                &wnd,
                gui::ButtonOpts {
                    text: "Apply filters",
                    position: apply_filters_layout.dpi_pos(),
                    width: apply_filters_layout.dpi_width(),
                    height: apply_filters_layout.dpi_height(),
                    resize_behavior: (gui::Horz::Repos, gui::Vert::Repos),
                    ..Default::default()
                },
            );

            {
                let mut guard = shared.mutex.lock().unwrap();
                guard.window = Some(wnd.clone());
                guard.state = WindowState::Open;
            }
            Self {
                wnd,
                data_view,
                _tab_area_parent: tab_area_parent,
                tab_area,
                btn_import,
                btn_export,
                btn_refresh,
                btn_apply_filters,
                filter_options_panel,
                filter_options_page,
                program_settings_panel,
                program_settings_page,
                tooltips: OnceCell::new(),
                shared,
                background_thread: RefCell::new(None),
                has_queued_refresh: Cell::new(false),
                is_data_sorted: Cell::new(true),
                loaded_settings: RefCell::new(settings),
            }
        });
        new_self.events(); // attach our events
        new_self
    }

    pub fn run(&self) -> winsafe::AnyResult<i32> {
        self.wnd.run_main(None) // show the main window; will block until closed
    }

    fn events(self: &Rc<Self>) {
        // The tab pages have white background but labels default to grey background, this fixes that:
        let transparent_background = move |msg: CtlColorStatic| {
            msg.hdc.SetBkMode(co::BKMODE::TRANSPARENT)?;

            Ok(winsafe::HBRUSH::GetSysColorBrush(co::COLOR::WINDOW)?)
        };
        self.filter_options_page
            .on()
            .wm_ctl_color_static(transparent_background);
        self.program_settings_page
            .on()
            .wm_ctl_color_static(transparent_background);

        self.wnd.on().wm_set_font({
            // Forward set font command to child controls
            // https://stackoverflow.com/questions/938216/how-to-set-default-font-for-all-the-windows-in-a-win32-application/939656#939656
            let this = Rc::downgrade(self);
            move |mut msg| {
                tracing::trace!("WinsafeSettingsWindow.wnd.wm_set_font");
                let Some(this) = this.upgrade() else {
                    return Ok(());
                };
                let handles = [
                    this.data_view.hwnd(),
                    this.tab_area.hwnd(),
                    this.btn_export.hwnd(),
                    this.btn_import.hwnd(),
                    this.btn_refresh.hwnd(),
                    this.btn_apply_filters.hwnd(),
                ];
                for handle in handles {
                    unsafe { handle.SendMessage(msg.as_generic_wm()) };
                }

                this.filter_options_panel.set_font(&mut msg);
                this.program_settings_panel.set_font(&mut msg);

                Ok(())
            }
        });
        self.wnd.on().wm_create({
            let this = Rc::downgrade(self);
            move |_| {
                tracing::trace!("WinsafeSettingsWindow.wnd.wm_create");
                let Some(this) = this.upgrade() else {
                    return Ok(0);
                };
                if let Some(font) = nwg::Font::global_default() {
                    unsafe {
                        this.wnd.hwnd().SendMessage(SetFont {
                            hfont: winsafe::HFONT::from_ptr(font.handle as *mut _),
                            redraw: true,
                        })
                    };
                }
                this.build_tooltips();

                unsafe {
                    this.data_view
                        .hwnd()
                        .SendMessage(EnableGroupView { enable: true })
                }
                    .expect("Failed to enable groups for list view");

                let groups = &[
                    (Self::GROUP_WINDOWS, "Active Windows"),
                    (Self::GROUP_FILTERS, "Filters / Rules"),
                ];
                for &(id, header) in groups {
                    let mut group = LVGROUP::default();
                    group.iGroupId = id;
                    group.mask |= co::LVGF::GROUPID;

                    let mut header = WString::from_str(header);
                    group.set_pszHeader(Some(&mut header));
                    group.mask |= co::LVGF::HEADER;

                    group.uAlign = co::LVGA_FH::HEADER_LEFT;
                    group.mask |= co::LVGF::ALIGN;

                    // https://learn.microsoft.com/en-us/windows/win32/controls/lvm-insertgroup
                    let result = unsafe {
                        this.data_view.hwnd().SendMessage(WndMsg {
                            msg_id: co::LVM::INSERTGROUP.into(),
                            wparam: (-1_isize) as usize,
                            lparam: (&mut group as *mut LVGROUP) as isize,
                        })
                    };
                    if result < 0 {
                        panic!("Failed to create group for list view");
                    }
                }

                if let Err(e) = this.data_view.focus() {
                    tracing::error!(error =? e, "Failed to focus on list view");
                }

                let settings = this
                    .shared
                    .mutex
                    .lock()
                    .unwrap()
                    .tracked_settings
                    .new
                    .clone();
                this.reload_from_settings(&settings);
                this.populate_filter_list(&settings.filters);
                this.gather_window_info();

                if let Some((x, y)) = settings.config_window.position {
                    if let Err(e) = this.wnd.hwnd().SetWindowPos(
                        winsafe::HwndPlace::None,
                        POINT::with(gui::dpi_x(x), gui::dpi_y(y)),
                        winsafe::SIZE::default(),
                        co::SWP::NOZORDER
                            | co::SWP::NOSIZE
                            | co::SWP::NOACTIVATE
                            | co::SWP::NOOWNERZORDER,
                    ) {
                        tracing::error!(error =? e, "Failed to set initial position for config window");
                    }
                }

                if settings.config_window.maximized {
                    let result = unsafe {
                        this.wnd.hwnd().PostMessage(WndMsg {
                            msg_id: custom_msg::DELAYED_MAXIMIZE,
                            wparam: 0,
                            lparam: 0,
                        })
                    };
                    if let Err(e) = result {
                        tracing::error!(?e, "Failed to post delayed maximize message");
                    }
                }

                Ok(0)
            }
        });
        self.wnd.on().wm_exit_size_move({
            let this = Rc::downgrade(self);
            move || {
                tracing::trace!("WinsafeSettingsWindow.wnd.wm_exit_size_move");
                let Some(this) = this.upgrade() else {
                    return Ok(());
                };
                // Save current position
                this.save_position_and_size();
                Ok(())
            }
        });
        self.wnd.on().wm_move({
            let this = Rc::downgrade(self);
            move |_| {
                tracing::trace!("WinsafeSettingsWindow.wnd.wm_move");
                let Some(this) = this.upgrade() else {
                    return Ok(());
                };
                // Save current position
                this.save_position_and_size();
                Ok(())
            }
        });
        self.wnd.on().wm_get_min_max_info(|info| {
            tracing::trace!("WinsafeSettingsWindow.wnd.wm_get_min_max_info");
            info.info.ptMinTrackSize = POINT::from(Self::MIN_SIZE);
            Ok(())
        });
        self.wnd.on().wm(custom_msg::DELAYED_MAXIMIZE, {
            let wnd = self.wnd.clone();
            move |_| {
                wnd.hwnd().ShowWindow(co::SW::SHOWMAXIMIZED);
                /*
                let mut wp = WINDOWPLACEMENT::default();
                if let Err(e) = this.wnd.hwnd().GetWindowPlacement(&mut wp) {
                    tracing::error!(error = ?e, "GetWindowPlacement failed at window creation")
                } else {
                    dbg!(wp.rcNormalPosition.to_string());
                    wp.showCmd = co::SW::SHOWMAXIMIZED;
                    if let Err(e) = this.wnd.hwnd().SetWindowPlacement(&wp) {
                        tracing::error!(error = ?e, "SetWindowPlacement failed at window creation")
                    }
                }
                */
                Ok(0)
            }
        });
        self.wnd.on().wm(custom_msg::STATE_CHANGED, {
            let this = Rc::downgrade(self);
            move |_params| {
                tracing::trace!("WinsafeSettingsWindow.wnd.wm(custom_msg::STATE_CHANGED)");
                let Some(this) = this.upgrade() else {
                    return Ok(0);
                };
                let mut guard = this.shared.mutex.lock().unwrap();
                let new_settings = Some(&guard.tracked_settings.new)
                    .filter(|new| {
                        let already_loaded = Arc::ptr_eq(new, &this.loaded_settings.borrow());
                        !already_loaded
                    })
                    .cloned();
                match guard.state {
                    WindowState::Open => {
                        drop(guard);
                    }
                    WindowState::Refocus => {
                        guard.state = WindowState::Open;
                        drop(guard);
                        this.wnd.hwnd().SetForegroundWindow();
                    }
                    WindowState::Closed => {
                        drop(guard);
                        this.wnd.close();
                        return Ok(0);
                    }
                }
                if let Some(new) = new_settings {
                    tracing::info!("Reloading config window from program settings");
                    this.reload_from_settings(&new);
                }
                Ok(0)
            }
        });
        self.wnd.on().wm(custom_msg::WINDOW_INFO_AVAILABLE, {
            let this = Rc::downgrade(self);
            move |_param| {
                tracing::trace!("WinsafeSettingsWindow.wnd.wm(custom_msg::WINDOW_INFO_AVAILABLE)");
                let Some(this) = this.upgrade() else {
                    return Ok(0);
                };
                let Ok(guard) = this.background_thread.try_borrow() else {
                    tracing::warn!("Received notice from background thread while RefCell was locked, might delay a table update");
                    return Ok(0);
                };
                let Some(background) = &*guard else {
                    tracing::warn!(
                        "Received notice from background thread, but no such thread was running"
                    );
                    return Ok(0);
                };
                loop {
                    match background.rx.try_recv() {
                        Ok(window) => {
                            tracing::trace!(info = ?window, "Received window info from background thread");
                            this.add_window_info(window);
                            continue;
                        }
                        Err(mpsc::TryRecvError::Disconnected) => {
                            // Got all data!
                            drop(guard);
                            if !this.is_data_sorted.get() {
                                this.resort_items();
                            }
                            if this.has_queued_refresh.get() {
                                this.gather_window_info();
                            }
                        }
                        Err(mpsc::TryRecvError::Empty) => {
                            // Will get more data later
                        }
                    }
                    break;
                }
                Ok(0)
            }
        });
        self.data_view.on().lvn_column_click({
            let this = Rc::downgrade(self);
            move |event| {
                tracing::trace!("WinsafeSettingsWindow.data_view.lvn_column_click");
                // Gather required information about event:
                let column_index = event.iSubItem;
                let Some(this) = this.upgrade() else {
                    return Ok(());
                };
                let header = this.data_view.header().unwrap();
                let column = header.items().get(column_index as u32);
                let sort_dir = this.get_arrow_direction(&column);

                // Set new sort direction:
                let new_dir = match sort_dir {
                    gui::HeaderArrow::None => gui::HeaderArrow::Desc,
                    gui::HeaderArrow::Desc => gui::HeaderArrow::Asc,
                    gui::HeaderArrow::Asc => gui::HeaderArrow::None,
                };
                column.set_arrow(new_dir);
                column.set_lparam(co::HDF::from(new_dir).raw() as isize);

                // Reset sorting direction of other columns:
                for column in header.items().iter()? {
                    if column.index() as i32 == column_index {
                        continue;
                    }
                    column.set_arrow(gui::HeaderArrow::None);
                    column.set_lparam(co::HDF::from(gui::HeaderArrow::None).raw() as isize);
                }

                // Apply new sorting direction:
                this.resort_items();
                Ok(())
            }
        });
        self.data_view.on().lvn_item_activate({
            let this = Rc::downgrade(self);
            move |event| {
                tracing::trace!(
                    index = ?event.iItem,
                    "WinsafeSettingsWindow.data_view.lvn_item_activate"
                );
                let Some(this) = this.upgrade() else {
                    return Ok(());
                };
                let item = this.data_view.items().get(event.iItem as u32);
                let data = item.data();
                let guard = data.borrow();
                if let &(index, DataViewItem::Filter(_)) = &*guard {
                    tracing::debug!(index = index, "Activated a filter");
                    drop(guard);
                    this.set_selected_filter(Some(index));
                }
                Ok(())
            }
        });
        self.btn_refresh.on().bn_clicked({
            let this = Rc::downgrade(self);
            move || {
                tracing::trace!("WinsafeSettingsWindow.btn_refresh.bn_clicked");
                let Some(this) = this.upgrade() else {
                    return Ok(());
                };
                this.gather_window_info();
                Ok(())
            }
        });
        self.btn_apply_filters.on().bn_clicked({
            let this = Rc::downgrade(self);
            move || {
                tracing::trace!("WinsafeSettingsWindow.btn_apply_filters.bn_clicked");
                let Some(this) = this.upgrade() else {
                    return Ok(());
                };
                SharedStateMut::notify_main_to_apply_filters(this.shared.mutex.lock().unwrap());
                Ok(())
            }
        });
        self.btn_import.on().bn_clicked({
            let this = Rc::downgrade(self);
            move || {
                tracing::trace!("WinsafeSettingsWindow.btn_import.bn_clicked");
                if let Some(this) = this.upgrade() {
                    this.import_filters_with_dialog();
                }
                Ok(())
            }
        });
        self.btn_export.on().bn_clicked({
            let this = Rc::downgrade(self);
            move || {
                tracing::trace!("WinsafeSettingsWindow.btn_export.bn_clicked");
                if let Some(this) = this.upgrade() {
                    this.export_filters_with_dialog();
                }
                Ok(())
            }
        });
    }
    fn save_position_and_size(&self) {
        let mut placement = WINDOWPLACEMENT::default();
        if let Err(e) = self.wnd.hwnd().GetWindowPlacement(&mut placement) {
            tracing::error!(error = ?e, "Failed to get window placement");
        }

        let Ok(client_rect) = self
            .wnd
            .hwnd()
            .GetClientRect()
            .map_err(|e| tracing::error!(error = ?e, "GetClientRect failed"))
        else {
            return;
        };

        let maximized = placement.showCmd == co::SW::SHOWMAXIMIZED;
        let minimized = placement.showCmd == co::SW::SHOWMINIMIZED;
        let window_pos = (
            placement.rcNormalPosition.left,
            placement.rcNormalPosition.top,
        );
        let window_size = (
            (placement.rcNormalPosition.right as u32)
                .saturating_sub(placement.rcNormalPosition.left as u32),
            (placement.rcNormalPosition.bottom as u32)
                .saturating_sub(placement.rcNormalPosition.top as u32),
        );

        let client_size = (
            (client_rect.right as u32).saturating_sub(client_rect.left as u32),
            (client_rect.bottom as u32).saturating_sub(client_rect.top as u32),
        );

        tracing::trace!(
            window_position =? window_pos,
            window_size =? window_size,
            client_rect = client_rect.to_string(),
            client_size =? client_size,
            placement =? placement.showCmd,
            maximized,
            "Config window resized or moved"
        );
        if minimized {
            return;
        }

        self.update_settings(|prev| UiSettings {
            config_window: if maximized {
                // Don't save size and position of maximized window:
                ConfigWindowInfo {
                    maximized,
                    ..prev.config_window
                }
            } else {
                ConfigWindowInfo {
                    position: Some(window_pos),
                    size: client_size,
                    maximized,
                }
            },
            ..prev.clone()
        });
    }

    fn build_tooltips(&self) {
        if self.tooltips.get().is_some() {
            return;
        }
        let mut tooltip = nwg::Tooltip::default();
        let result = nwg::Tooltip::builder()
            .register(
                self.btn_import.native_handle(),
                "Add new filters by loading them from a selected file.",
            )
            .register(
                self.btn_export.native_handle(),
                "Save all filters to a file",
            )
            .register(
                self.btn_refresh.native_handle(),
                "Reload info about all open windows",
            )
            .register(
                self.btn_apply_filters.native_handle(),
                "Use the configured filters to move windows to specific virtual desktops",
            )
            .build(&mut tooltip);
        if let Err(e) = result {
            tracing::error!(error = ?e, "Failed to build tooltips for WinsafeSettingsWindow");
        } else {
            tracing::debug!("Built tooltips for WinsafeSettingsWindow");
            _ = self.tooltips.set(tooltip);
        }
    }
}
/// Import/export filters
impl WinsafeSettingsWindow {
    pub fn export_filters_to_xml_string(&self) -> Result<String, Box<dyn Error>> {
        #[cfg(feature = "persist_filters_xml")]
        {
            let filters = self.loaded_settings.borrow().filters.clone();
            let data = WindowFilter::serialize_to_xml(&filters)
                .map_err(|e| format!("Failed to convert filters to legacy XML format:\n{e}"))?;

            Ok(data)
        }
        #[cfg(not(feature = "persist_filters_xml"))]
        {
            Err(
                "This program was compiled without support for legacy XML filters/rules. \
                    Recompile the program from source with the \"persist_filters_xml\" feature \
                    in order to support exporting such filter files."
                    .into(),
            )
        }
    }
    pub fn export_filters_to_json_string(&self) -> Result<String, Box<dyn Error>> {
        #[cfg(feature = "persist_filters")]
        {
            let exported = ExportedWindowFilters {
                filters: self.loaded_settings.borrow().filters.to_vec(),
                ..Default::default()
            };
            let data = serde_json::to_string_pretty(&exported)
                .map_err(|e| format!("Failed to convert filters to JSON:\n{e}"))?;
            Ok(data)
        }
        #[cfg(not(feature = "persist_filters"))]
        {
            Err(
                "This program was compiled without support for JSON filters/rules. \
                    Recompile the program from source with the \"persist_filters\" feature \
                    in order to support exporting such filter files."
                    .into(),
            )
        }
    }
    pub fn export_filters_to_file_path(
        &self,
        mut file_path: PathBuf,
    ) -> Result<(), Box<dyn Error>> {
        // The file dialog should have asked about overwriting existing file.
        let mut allow_overwrite = true;

        let is_legacy = if let Some(ext) = file_path.extension() {
            ext.eq_ignore_ascii_case("xml") || ext.eq_ignore_ascii_case("txt")
        } else {
            file_path.set_extension("json");
            allow_overwrite = false; // <- Since we change the path the dialog would not have warned about overwrite
            false
        };

        let data = if is_legacy {
            self.export_filters_to_xml_string()?
        } else {
            self.export_filters_to_json_string()?
        };

        let mut file = OpenOptions::new()
            .create(true)
            .create_new(!allow_overwrite)
            .write(true)
            .truncate(true)
            .open(file_path.as_path())
            .map_err(|e| format!("Failed to create file at \"{}\":\n{e}", file_path.display()))?;

        file.write_all(data.as_bytes()).map_err(|e| {
            format!(
                "Failed to write data to file at \"{}\":\n{e}",
                file_path.display()
            )
        })?;

        Ok(())
    }
    pub fn export_filters_with_dialog(&self) {
        let Some(selected_file_path) = rfd::FileDialog::new()
            .set_title("Export Virtual Desktop Manager Rules / Filters")
            .add_filter("JSON filters", &["json"])
            .add_filter("Xml legacy filters", &["xml", "txt"])
            .add_filter("Any filter file", &["json", "xml", "txt"])
            .add_filter("All files", &["*"])
            .set_parent(&WinsafeHandleToRawHandle(self.wnd.hwnd()))
            .set_file_name("filters.json")
            .save_file()
        else {
            return;
        };

        if let Err(e) = self.export_filters_to_file_path(selected_file_path) {
            rfd::MessageDialog::new()
                .set_title("Virtual Desktop Manager - Export error")
                .set_description(&e.to_string())
                .set_buttons(rfd::MessageButtons::Ok)
                .set_level(rfd::MessageLevel::Error)
                .set_parent(&WinsafeHandleToRawHandle(self.wnd.hwnd()))
                .show();
        }
    }
    pub fn import_filters_from_legacy_xml(&self, xml: String) -> Result<(), Box<dyn Error>> {
        #[cfg(feature = "persist_filters_xml")]
        {
            let imported = WindowFilter::deserialize_from_xml(&xml)
                .map_err(|e| format!("Failed to parse legacy XML filters/rules:\n{e}"))?;

            self.update_settings(|prev| UiSettings {
                filters: prev.filters.iter().cloned().chain(imported).collect(),
                ..prev.clone()
            });

            Ok(())
        }
        #[cfg(not(feature = "persist_filters_xml"))]
        {
            Err(
                "This program was compiled without support for legacy XML filters/rules. \
                    Recompile the program from source with the \"persist_filters_xml\" feature \
                    in order to support such filter files."
                    .into(),
            )
        }
    }
    pub fn import_filters_from_json(&self, json: String) -> Result<(), Box<dyn Error>> {
        #[cfg(feature = "persist_filters")]
        {
            let mut deserializer = serde_json::Deserializer::from_str(&json);
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
            let imported = result
                .map_err(|e| format!("Failed to parse JSON filters/rules:\n{e}"))?
                .migrate_and_get_filters();

            self.update_settings(|prev| UiSettings {
                filters: prev.filters.iter().cloned().chain(imported).collect(),
                ..prev.clone()
            });

            Ok(())
        }
        #[cfg(not(feature = "persist_filters"))]
        {
            Err(
                "This program was compiled without support for JSON filters/rules. \
                    Recompile the program from source with the \"persist_filters\" feature \
                    in order to support such filter files."
                    .into(),
            )
        }
    }
    pub fn import_filters_from_file_path(&self, file_path: PathBuf) -> Result<(), Box<dyn Error>> {
        let data = std::fs::read_to_string(file_path.as_path()).map_err(|e| {
            format!(
                "Error when reading file with filter/rule at \"{}\":\n\n{e}",
                file_path.display()
            )
        })?;

        let is_legacy = file_path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("xml") || ext.eq_ignore_ascii_case("txt"));

        if is_legacy {
            self.import_filters_from_legacy_xml(data)?;
        } else {
            self.import_filters_from_json(data)?;
        }

        Ok(())
    }
    pub fn import_filters_with_dialog(&self) {
        let Some(selected_file_path) = rfd::FileDialog::new()
            .set_title("Import Virtual Desktop Manager Rules / Filters")
            .add_filter("Any filter file", &["json", "xml", "txt"])
            .add_filter("JSON filters", &["json"])
            .add_filter("Xml legacy filters", &["xml", "txt"])
            .add_filter("All files", &["*"])
            .set_parent(&WinsafeHandleToRawHandle(self.wnd.hwnd()))
            .pick_file()
        else {
            return;
        };

        if let Err(e) = self.import_filters_from_file_path(selected_file_path) {
            rfd::MessageDialog::new()
                .set_title("Virtual Desktop Manager - Import error")
                .set_description(&e.to_string())
                .set_buttons(rfd::MessageButtons::Ok)
                .set_level(rfd::MessageLevel::Error)
                .set_parent(&WinsafeHandleToRawHandle(self.wnd.hwnd()))
                .show();
        }
    }
}
/// Sort list view.
impl WinsafeSettingsWindow {
    fn get_arrow_direction(&self, header: &gui::HeaderItem) -> gui::HeaderArrow {
        let sort_dir = header.lparam();
        if sort_dir == (co::HDF::SORTDOWN.raw() as isize) {
            gui::HeaderArrow::Desc
        } else if sort_dir == (co::HDF::SORTUP.raw() as isize) {
            gui::HeaderArrow::Asc
        } else {
            gui::HeaderArrow::None
        }
    }
    fn get_sort_info(&self) -> Option<(usize, gui::HeaderArrow)> {
        for (i, header) in self
            .data_view
            .header()
            .expect("No header")
            .items()
            .iter()
            .expect("can't iterate over headers")
            .enumerate()
        {
            let sort_dir = self.get_arrow_direction(&header);
            if sort_dir != gui::HeaderArrow::None {
                return Some((i, sort_dir));
            }
        }
        None
    }
    fn resort_items(&self) {
        tracing::trace!("WinsafeSettingsWindow::resort_items");
        let sort_info = self.get_sort_info();
        let result = self.data_view.items().sort(|a_item, b_item| {
            let a = self.data_view.items().get(a_item.index()).data();
            let a = a.borrow();
            let b = self.data_view.items().get(b_item.index()).data();
            let b = b.borrow();

            a.1.compare_kind(&b.1)
                // Compare column text:
                .then_with(|| {
                    let Some((column, direction)) = sort_info else {
                        return Ordering::Equal;
                    };
                    let a_text = a_item.text(column as u32);
                    let b_text = b_item.text(column as u32);
                    // Try to compare as numbers if possible:
                    let result = match a_text
                        .parse::<i64>()
                        .and_then(|a| Ok((a, b_text.parse::<i64>()?)))
                    {
                        Ok((a, b)) => a.cmp(&b),
                        Err(_) => a_text.cmp(&b_text),
                    };
                    if direction == gui::HeaderArrow::Asc {
                        result.reverse()
                    } else {
                        result
                    }
                })
                // Fallback to original indexes:
                .then_with(|| a.0.cmp(&b.0))
        });
        if let Err(e) = result {
            tracing::error!(error = ?e, "Failed to sort list view");
        }
    }
}
/// Manage [`WindowInfo`] shown in the settings window.
impl WinsafeSettingsWindow {
    fn clear_window_info(&self) {
        let mut count = self.data_view.items().count();
        let mut index = 0;
        while index < count {
            let item = self.data_view.items().get(index);
            index += 1;
            let has_window_data = item.data().borrow().1.is_window_info();
            if has_window_data {
                if let Err(e) = item.delete() {
                    tracing::error!(error =? e, "Failed to delete info about window from list view");
                } else {
                    count -= 1;
                    index -= 1;
                }
            }
        }
    }
    /// Finds filters/rules that apply to a specific window. Returns a string with one-based indexes
    /// of those filters (separated by commas).
    fn determine_active_filter_indexes_for_window(
        &self,
        window_index: i32,
        window: &WindowInfo,
    ) -> String {
        self.loaded_settings
            .borrow()
            .filters
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
        let index = self
            .data_view
            .items()
            .iter()
            .filter(|item| item.data().borrow().1.is_window_info())
            .count();

        let filter_indexes = self.determine_active_filter_indexes_for_window(index as i32, &window);
        let action = WindowFilter::find_first_action(
            &self.loaded_settings.borrow().filters,
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
        } = window.clone();

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
        match self
            .data_view
            .items()
            .add(&info, None, (index, DataViewItem::WindowInfo(window)))
        {
            Err(e) => tracing::error!(error =? e, "Failed to add window info to list view"),
            Ok(item) => {
                let index = item.index();

                let mut item_data = LVITEM::default();
                item_data.iGroupId = unsafe { co::LVI_GROUPID::from_raw(Self::GROUP_WINDOWS) };
                item_data.mask = co::LVIF::GROUPID;
                item_data.iItem = index as i32;
                let result = unsafe {
                    self.data_view
                        .hwnd()
                        .SendMessage(SetItem { lvitem: &item_data })
                };
                if let Err(e) = result {
                    tracing::error!(error =? e, "Failed to set group id for list view item");
                }

                self.is_data_sorted.set(false);
            }
        }
    }
    fn gather_window_info(&self) {
        tracing::trace!("WinsafeSettingsWindow::gather_window_info");
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
        let wnd = self.wnd.clone();
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
                            _ = unsafe {
                                wnd.hwnd().PostMessage(WndMsg {
                                    msg_id: custom_msg::WINDOW_INFO_AVAILABLE,
                                    wparam: 0,
                                    lparam: 0,
                                })
                            };
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
                    _ = unsafe {
                        wnd.hwnd().PostMessage(WndMsg {
                            msg_id: custom_msg::WINDOW_INFO_AVAILABLE,
                            wparam: 0,
                            lparam: 0,
                        })
                    };
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
    /// Re-check which filters apply to loaded windows.
    fn update_window_infos(&self) {
        let filters = self.loaded_settings.borrow().filters.clone();
        for item in self.data_view.items().iter() {
            let data = item.data();
            let data = data.borrow();
            let (index, DataViewItem::WindowInfo(window_info)) = &*data else {
                continue;
            };

            let filter_indexes =
                self.determine_active_filter_indexes_for_window(*index as i32, &window_info);
            if let Err(e) = item.set_text(Self::COLUMN_FILTERS_INDEX, filter_indexes.as_str()) {
                tracing::error!(error =? e, "Failed to update shown filter indexes");
            }

            let action = WindowFilter::find_first_action(&filters, *index as i32, &window_info)
                .map(|filter| filter.display_target_desktop().to_string());
            if let Err(e) = item.set_text(
                Self::COLUMN_TARGET_DESKTOP,
                action.unwrap_or_default().as_str(),
            ) {
                tracing::error!(error =? e, "Failed to update shown filter indexes");
            }
        }
    }
}
/// Manage rules/filters.
impl WinsafeSettingsWindow {
    fn update_settings(&self, f: impl FnOnce(&UiSettings) -> UiSettings) {
        let prev = self
            .shared
            .mutex
            .lock()
            .unwrap()
            .tracked_settings
            .new
            .clone();
        let new = Arc::new(f(&prev));
        {
            let mut guard = self.shared.mutex.lock().unwrap();
            guard.tracked_settings.new = new.clone();
            SharedStateMut::notify_main_of_settings_change(guard);
        }
        self.reload_from_settings(&new)
    }
    fn reload_from_settings(&self, new: &Arc<UiSettings>) {
        let prev = self.loaded_settings.replace(new.clone());
        if !Arc::ptr_eq(&prev.filters, &new.filters) {
            self.populate_filter_list(&new.filters)
        }
        self.program_settings_panel.set_settings_data(new);
    }
    fn populate_filter_list(&self, filters: &Arc<[WindowFilter]>) {
        // Update existing filter items:
        let existing_filter_rows: Vec<(_, WindowFilter)> = self
            .data_view
            .items()
            .iter()
            .filter_map(|item| {
                if let DataViewItem::Filter(filter) = &item.data().borrow().1 {
                    Some((item, filter.clone()))
                } else {
                    None
                }
            })
            .collect();

        tracing::trace!(
            old_filters_count = existing_filter_rows.len(),
            new_filters_count = filters.len(),
            "WinsafeSettingsWindow::populate_filter_list"
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
                window_index.into_one_based_indexes().to_string(),
                desktop_index.into_one_based_indexes().to_string(),
                window_title.display_escaped_newline_glob().to_string(),
                process_name.display_escaped_newline_glob().to_string(),
                filter_index.saturating_add(1).to_string(),
                filter.display_target_desktop().to_string(),
            ]
        }

        let mut did_change = false;
        let mut did_delete = false;

        for index in 0.. {
            let existing_row = existing_filter_rows.get(index);
            let new = filters.get(index);
            match (existing_row, new) {
                (Some((existing_row, prev)), Some(new)) => {
                    debug_assert_eq!(
                        existing_row.data().borrow().0,
                        index,
                        "stored filter index should always equal row index"
                    );
                    if prev != new {
                        did_change = true;
                        let info = get_filter_columns(index, new);
                        for (column_ix, text) in info.into_iter().enumerate() {
                            if let Err(e) = existing_row.set_text(column_ix as u32, text.as_str()) {
                                tracing::error!(error = ?e, column_index = column_ix, "failed to update column text for filter");
                            }
                        }
                        existing_row.data().borrow_mut().1 = DataViewItem::Filter(new.clone());
                    }
                }
                (Some((_, _)), None) => {
                    did_delete = true;
                    // Delete last row first (workaround iterator invalidation):
                    for (existing_row, _) in existing_filter_rows[index..].iter().rev() {
                        if let Err(e) = existing_row.delete() {
                            tracing::error!(row_index = index, error = ?e, "Failed to delete filter row");
                        }
                    }
                    break;
                }
                (None, Some(new)) => {
                    // No existing row so create one
                    did_change = true;
                    let info = get_filter_columns(index, new);
                    match self.data_view.items().add(
                        &info,
                        None,
                        (index, DataViewItem::Filter(new.clone())),
                    ) {
                        Err(e) => {
                            tracing::error!(row_index = index, error = ?e, "Failed to add filter item");
                        }
                        Ok(item) => {
                            let index = item.index();

                            let mut item_data = LVITEM::default();
                            item_data.iGroupId =
                                unsafe { co::LVI_GROUPID::from_raw(Self::GROUP_FILTERS) };
                            item_data.mask = co::LVIF::GROUPID;
                            item_data.iItem = index as i32;
                            let result = unsafe {
                                self.data_view
                                    .hwnd()
                                    .SendMessage(SetItem { lvitem: &item_data })
                            };
                            if let Err(e) = result {
                                tracing::error!(error =? e, "Failed to set group id for list view item");
                            }
                        }
                    }
                }
                (None, None) => break,
            }
        }

        if did_change {
            self.is_data_sorted.set(false);
        }
        if did_delete || did_change {
            // Windows might now be affected by different filters:
            self.update_window_infos();
        }
        if did_change {
            self.resort_items();
        }

        // Update sidebar with config for selected filter:
        let selected_filter = if let Some(prev_selected) =
            self.filter_options_panel.get_selected_filter_index()
        {
            if prev_selected >= filters.len() {
                // Prev selection was removed, select last remaining:
                Some(filters.len().saturating_sub(1))
            } else if !existing_filter_rows.is_empty() && filters.len() > existing_filter_rows.len()
            {
                // New items were added to an existing list, select newest item:
                Some(filters.len().saturating_sub(1))
            } else {
                Some(prev_selected)
            }
        } else {
            None
        };
        self.set_selected_filter(selected_filter);
    }

    pub fn set_selected_filter(&self, mut selected_filter: Option<usize>) {
        let settings = self.loaded_settings.borrow().clone();
        if matches!(selected_filter, Some(index) if index >= settings.filters.len()) {
            selected_filter = settings.filters.len().checked_sub(1);
        }
        self.filter_options_panel
            .set_filters_len(settings.filters.len());
        self.filter_options_panel
            .set_selected_filter_index(selected_filter);
        let filter_data;
        let filter_data =
            if let Some(data) = selected_filter.and_then(|index| settings.filters.get(index)) {
                data
            } else {
                filter_data = WindowFilter::default();
                &filter_data
            };
        self.filter_options_panel.set_filter_data(filter_data);

        self.highlight_selected_filter_in_list();
    }

    pub fn highlight_selected_filter_in_list(&self) {
        let selected_filter_index = self.filter_options_panel.get_selected_filter_index();
        for item in self.data_view.items().iter() {
            let selected = {
                let data = item.data();
                let guard = data.borrow();
                if !guard.1.is_filter() {
                    continue;
                }

                selected_filter_index == Some(guard.0)
            };
            if let Err(e) = item.select(selected) {
                tracing::error!(error = ?e, "Failed to select filter row");
            }
        }
    }
}
impl FilterOptionsHooks for Weak<WinsafeSettingsWindow> {
    #[tracing::instrument(level = "trace", skip(self))]
    fn on_option_change(&self, _change: filter_options::FilterChange) {
        let Some(this) = self.upgrade() else { return };
        let Some(index) = this.filter_options_panel.get_selected_filter_index() else {
            return;
        };
        let filter_data = this.filter_options_panel.get_filter_data();
        tracing::trace!(filter_data = ?filter_data, filter_index = index, "New filter options");

        this.update_settings(|prev| {
            let mut filters = prev.filters.clone();
            if let Some(filter) = Arc::make_mut(&mut filters).get_mut(index) {
                *filter = filter_data;
            }
            UiSettings {
                filters,
                ..prev.clone()
            }
        });
    }
    #[tracing::instrument(level = "trace", skip(self))]
    fn on_index_change(&self) {
        let Some(this) = self.upgrade() else { return };
        let mut filter_index = this.filter_options_panel.get_selected_filter_index();
        tracing::trace!(filter_index = filter_index, "New filter selected");

        let settings = this.loaded_settings.borrow().clone();
        if matches!(filter_index, Some(filter_index) if filter_index >= settings.filters.len()) {
            filter_index = Some(settings.filters.len().saturating_sub(1));
            tracing::debug!(new_index = ?filter_index, "Selected filter index was too high, lowered it to the max value");
        }

        this.set_selected_filter(filter_index);
    }
    #[tracing::instrument(level = "trace", ret, skip(self))]
    fn on_move_up(&self) {
        let Some(this) = self.upgrade() else { return };
        let Some(index) = this.filter_options_panel.get_selected_filter_index() else {
            return;
        };
        let Some(swap_with) = index.checked_sub(1) else {
            return;
        };
        this.update_settings(|prev| {
            let mut filters = prev.filters.clone();
            Arc::make_mut(&mut filters).swap(index, swap_with);
            UiSettings {
                filters,
                ..prev.clone()
            }
        });
        this.set_selected_filter(Some(swap_with));
    }
    #[tracing::instrument(level = "trace", ret, skip(self))]
    fn on_move_down(&self) {
        let Some(this) = self.upgrade() else { return };
        let Some(index) = this.filter_options_panel.get_selected_filter_index() else {
            return;
        };
        let swap_with = index + 1;
        if swap_with >= this.loaded_settings.borrow().filters.len() {
            return;
        }
        this.update_settings(|prev| {
            let mut filters = prev.filters.clone();
            Arc::make_mut(&mut filters).swap(index, swap_with);
            UiSettings {
                filters,
                ..prev.clone()
            }
        });
        this.set_selected_filter(Some(swap_with));
    }
    #[tracing::instrument(level = "trace", ret, skip(self))]
    fn on_create_new(&self) {
        let Some(this) = self.upgrade() else { return };
        this.update_settings(|prev| UiSettings {
            filters: prev
                .filters
                .iter()
                .cloned()
                .chain(Some(WindowFilter::default()))
                .collect(),
            ..prev.clone()
        });
        let settings = this.loaded_settings.borrow().clone();
        this.set_selected_filter(Some(settings.filters.len().saturating_sub(1)));
    }
    #[tracing::instrument(level = "trace", ret, skip(self))]
    fn on_delete(&self) {
        let Some(this) = self.upgrade() else { return };
        let Some(index) = this.filter_options_panel.get_selected_filter_index() else {
            return;
        };
        this.update_settings(|prev| UiSettings {
            filters: prev
                .filters
                .iter()
                .enumerate()
                .filter(|&(ix, _)| ix != index)
                .map(|(_, filter)| filter.clone())
                .collect(),
            ..prev.clone()
        });
        this.set_selected_filter(Some(index));
    }
}
impl ProgramSettingsHooks for Weak<WinsafeSettingsWindow> {
    #[tracing::instrument(level = "trace", ret, skip(self))]
    fn on_setting_change(&self) {
        let Some(this) = self.upgrade() else { return };
        let mut quick_switch_menu_shortcuts_error = false;
        let settings_data = this
            .program_settings_panel
            .get_settings_data(&mut quick_switch_menu_shortcuts_error);
        this.update_settings(|prev| UiSettings {
            filters: prev.filters.clone(),
            config_window: prev.config_window.clone(),
            quick_switch_menu_shortcuts: if quick_switch_menu_shortcuts_error {
                tracing::warn!("UI data for \"quick_switch_menu_shortcuts\" had errors so resetting to last known good state.");
                prev.quick_switch_menu_shortcuts.clone()
            } else {
                settings_data.quick_switch_menu_shortcuts.clone()
            },
            ..settings_data
        });
    }
}
impl Drop for WinsafeSettingsWindow {
    fn drop(&mut self) {
        let mut guard = self.shared.mutex.lock().unwrap();
        guard.state = WindowState::Closed;
        guard.window = None;
    }
}
