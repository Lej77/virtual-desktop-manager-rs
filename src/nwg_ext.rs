//! Extends the `nwg` crate with additional features.
#![allow(dead_code)] // We consider this more of an external library.

mod number_select;

use std::{
    any::Any,
    borrow::Cow,
    cell::{Cell, RefCell},
    cmp::Ordering,
    collections::BTreeMap,
    mem,
    ops::ControlFlow,
    ptr::null_mut,
    sync::{
        atomic::AtomicBool,
        mpsc::{self, RecvTimeoutError},
        Arc, OnceLock,
    },
    time::{Duration, Instant},
};

use nwg::ControlHandle;
use windows::Win32::Foundation::{HWND, RECT};

pub use number_select::{NumberSelect2, NumberSelectBuilder};

/// Copied from [`native_windows_gui::win32::base_helper::to_utf16`].
pub fn to_utf16(s: &str) -> Vec<u16> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    OsStr::new(s)
        .encode_wide()
        .chain(core::iter::once(0u16))
        .collect()
}
/// Decode a raw utf16 string. Should be null terminated.
///
/// Adapted from [`native_windows_gui::win32::base_helper::from_utf16`].
pub fn from_utf16(s: &[u16]) -> String {
    use std::os::windows::ffi::OsStringExt;

    let null_index = s.iter().position(|&i| i == 0).unwrap_or(s.len());
    let os_string = std::ffi::OsString::from_wide(&s[0..null_index]);

    os_string
        .into_string()
        .unwrap_or("Decoding error".to_string())
}

/// Utility for catching panics and resuming them. Useful to implement unsafe
/// callbacks.
pub struct PanicCatcher {
    caught: Option<Box<dyn Any + Send + 'static>>,
}
impl PanicCatcher {
    pub const fn new() -> Self {
        Self { caught: None }
    }
    fn drop_without_unwind(value: Box<dyn Any + Send + 'static>) {
        struct SafeDrop(Option<Box<dyn Any + Send + 'static>>);
        impl Drop for SafeDrop {
            fn drop(&mut self) {
                while let Some(value) = self.0.take() {
                    if let Err(payload) =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(value)))
                    {
                        self.0 = Some(payload);
                    }
                }
            }
        }
        drop(SafeDrop(Some(value)));
    }
    pub fn has_caught_panic(&self) -> bool {
        self.caught.is_some()
    }
    /// Catch panics that occur in the provided callback.
    pub fn catch<R, F: FnOnce() -> R>(&mut self, f: F) -> Option<R> {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
            Ok(res) => Some(res),
            Err(e) => {
                if let Some(old) = std::mem::replace(&mut self.caught, Some(e)) {
                    Self::drop_without_unwind(old);
                }
                None
            }
        }
    }
    /// Resume any panics that occurred in the callback.
    pub fn resume_panic(&mut self) {
        if let Some(e) = self.caught.take() {
            std::panic::resume_unwind(e);
        }
    }
}
impl Default for PanicCatcher {
    fn default() -> Self {
        Self::new()
    }
}

/// Get handles to all open windows or to child windows of a specific window.
///
/// # References
///
/// - Rust library for getting titles of all open windows:
///   <https://github.com/HiruNya/window_titles/blob/924feffac93c9ac7238d6fa5c39c1453815a0408/src/winapi.rs>
/// - [Getting a list of all open windows in c++ and storing them - Stack
///   Overflow](https://stackoverflow.com/questions/42589496/getting-a-list-of-all-open-windows-in-c-and-storing-them)
/// - [java - Windows: how to get a list of all visible windows? - Stack
///   Overflow](https://stackoverflow.com/questions/3188484/windows-how-to-get-a-list-of-all-visible-windows)
/// - [EnumChildWindows function (winuser.h) - Win32 apps | Microsoft
///   Learn](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-enumchildwindows)
pub fn enum_child_windows<F: FnMut(HWND) -> ControlFlow<()>>(parent: Option<HWND>, f: F) {
    use windows::Win32::{
        Foundation::{BOOL, HWND, LPARAM},
        UI::WindowsAndMessaging::EnumChildWindows,
    };

    struct State<F> {
        f: F,
        catcher: PanicCatcher,
    }

    unsafe extern "system" fn enumerate_windows<F: FnMut(HWND) -> ControlFlow<()>>(
        window: HWND,
        state: LPARAM,
    ) -> BOOL {
        let state = state.0 as *mut State<F>;
        let state: &mut State<F> = unsafe { &mut *state };
        let result = state
            .catcher
            .catch(|| (state.f)(window))
            .unwrap_or(ControlFlow::Break(()));
        BOOL::from(result.is_continue())
    }

    let mut state = State {
        f,
        catcher: PanicCatcher::new(),
    };

    unsafe {
        let _ = EnumChildWindows(
            parent.unwrap_or(HWND(null_mut())),
            Some(enumerate_windows::<F>),
            LPARAM(&mut state as *mut State<F> as isize),
        );
    }

    state.catcher.resume_panic();
}

/// Return the index of a menu item in a parent menu.
///
/// Adapted from [`native_windows_gui::win32::menu::menu_index_in_parent`] (in
/// the private `win32` module).
pub fn menu_item_index_in_parent(item_handle: ControlHandle) -> Option<u32> {
    if item_handle.blank() {
        return None;
    }
    let (parent, id) = item_handle.hmenu_item()?;

    use windows::Win32::UI::WindowsAndMessaging::{GetMenuItemCount, GetMenuItemID, HMENU};

    let parent = HMENU(parent.cast());
    let children_count = unsafe { GetMenuItemCount(parent) };

    for i in 0..children_count {
        let item_id = unsafe { GetMenuItemID(parent, i) };
        if item_id == (-1_i32 as u32) {
            continue;
        } else if item_id == id {
            return Some(i as u32);
        }
    }

    None
}

/**
    Return the index of a children menu/menuitem in a parent menu.

    Adapted from [`native_windows_gui::win32::menu::menu_index_in_parent`] (in the private `win32` module).
*/
pub fn menu_index_in_parent(menu_handle: ControlHandle) -> Option<u32> {
    if menu_handle.blank() {
        return None;
    }
    let (parent, menu) = menu_handle.hmenu()?;

    // Safety: we check the same preconditions as the nwg crate does when it
    // calls this function on a menu.
    use windows::Win32::UI::WindowsAndMessaging::{GetMenuItemCount, GetSubMenu, HMENU};

    let parent = HMENU(parent.cast());
    let children_count = unsafe { GetMenuItemCount(parent) };
    let mut sub_menu;

    for i in 0..children_count {
        sub_menu = unsafe { GetSubMenu(parent, i) };
        if sub_menu.0 == null_mut() {
            continue;
        } else if sub_menu.0 == (menu.cast()) {
            return Some(i as u32);
        }
    }

    None
}

/// Update the text of a submenu or menu item.
pub fn menu_set_text(handle: ControlHandle, text: &str) {
    if handle.blank() {
        panic!("Unbound handle");
    }
    enum MenuItemInfo {
        Position(u32),
        Id(u32),
    }
    let (parent, item_info) = match handle {
        ControlHandle::Menu(parent, _) => {
            // Safety: the handles inside ControlHandle is valid, according to
            // https://gabdube.github.io/native-windows-gui/native-windows-docs/extern_wrapping.html
            // constructing new ControlHandle instances should be considered
            // unsafe.
            if let Some(index) = menu_index_in_parent(handle) {
                (parent, MenuItemInfo::Position(index))
            } else {
                return;
            }
        }
        ControlHandle::MenuItem(parent, id) => (parent, MenuItemInfo::Id(id)),
        _ => return,
    };

    use windows::{
        core::PWSTR,
        Win32::UI::WindowsAndMessaging::{SetMenuItemInfoW, HMENU, MENUITEMINFOW, MIIM_STRING},
    };

    // The code below was inspired by `nwg::win32::menu::enable_menuitem`
    // and: https://stackoverflow.com/questions/25139819/change-text-of-an-menu-item

    let use_position = matches!(item_info, MenuItemInfo::Position(_));
    let value = match item_info {
        MenuItemInfo::Position(p) => p,
        MenuItemInfo::Id(id) => id,
    };

    let text = to_utf16(text);

    let mut info = MENUITEMINFOW::default();
    info.cbSize = core::mem::size_of_val(&info) as u32;
    info.fMask = MIIM_STRING;
    info.dwTypeData = PWSTR(text.as_ptr().cast_mut());

    let _ = unsafe { SetMenuItemInfoW(HMENU(parent as _), value, use_position, &info) };
}

/// Remove a submenu from its parent. Note that this is not done automatically
/// when a menu is dropped.
pub fn menu_remove(menu: &nwg::Menu) {
    if menu.handle.blank() {
        return;
    }
    let Some((parent, _)) = menu.handle.hmenu() else {
        return;
    };

    let Some(index) = menu_index_in_parent(menu.handle) else {
        return;
    };

    use windows::Win32::UI::WindowsAndMessaging::{RemoveMenu, HMENU, MF_BYPOSITION};

    let _ = unsafe { RemoveMenu(HMENU(parent.cast()), index, MF_BYPOSITION) };
}

/// Finds the current context menu window using an undocumented trick.
///
/// Note that you can send the undocumented message `0x1e5` to the window in
/// order to select an item, specify the item index as the `wparam`. (Leave
/// `lparam` as 0.) Then you can activate that item (to for example open a
/// submenu) by sending a [`WM_KEYDOWN`] message with the [`VK_RETURN`] key.
///
/// [`WM_KEYDOWN`]: windows::Win32::UI::WindowsAndMessaging::WM_KEYDOWN
/// [`VK_RETURN`]: windows::Win32::UI::Input::KeyboardAndMouse::VK_RETURN
///
/// # References
///
/// - <https://microsoft.public.win32.programmer.ui.narkive.com/jQJBmxzp/open-submenu-programmatically#post6>
///    - Which links to:
///      <http://www.codeproject.com/menu/skinmenu.asp?df=100&forumid=14636&exp=0&select=2219867>
pub fn find_context_menu_window() -> Option<HWND> {
    use windows::{
        core::PCWSTR,
        Win32::{Foundation::HWND, UI::WindowsAndMessaging::FindWindowW},
    };

    static CONTEXT_MENU_CLASS_NAME: OnceLock<Vec<u16>> = OnceLock::new();
    let class_name = CONTEXT_MENU_CLASS_NAME.get_or_init(|| {
        let mut t = to_utf16("#32768");
        t.shrink_to_fit();
        t
    });

    let window = unsafe { FindWindowW(PCWSTR::from_raw(class_name.as_ptr()), None) }.ok()?;
    if window == HWND::default() {
        None
    } else {
        Some(window)
    }
}

/// Check if a window is valid (not destroyed). A closed window might still be
/// valid.
///
/// Adapted from [`native_windows_gui::win32::base_helper::check_hwnd`] used by
/// many methods of [`nwg::Window`].
pub fn window_is_valid(handle: nwg::ControlHandle) -> bool {
    if handle.blank() {
        return false;
    }
    let Some(hwnd) = handle.hwnd() else {
        return false;
    };
    unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindow(HWND(hwnd.cast())) }.as_bool()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WindowPlacement {
    Normal,
    Maximized,
    Minimized,
}

/// Retrieves the show state and the restored, minimized, and maximized positions of the specified window.
///
/// # References
///
/// - <https://learn.microsoft.com/sv-se/windows/win32/api/winuser/nf-winuser-getwindowplacement?redirectedfrom=MSDN>
/// - <https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-windowplacement>
pub fn window_placement(window: &nwg::Window) -> windows::core::Result<WindowPlacement> {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowPlacement, SW_SHOWMAXIMIZED, SW_SHOWMINIMIZED, SW_SHOWNORMAL, WINDOWPLACEMENT,
    };

    let handle = window.handle.hwnd().expect("Not a window handle");

    let mut info = WINDOWPLACEMENT {
        length: core::mem::size_of::<WINDOWPLACEMENT>() as u32,
        ..WINDOWPLACEMENT::default()
    };

    unsafe { GetWindowPlacement(HWND(handle.cast()), &mut info) }.inspect_err(|e| {
        tracing::error!(error = e.to_string(), "GetWindowPlacement failed");
    })?;

    Ok(if info.showCmd == SW_SHOWMAXIMIZED.0 as u32 {
        WindowPlacement::Maximized
    } else if info.showCmd == SW_SHOWMINIMIZED.0 as u32 {
        WindowPlacement::Minimized
    } else if info.showCmd == SW_SHOWNORMAL.0 as u32 {
        WindowPlacement::Normal
    } else {
        tracing::error!(
            showCmd = info.showCmd,
            "Invalid return value from GetWindowPlacement"
        );
        WindowPlacement::Normal
    })
}

/// Set a tray to use version 4. Shell_NotifyIcon mouse and keyboard events are
/// handled differently than in earlier versions of Windows.
///
/// # References
///
/// - <https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shell_notifyiconw#remarks>
/// - <https://stackoverflow.com/questions/41649303/difference-between-notifyicon-version-and-notifyicon-version-4-used-in-notifyico>
/// - Note that the NIN_KEYSELECT event will be sent twice for the enter key:
///   <https://github.com/openjdk/jdk/blob/master/src/java.desktop/windows/native/libawt/windows/awt_TrayIcon.cpp#L449>
pub fn tray_set_version_4(tray: &nwg::TrayNotification) {
    use windows::Win32::UI::Shell::{
        Shell_NotifyIconW, NIM_SETVERSION, NOTIFYICONDATAW, NOTIFYICON_VERSION_4,
    };

    const NOT_BOUND: &str = "TrayNotification is not yet bound to a winapi object";
    const BAD_HANDLE: &str = "INTERNAL ERROR: TrayNotification handle is not HWND!";

    if tray.handle.blank() {
        panic!("{}", NOT_BOUND);
    }

    let parent = tray.handle.tray().expect(BAD_HANDLE);
    let mut data = NOTIFYICONDATAW {
        hWnd: HWND(parent.cast()),
        ..Default::default()
    };
    data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
    data.cbSize = mem::size_of_val(&data) as u32;

    let success = unsafe { Shell_NotifyIconW(NIM_SETVERSION, &data) };
    if !success.as_bool() {
        tracing::error!("Failed to set tray version to 4");
    }
}

#[inline]
pub fn tray_get_rect(tray: &nwg::TrayNotification) -> windows::core::Result<RECT> {
    use windows::Win32::UI::Shell::{Shell_NotifyIconGetRect, NOTIFYICONIDENTIFIER};

    const NOT_BOUND: &str = "TrayNotification is not yet bound to a winapi object";
    const BAD_HANDLE: &str = "INTERNAL ERROR: TrayNotification handle is not HWND!";

    if tray.handle.blank() {
        panic!("{}", NOT_BOUND);
    }
    let parent = tray.handle.tray().expect(BAD_HANDLE);

    let nid = NOTIFYICONIDENTIFIER {
        hWnd: HWND(parent.cast()),
        cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as _,
        ..NOTIFYICONIDENTIFIER::default()
    };

    unsafe { Shell_NotifyIconGetRect(&nid) }
}

/// Sort the items in a list view. The callback is given the current indexes of
/// list items that should be compared.
///
/// # [Note](https://learn.microsoft.com/en-us/windows/win32/controls/lvm-sortitemsex#remarks)
///
/// During the sorting process, the list-view contents are unstable. If the
/// callback function sends any messages to the list-view control aside from
/// LVM_GETITEM (ListView_GetItem), the results are unpredictable.
///
/// That message corresponds to the [`nwg::ListView::item`] function.
///
/// # References
///
/// - <https://learn.microsoft.com/en-us/windows/win32/controls/lvm-sortitemsex>
pub fn list_view_sort_rows<F>(list_view: &nwg::ListView, f: F)
where
    F: FnMut(usize, usize) -> Ordering,
{
    use windows::Win32::{
        Foundation::{LPARAM, LRESULT, WPARAM},
        UI::{
            Controls::LVM_SORTITEMSEX,
            WindowsAndMessaging::{IsWindow, SendMessageW},
        },
    };

    const NOT_BOUND: &str = "ListView is not yet bound to a winapi object";
    const BAD_HANDLE: &str = "INTERNAL ERROR: ListView handle is not HWND!";

    /// Adapted from [`native_windows_gui::win32::base_helper::check_hwnd`].
    fn check_hwnd(handle: &ControlHandle, not_bound: &str, bad_handle: &str) -> HWND {
        if handle.blank() {
            panic!("{}", not_bound);
        }
        match handle.hwnd() {
            Some(hwnd) => {
                if unsafe { IsWindow(HWND(hwnd.cast())) }.as_bool() {
                    HWND(hwnd.cast())
                } else {
                    panic!("The window handle is no longer valid. This usually means the control was freed by the OS");
                }
            }
            None => {
                panic!("{}", bad_handle);
            }
        }
    }
    let handle = check_hwnd(&list_view.handle, NOT_BOUND, BAD_HANDLE);

    struct State<F> {
        f: F,
        catcher: PanicCatcher,
    }

    unsafe extern "system" fn compare_func<F: FnMut(usize, usize) -> Ordering>(
        index1: LPARAM,
        index2: LPARAM,
        state: LPARAM,
    ) -> LRESULT {
        let state = state.0 as *mut State<F>;
        let state: &mut State<F> = unsafe { &mut *state };
        if state.catcher.has_caught_panic() {
            return LRESULT(0);
        }
        let result = state
            .catcher
            .catch(|| (state.f)(index1.0 as usize, index2.0 as usize))
            .unwrap_or(Ordering::Equal);
        LRESULT(match result {
            Ordering::Less => -1,
            Ordering::Equal => 0,
            Ordering::Greater => 1,
        })
    }

    let mut state = State {
        f,
        catcher: PanicCatcher::new(),
    };

    unsafe {
        SendMessageW(
            handle,
            LVM_SORTITEMSEX,
            WPARAM(&mut state as *mut State<F> as usize),
            LPARAM(compare_func::<F> as usize as isize),
        )
    };

    state.catcher.resume_panic();
}

/// Enables or disables whether the items in a list-view control display as a
/// group.
///
/// # References
///
/// - [ListView_EnableGroupView macro (commctrl.h) - Win32 apps | Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/api/commctrl/nf-commctrl-listview_enablegroupview)
/// - [LVM_ENABLEGROUPVIEW message (Commctrl.h) - Win32 apps | Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/controls/lvm-enablegroupview)
pub fn list_view_enable_groups(list_view: &nwg::ListView, enable: bool) {
    if !window_is_valid(list_view.handle) {
        tracing::error!("Tried to toggle groups for invalid list view");
        return;
    }
    let Some(handle) = list_view.handle.hwnd() else {
        tracing::error!("Tried to toggle groups for invalid list view");
        return;
    };
    let result = unsafe {
        windows::Win32::UI::WindowsAndMessaging::SendMessageW(
            HWND(handle.cast()),
            windows::Win32::UI::Controls::LVM_ENABLEGROUPVIEW,
            windows::Win32::Foundation::WPARAM(enable as usize),
            windows::Win32::Foundation::LPARAM(0),
        )
    };
    match result.0 {
        0 => tracing::trace!(
            "Groups in list view was already {}",
            if enable { "enabled" } else { "disabled" }
        ),
        1 => tracing::trace!(
            "Groups in list view was successfully {}",
            if enable { "enabled" } else { "disabled" }
        ),
        -1 => tracing::error!("Failed to enable/disable groups in list view"),
        _ => {
            tracing::error!(result =? result.0, "Unexpected return value when toggling groups in list view")
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ListViewGroupAlignment {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ListViewGroupInfo<'a> {
    pub group_id: i32,
    /// Determines if a new group is created or if an existing group should be
    /// updated.
    pub create_new: bool,
    pub header: Option<Cow<'a, str>>,
    /// This element is drawn under the header text.
    pub subtitle: Option<Cow<'a, str>>,
    pub footer: Option<Cow<'a, str>>,
    /// This item is drawn right-aligned opposite the header text. When clicked
    /// by the user, the task link generates an `LVN_LINKCLICK` notification.
    pub task: Option<Cow<'a, str>>,
    /// This item is drawn opposite the title image when there is a title image,
    /// no extended image, and header is centered aligned.
    pub description_top: Option<Cow<'a, str>>,
    /// This item is drawn under the top description text when there is a title
    /// image, no extended image, and header is center aligned.
    pub description_bottom: Option<Cow<'a, str>>,
    pub header_alignment: Option<ListViewGroupAlignment>,
    /// If this is specified then the `header_alignment` should also be specified.
    pub footer_alignment: Option<ListViewGroupAlignment>,
    /// If the group is collapsed/expanded.
    pub collapsed: Option<bool>,
    /// The group is hidden.
    pub hidden: Option<bool>,
    /// The group does not display a header.
    pub no_header: Option<bool>,
    /// The group can be collapsed.
    pub collapsible: Option<bool>,
    /// The group has keyboard focus.
    pub focused: Option<bool>,
    /// The group is selected.
    pub selected: Option<bool>,
    /// The group displays only a portion of its items.
    pub subseted: Option<bool>,
    /// The subset link of the group has keyboard focus.
    pub subset_link_focused: Option<bool>,
}

/// Create or update a group inside a list view.
///
/// Adapted from [`native_windows_gui::ListView::update_item`].
///
/// # References
///
/// - [How to Use Groups in a List-View - Win32 apps | Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/controls/use-groups-in-a-list-view)
/// - [LVM_SETGROUPINFO message (Commctrl.h) - Win32 apps | Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/controls/lvm-setgroupinfo)
/// - [LVM_INSERTGROUP message (Commctrl.h) - Win32 apps | Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/controls/lvm-insertgroup)
/// - [LVGROUP (commctrl.h) - Win32 apps | Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/api/commctrl/ns-commctrl-lvgroup)
/// - [Discussion/Proposal: more ListViewGroup functionality · Issue #2623 · dotnet/winforms · GitHub](https://github.com/dotnet/winforms/issues/2623)
///    - This also links to some source code that sends the above messages.
/// - [Grouping in List View Controls - CodeProject](https://www.codeproject.com/Questions/415005/Grouping-in-List-View-Controls)
pub fn list_view_set_group_info(list_view: &nwg::ListView, info: ListViewGroupInfo) {
    use windows::{
        core::PWSTR,
        Win32::{
            Foundation::{LPARAM, WPARAM},
            UI::{
                Controls::{
                    LVGA_FOOTER_CENTER, LVGA_FOOTER_LEFT, LVGA_FOOTER_RIGHT, LVGA_HEADER_CENTER,
                    LVGA_HEADER_LEFT, LVGA_HEADER_RIGHT, LVGF_ALIGN, LVGF_DESCRIPTIONBOTTOM,
                    LVGF_DESCRIPTIONTOP, LVGF_FOOTER, LVGF_GROUPID, LVGF_HEADER, LVGF_NONE,
                    LVGF_STATE, LVGF_SUBTITLE, LVGF_TASK, LVGROUP, LVGS_COLLAPSED,
                    LVGS_COLLAPSIBLE, LVGS_FOCUSED, LVGS_HIDDEN, LVGS_NOHEADER, LVGS_SELECTED,
                    LVGS_SUBSETED, LVGS_SUBSETLINKFOCUSED, LVM_INSERTGROUP, LVM_SETGROUPINFO,
                },
                WindowsAndMessaging::SendMessageW,
            },
        },
    };

    if !window_is_valid(list_view.handle) {
        tracing::error!("Tried to create/update group for invalid list view");
        return;
    }
    let Some(handle) = list_view.handle.hwnd() else {
        tracing::error!("Tried to create/update group for invalid list view");
        return;
    };
    let mut item = LVGROUP {
        cbSize: core::mem::size_of::<LVGROUP>() as u32,
        mask: if info.create_new {
            LVGF_GROUPID
        } else {
            LVGF_NONE
        },
        iGroupId: info.group_id,
        ..Default::default()
    };

    /// # Safety
    ///
    /// Can't be used inside a nested block or the generated buffer will free the
    /// string early.
    macro_rules! set_str {
        (Options {
            mask: $mask:expr,
            input: $input:expr,
            str_field: $str:ident,
        }) => {
            let mut __temp_buffer;
            if let Some(text_utf8) = $input {
                item.mask |= $mask;
                __temp_buffer = to_utf16(text_utf8);
                item.$str = PWSTR::from_raw(__temp_buffer.as_mut_ptr());
            }
        };
    }

    set_str!(Options {
        mask: LVGF_HEADER,
        input: &info.header,
        str_field: pszHeader,
    });
    set_str!(Options {
        mask: LVGF_FOOTER,
        input: &info.footer,
        str_field: pszFooter,
    });
    set_str!(Options {
        mask: LVGF_SUBTITLE,
        input: &info.subtitle,
        str_field: pszSubtitle,
    });
    set_str!(Options {
        mask: LVGF_TASK,
        input: &info.task,
        str_field: pszTask,
    });
    set_str!(Options {
        mask: LVGF_DESCRIPTIONTOP,
        input: &info.description_top,
        str_field: pszDescriptionTop,
    });
    set_str!(Options {
        mask: LVGF_DESCRIPTIONBOTTOM,
        input: &info.description_bottom,
        str_field: pszDescriptionBottom,
    });

    if info.header_alignment.is_some() || info.footer_alignment.is_some() {
        item.mask |= LVGF_ALIGN;
    }
    if let Some(header_align) = info.header_alignment {
        item.uAlign = match header_align {
            ListViewGroupAlignment::Left => LVGA_HEADER_LEFT,
            ListViewGroupAlignment::Center => LVGA_HEADER_CENTER,
            ListViewGroupAlignment::Right => LVGA_HEADER_RIGHT,
        };
    } else if info.footer_alignment.is_some() {
        item.uAlign = LVGA_HEADER_LEFT;
    }
    if let Some(footer_align) = info.footer_alignment {
        item.uAlign |= match footer_align {
            ListViewGroupAlignment::Left => LVGA_FOOTER_LEFT,
            ListViewGroupAlignment::Center => LVGA_FOOTER_CENTER,
            ListViewGroupAlignment::Right => LVGA_FOOTER_RIGHT,
        };
    }

    let state_flags = [
        (info.collapsed, LVGS_COLLAPSED),
        (info.hidden, LVGS_HIDDEN),
        (info.no_header, LVGS_NOHEADER),
        (info.collapsible, LVGS_COLLAPSIBLE),
        (info.focused, LVGS_FOCUSED),
        (info.selected, LVGS_SELECTED),
        (info.subseted, LVGS_SUBSETED),
        (info.subset_link_focused, LVGS_SUBSETLINKFOCUSED),
    ];
    if state_flags.iter().any(|(v, _)| v.is_some()) {
        item.mask |= LVGF_STATE;
        for (value, flag) in state_flags {
            if let Some(value) = value {
                item.stateMask |= flag;
                if value {
                    item.state |= flag;
                }
            }
        }
    }

    let res = unsafe {
        if info.create_new {
            SendMessageW(
                HWND(handle.cast()),
                LVM_INSERTGROUP,
                // Index where the group is to be added. If this is -1, the group is added at the end of the list.
                WPARAM((-1_isize) as usize),
                LPARAM(&mut item as *mut LVGROUP as _),
            )
        } else {
            SendMessageW(
                HWND(handle.cast()),
                LVM_SETGROUPINFO,
                WPARAM(info.group_id as _),
                LPARAM(&mut item as *mut LVGROUP as _),
            )
        }
    };
    if res.0 == -1 {
        tracing::error!(
            create_new = info.create_new,
            "Failed to create or update group for list view"
        );
    }
    // Note: res contains the id of the group.
}

/// Set the group that a list view item belongs to.
///
/// Adapted from [`native_windows_gui::ListView::update_item`].
///
/// # References
///
/// - [How to Use Groups in a List-View - Win32 apps | Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/controls/use-groups-in-a-list-view)
/// - [LVITEMW (commctrl.h) - Win32 apps | Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/api/commctrl/ns-commctrl-lvitemw)
/// - [LVM_SETITEM message (Commctrl.h) - Win32 apps | Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/controls/lvm-setitem)
pub fn list_view_item_set_group_id(
    list_view: &nwg::ListView,
    row_index: usize,
    group_id: Option<i32>,
) {
    use windows::Win32::{
        Foundation::{LPARAM, WPARAM},
        UI::{
            Controls::{I_GROUPIDNONE, LVIF_GROUPID, LVITEMW, LVM_SETITEMW},
            WindowsAndMessaging::SendMessageW,
        },
    };

    if !list_view.has_item(row_index, 0) {
        tracing::error!(
            row_index,
            "Tried to set group id for row that didn't exist."
        );
        return;
    }
    if !window_is_valid(list_view.handle) {
        tracing::error!("Tried to set group id for item inside invalid list view");
        return;
    }
    let Some(handle) = list_view.handle.hwnd() else {
        tracing::error!("Tried to set group id for item inside invalid list view");
        return;
    };
    let mut item = LVITEMW {
        mask: LVIF_GROUPID,
        iGroupId: group_id.unwrap_or(I_GROUPIDNONE.0),
        iItem: row_index as _,
        iSubItem: 0,
        ..Default::default()
    };

    let res = unsafe {
        SendMessageW(
            HWND(handle.cast()),
            LVM_SETITEMW,
            WPARAM(0),
            LPARAM(&mut item as *mut LVITEMW as _),
        )
    };
    if res.0 == 0 {
        tracing::error!("Failed to set group id for list view item");
    }
}

/// Get the group that a list view item belongs to.
///
/// Adapted from [`native_windows_gui::ListView::item`].
///
/// # References
///
/// - [How to Use Groups in a List-View - Win32 apps | Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/controls/use-groups-in-a-list-view)
/// - [LVITEMW (commctrl.h) - Win32 apps | Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/api/commctrl/ns-commctrl-lvitemw)
/// - <https://learn.microsoft.com/en-us/windows/win32/controls/lvm-getitem>
pub fn list_view_item_get_group_id(list_view: &nwg::ListView, row_index: usize) -> i32 {
    use windows::Win32::{
        Foundation::{LPARAM, WPARAM},
        UI::{
            Controls::{I_GROUPIDNONE, LVIF_GROUPID, LVITEMW, LVM_GETITEMW},
            WindowsAndMessaging::SendMessageW,
        },
    };

    if !window_is_valid(list_view.handle) {
        tracing::error!("Tried to set group id for item inside invalid list view");
        return I_GROUPIDNONE.0;
    }
    let Some(handle) = list_view.handle.hwnd() else {
        tracing::error!("Tried to set group id for item inside invalid list view");
        return I_GROUPIDNONE.0;
    };
    let mut item = LVITEMW {
        mask: LVIF_GROUPID,
        iItem: row_index as _,
        iSubItem: 0,
        ..Default::default()
    };

    let res = unsafe {
        SendMessageW(
            HWND(handle.cast()),
            LVM_GETITEMW,
            WPARAM(0),
            LPARAM(&mut item as *mut LVITEMW as _),
        )
    };
    if res.0 == 0 {
        // Item not found
        I_GROUPIDNONE.0
    } else {
        item.iGroupId
    }
}

/// When the taskbar is created, it registers a message with the
/// "TaskbarCreated" string and then broadcasts this message to all top-level
/// windows When the application receives this message, it should assume that
/// any taskbar icons it added have been removed and add them again.
///
/// # Reference
///
/// - Code copied from: [tray-icon/src/platform_impl/windows/mod.rs at
///   3c75d9031a915c108cc1886121b9b84cb9c8c312 ·
///   tauri-apps/tray-icon](https://github.com/tauri-apps/tray-icon/blob/3c75d9031a915c108cc1886121b9b84cb9c8c312/src/platform_impl/windows/mod.rs#L45-L48)
pub fn windows_msg_for_explorer_restart() -> u32 {
    static TASKBAR_RESTART_MSG: OnceLock<u32> = OnceLock::new();
    *TASKBAR_RESTART_MSG.get_or_init(|| {
        let msg = unsafe {
            windows::Win32::UI::WindowsAndMessaging::RegisterWindowMessageA(windows::core::s!(
                "TaskbarCreated"
            ))
        };
        if msg == 0 {
            tracing::error!(
                error = ?windows::core::Error::from_win32(),
                "Called \"RegisterWindowMessageA\" with \"TaskbarCreated\" and failed!"
            );
        } else {
            tracing::debug!(
                msg = ?msg,
                "Called \"RegisterWindowMessageA\" with \"TaskbarCreated\" and succeeded"
            );
        }
        msg
    })
}

/// A modified version of [`nwg::MessageWindow`] that allows detecting if
/// `explorer.exe` has restarted.
///
/// This requires creating the window with certain flags, see:
/// [tray-icon/src/platform_impl/windows/mod.rs at
/// 3c75d9031a915c108cc1886121b9b84cb9c8c312 ·
/// tauri-apps/tray-icon](https://github.com/tauri-apps/tray-icon/blob/3c75d9031a915c108cc1886121b9b84cb9c8c312/src/platform_impl/windows/mod.rs#L91-L112)
#[derive(Default, PartialEq, Eq)]
pub struct TrayWindow {
    pub handle: ControlHandle,
}
impl TrayWindow {
    pub fn builder() -> TrayWindowBuilder {
        TrayWindowBuilder {}
    }
}
impl Drop for TrayWindow {
    fn drop(&mut self) {
        self.handle.destroy();
    }
}
impl<'a> From<&'a TrayWindow> for nwg::ControlHandle {
    fn from(value: &'a TrayWindow) -> Self {
        value.handle
    }
}
/// Can use this component as a partial GUI as a workaround for the
/// [`nwd::NwgPartial`] derive macro's requirement that unknown controls must
/// have a parent.
impl nwg::PartialUi for TrayWindow {
    fn build_partial<W: Into<ControlHandle>>(
        data: &mut Self,
        parent: Option<W>,
    ) -> Result<(), nwg::NwgError> {
        let mut b = Self::builder();
        if let Some(p) = parent {
            b = b.parent(p.into());
        }
        b.build(data)
    }

    fn handles(&self) -> Vec<&ControlHandle> {
        vec![&self.handle]
    }
}

#[non_exhaustive]
pub struct TrayWindowBuilder {}

impl TrayWindowBuilder {
    pub fn parent<C: Into<ControlHandle>>(self, _p: C) -> Self {
        // self.parent = Some(p.into());
        self
    }
    pub fn build(self, out: &mut TrayWindow) -> Result<(), nwg::NwgError> {
        use windows::Win32::UI::WindowsAndMessaging::*;

        *out = Default::default();
        out.handle = nwg::ControlBase::build_hwnd()
            .class_name(nwg::Window::class_name(&Default::default()))
            .ex_flags(
                // Same styles as the tray-icon crate, see:
                // https://github.com/tauri-apps/tray-icon/blob/9231438b895055dddaf817dc44f680988c3d3c90/src/platform_impl/windows/mod.rs#L99-L106
                WS_EX_NOACTIVATE.0 | WS_EX_TRANSPARENT.0 | WS_EX_LAYERED.0 |
                // WS_EX_TOOLWINDOW prevents this window from ever showing up in the taskbar, which
                // we want to avoid. If you remove this style, this window won't show up in the
                // taskbar *initially*, but it can show up at some later point. This can sometimes
                // happen on its own after several hours have passed, although this has proven
                // difficult to reproduce. Alternatively, it can be manually triggered by killing
                // `explorer.exe` and then starting the process back up.
                // It is unclear why the bug is triggered by waiting for several hours.
                WS_EX_TOOLWINDOW.0,
            )
            .flags(WS_OVERLAPPED.0)
            .size((CW_USEDEFAULT, 0))
            .position((CW_USEDEFAULT, 0))
            .text("")
            .build()?;

        Ok(())
    }
}

pub trait LazyUiHooks {
    fn set_parent(&mut self, _parent: Option<ControlHandle>) {}
    /// Build this type when the `nwg::PartialUi::build_partial` method is
    /// called on `LazyUi`. (Defaults to `false`.)
    fn eager_build(&mut self) -> bool {
        false
    }
}
/// Implements `PartialUi` and delegates to a `PartialUi` inside a `RefCell`.
///
/// Note: if this is located inside a `PartialUI` then any parent passed to that
/// partial UI won't be passed down one step more to this type, and so the
/// initial parent will be set to `None`.
#[derive(Default)]
pub struct LazyUi<T> {
    pub ui: RefCell<T>,
    pub latest_parent: Cell<Option<ControlHandle>>,
    pub is_built: Cell<bool>,
}
impl<T> LazyUi<T> {
    /// Reset the wrapped UI to its default state.
    pub fn clear(&self)
    where
        T: Default,
    {
        *self.ui.borrow_mut() = T::default();
        self.is_built.set(false);
    }
    /// Build the UI with the most recently used parent. Make sure the UI isn't
    /// already constructed when calling this method, perhaps by calling `clear`
    /// first.
    pub fn build_with_latest_parent(&self) -> Result<(), nwg::NwgError>
    where
        T: LazyUiHooks + nwg::PartialUi,
    {
        self.build(self.latest_parent.get())
    }
    /// Build the UI with the given parent. Make sure the UI isn't already
    /// constructed when calling this method, perhaps by calling `clear` first.
    pub fn build(&self, parent: Option<ControlHandle>) -> Result<(), nwg::NwgError>
    where
        T: LazyUiHooks + nwg::PartialUi,
    {
        let mut this = self.ui.borrow_mut();
        self.latest_parent.set(parent);
        this.set_parent(parent);
        <T as nwg::PartialUi>::build_partial(&mut *this, parent)?;
        self.is_built.set(true);
        Ok(())
    }
}
impl<T> nwg::PartialUi for LazyUi<T>
where
    T: LazyUiHooks + nwg::PartialUi,
{
    fn build_partial<W: Into<ControlHandle>>(
        data: &mut Self,
        parent: Option<W>,
    ) -> Result<(), nwg::NwgError> {
        let parent = parent.map(Into::into);
        data.latest_parent.set(parent);
        data.ui.get_mut().set_parent(parent);
        if data.ui.get_mut().eager_build() {
            nwg::PartialUi::build_partial(data.ui.get_mut(), parent)?;
        }
        Ok(())
    }

    fn process_event(&self, evt: nwg::Event, evt_data: &nwg::EventData, handle: ControlHandle) {
        match self.ui.try_borrow() {
            Ok(ui) => ui.process_event(evt, evt_data, handle),
            // Events can be sent while we are constructing the inner UI, since
            // we can't clone `EventData` we just ignore them:
            Err(e) => tracing::error!(
                "Failed to process event {evt:?} on a `LazyUi<{}>` type: {e}",
                std::any::type_name::<T>()
            ),
        }
    }

    fn handles(&self) -> Vec<&'_ ControlHandle> {
        vec![]
    }
}
impl<T> core::ops::Deref for LazyUi<T> {
    type Target = RefCell<T>;
    fn deref(&self) -> &Self::Target {
        &self.ui
    }
}
impl<T> core::ops::DerefMut for LazyUi<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.ui
    }
}

/// Can be used as a `nwg_control` to store the parent that a `PartialUi` is
/// created with. This control can then be referred to as the parent of another
/// control and that will simply forward the captured parent. (Make sure this
/// capture control is placed before any control that uses it as a parent.)
#[derive(Default, PartialEq, Eq)]
pub struct ParentCapture {
    pub captured_parent: Option<ControlHandle>,
}
impl ParentCapture {
    pub fn builder() -> ParentCaptureBuilder {
        ParentCaptureBuilder(Self::default())
    }
    pub fn handle(&self) -> ControlHandle {
        self.captured_parent.unwrap_or(ControlHandle::NoHandle)
    }
}
pub struct ParentCaptureBuilder(ParentCapture);
impl ParentCaptureBuilder {
    pub fn parent<C: Into<ControlHandle>>(mut self, p: C) -> Self {
        self.0.captured_parent = Some(p.into());
        self
    }
    pub fn build(self, out: &mut ParentCapture) -> Result<(), nwg::NwgError> {
        *out = self.0;
        Ok(())
    }
}
impl From<&ParentCapture> for ControlHandle {
    fn from(control: &ParentCapture) -> Self {
        control.handle()
    }
}
impl From<&mut ParentCapture> for ControlHandle {
    fn from(control: &mut ParentCapture) -> Self {
        control.handle()
    }
}
impl PartialEq<ControlHandle> for ParentCapture {
    fn eq(&self, other: &ControlHandle) -> bool {
        self.handle() == *other
    }
}
impl PartialEq<ParentCapture> for ControlHandle {
    fn eq(&self, other: &ParentCapture) -> bool {
        *self == other.handle()
    }
}

/// Uses a single thread to serve multiple sleep requests.
pub struct TimerThread {
    join_handle: std::thread::JoinHandle<()>,
    send_time_request: mpsc::Sender<(Instant, Box<dyn FnOnce() + Send + 'static>)>,
}
impl TimerThread {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let join_handle = std::thread::Builder::new()
            .name("TimerThread".to_string())
            .spawn(move || Self::background_work(rx))
            .expect("Failed to spawn timer thread");
        Self {
            join_handle,
            send_time_request: tx,
        }
    }
    /// Call a function and catch all potential panics.
    fn safe_call(f: impl FnOnce()) {
        /// Dropping a value might panic, so we catch that in a custom Drop.
        struct SafeDrop(Option<Box<dyn std::any::Any + Send>>);
        impl Drop for SafeDrop {
            fn drop(&mut self) {
                while let Some(value) = self.0.take() {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        drop(value);
                    }));
                    match result {
                        Ok(()) => {}
                        Err(e) => {
                            self.0 = Some(e);
                        }
                    }
                }
            }
        }
        if let Err(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
            drop(SafeDrop(Some(e)));
        }
    }
    fn background_work(rx: mpsc::Receiver<(Instant, Box<dyn FnOnce() + Send + 'static>)>) {
        let mut times = BTreeMap::<Instant, Box<dyn FnOnce() + Send + 'static>>::new();
        loop {
            let (new_time, f) = if let Some(first_time) = times.first_entry() {
                let sleep_to = first_time.key();
                let Some(timeout) = sleep_to.checked_duration_since(Instant::now()) else {
                    let f = first_time.remove();
                    Self::safe_call(f);
                    continue;
                };
                match rx.recv_timeout(timeout) {
                    Ok(msg) => msg,
                    Err(RecvTimeoutError::Disconnected) => {
                        // No more messages, finish waiting for existing messages:
                        for (sleep_to, f) in times.into_iter() {
                            if let Some(timeout) = sleep_to.checked_duration_since(Instant::now()) {
                                std::thread::sleep(timeout);
                            }
                            Self::safe_call(f);
                        }
                        break;
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        let f = first_time.remove();
                        Self::safe_call(f);
                        continue;
                    }
                }
            } else {
                match rx.recv() {
                    Ok(msg) => msg,
                    Err(mpsc::RecvError) => break,
                }
            };
            times.insert(new_time, f);
        }
    }
    pub fn notify_at(&self, time: Instant, f: impl FnOnce() + Send + 'static) {
        self.send_time_request
            .send((time, Box::new(f)))
            .expect("Background timer thread has exited");
    }
    /// Notify a waker when the time has occurred. Sets `Err(true)` for inside
    /// the `Mutex` after the time has elapsed, so there is no point to queue a
    /// waker there after that.
    fn notify_waker(
        &self,
        time: Instant,
        waker: Arc<std::sync::Mutex<Result<std::task::Waker, bool>>>,
    ) {
        self.notify_at(time, move || {
            let result = std::mem::replace(&mut *waker.lock().unwrap(), Err(true));
            if let Ok(waker) = result {
                waker.wake();
            }
        })
    }
    pub fn delay_future(&self, delay: Duration) -> impl std::future::Future<Output = ()> {
        self.notify_future(Instant::now() + delay)
    }
    /// Create a future that will become ready at the specified time.
    pub fn notify_future(&self, time: Instant) -> impl std::future::Future<Output = ()> {
        struct WaitFut(Arc<std::sync::Mutex<Result<std::task::Waker, bool>>>);
        impl std::future::Future for WaitFut {
            type Output = ();

            fn poll(
                self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Self::Output> {
                let this = &mut self.get_mut().0;
                let mut guard = this.lock().unwrap();
                if let &Err(true) = &*guard {
                    std::task::Poll::Ready(())
                } else {
                    *guard = Ok(cx.waker().clone());
                    std::task::Poll::Pending
                }
            }
        }
        let shared = Arc::new(std::sync::Mutex::new(Err(false)));
        self.notify_waker(time, shared.clone());
        WaitFut(shared)
    }
    pub fn get_global() -> &'static Self {
        static GLOBAL: OnceLock<TimerThread> = OnceLock::new();
        GLOBAL.get_or_init(Self::new)
    }
}
impl Default for TimerThread {
    fn default() -> Self {
        Self::new()
    }
}

/// An alternative to [`nwg::AnimationTimer`] that has less CPU usage.
///
/// Note: this is a [`nwg::PartialUi`] instead of a control because it needs to
/// listen to events.
///
/// # Examples
///
/// ```rust
/// extern crate native_windows_derive as nwd;
/// extern crate native_windows_gui as nwg;
///
/// use virtual_desktop_manager::nwg_ext::{FastTimer, ParentCapture};
/// use std::time::Duration;
///
/// #[derive(nwd::NwgPartial, Default)]
/// struct MyUi {
///     /// Captures the parent that this partial UI is instantiated with.
///     #[nwg_control]
///     capture: ParentCapture,
///
///     #[nwg_partial(parent: capture)]
///     #[nwg_events((notice, OnNotice): [Self::on_tick])]
///     my_timer: FastTimer,
/// }
/// impl MyUi {
///     pub fn start_interval(&self) {
///         self.my_timer.start_interval(Duration::from_millis(100));
///     }
///     pub fn on_tick(&self) {
///         // Do something every 100 milliseconds...
///     }
/// }
///
///# fn main() {}
/// ```
#[derive(nwd::NwgPartial)]
pub struct FastTimer {
    #[nwg_control]
    #[nwg_events( OnNotice: [Self::on_notice] )]
    pub notice: nwg::Notice,
    callback: RefCell<Box<dyn Fn() + 'static>>,
    cancel_latest: RefCell<Arc<AtomicBool>>,
    /// `Some` if an interval is configured in which case the duration between
    /// ticks is stored as well as when the next tick was scheduled.
    interval_config: Cell<Option<(Duration, Instant)>>,
}
impl FastTimer {
    pub fn set_callback(&self, callback: impl Fn() + 'static) {
        *self.callback.borrow_mut() = Box::new(callback);
    }
    /// This will cancel any queued timeout or interval.
    pub fn cancel_last(&self) {
        let cancel_latest = self.cancel_latest.borrow();
        cancel_latest.store(true, std::sync::atomic::Ordering::Release);
        self.interval_config.set(None);
    }
    pub fn notify_after(&self, duration: Duration) {
        self.notify_at(
            Instant::now()
                .checked_add(duration)
                .expect("Time is out of bounds"),
        );
    }
    pub fn notify_at(&self, time_to_notify_at: Instant) {
        let sender = self.notice.sender();
        let canceled = {
            let mut cancel_latest = self.cancel_latest.borrow_mut();
            cancel_latest.store(true, std::sync::atomic::Ordering::Release);
            let canceled = Arc::new(AtomicBool::new(false));
            *cancel_latest = canceled.clone();
            canceled
        };
        TimerThread::get_global().notify_at(time_to_notify_at, move || {
            if !canceled.load(std::sync::atomic::Ordering::Acquire) {
                sender.notice();
            }
        })
    }
    pub fn start_interval(&self, between_ticks: Duration) {
        let target_time = Instant::now() + between_ticks;
        self.interval_config.set(Some((between_ticks, target_time)));
        self.notify_at(target_time);
    }
    fn on_notice(&self) {
        if let Some((between_ticks, target_time)) = self.interval_config.get() {
            let mut new_target = target_time + between_ticks;
            let now = Instant::now();
            if new_target < now {
                // System might have been asleep or something, just restart
                // interval from current time.
                new_target = now + between_ticks;
            }
            self.notify_at(new_target);
        }
        self.callback.borrow()();
    }
}
impl Default for FastTimer {
    fn default() -> Self {
        Self {
            notice: Default::default(),
            callback: RefCell::new(Box::new(|| {})),
            cancel_latest: RefCell::new(Arc::new(AtomicBool::new(false))),
            interval_config: Cell::new(None),
        }
    }
}
impl Drop for FastTimer {
    fn drop(&mut self) {
        self.cancel_last();
    }
}

/// An alternative to [`nwg::AnimationTimer`] that has less CPU usage.
///
/// # Examples
///
/// ```rust,no_run
/// extern crate native_windows_gui as nwg;
/// extern crate native_windows_derive as nwd;
///
/// use nwd::{NwgUi, NwgPartial};
/// use nwg::NativeUi;
/// use virtual_desktop_manager::nwg_ext::FastTimerControl;
/// use std::time::Duration;
///
/// #[derive(NwgUi, Default)]
/// pub struct MyUi {
///     #[nwg_control]
///     window: nwg::MessageWindow,
///
///     #[nwg_control(interval: Duration::from_millis(25))]
///     #[nwg_events(OnNotice: [Self::on_tick])]
///     my_timer: FastTimerControl,
///
///     count: std::cell::Cell<u32>,
///
///     #[nwg_partial(parent: window)]
///     sub: MySubUi,
/// }
/// impl MyUi {
///     pub fn on_tick(&self) {
///         // Do something every 2 milliseconds...
///         self.count.set(self.count.get() + 1);
///     }
/// }
///
/// #[derive(NwgPartial, Default)]
/// struct MySubUi {
///     #[nwg_control(interval: Duration::from_millis(110))]
///     #[nwg_events(OnNotice: [Self::on_sub_tick])]
///     my_sub_timer: FastTimerControl,
/// }
/// impl MySubUi {
///     pub fn on_sub_tick(&self) {
///         // Do something every 10 milliseconds...
///         nwg::stop_thread_dispatch();
///     }
/// }
///
/// fn main() {
///     nwg::init().expect("Failed to init Native Windows GUI");
///     let ui = MyUi::build_ui(Default::default()).expect("Failed to build UI");
///     nwg::dispatch_thread_events();
///     assert_eq!(ui.count.get(), 4);
/// }
/// ```
#[derive(Default)]
pub struct FastTimerControl {
    pub notice: nwg::Notice,
    is_last_active: RefCell<Arc<AtomicBool>>,
}
impl FastTimerControl {
    pub fn builder() -> FastTimerControlBuilder {
        FastTimerControlBuilder {
            parent: None,
            interval: None,
        }
    }

    /// True if the timer is waiting for the next timeout or interval. This
    /// means that an [`nwg::Event::OnNotice`] event will be emitted in the
    /// future unless the timer is canceled.
    pub fn is_waiting(&self) -> bool {
        self.is_last_active
            .borrow()
            .load(std::sync::atomic::Ordering::Acquire)
    }
    /// This will cancel any queued timeout or interval.
    pub fn cancel_last(&self) {
        let last_active = self.is_last_active.borrow();
        last_active.store(false, std::sync::atomic::Ordering::Release);
    }
    fn new_enable_signal(&self) -> Arc<AtomicBool> {
        let mut last_active = self.is_last_active.borrow_mut();
        last_active.store(false, std::sync::atomic::Ordering::Release);
        let new_active = Arc::new(AtomicBool::new(true));
        *last_active = new_active.clone();
        new_active
    }

    pub fn notify_after(&self, duration: Duration) {
        self.notify_at(
            Instant::now()
                .checked_add(duration)
                .expect("Time is out of bounds"),
        );
    }
    pub fn notify_at(&self, time_to_notify_at: Instant) {
        let sender = self.notice.sender();
        let is_active = self.new_enable_signal();
        TimerThread::get_global().notify_at(time_to_notify_at, move || {
            if is_active.load(std::sync::atomic::Ordering::Acquire) {
                is_active.store(false, std::sync::atomic::Ordering::Release);
                sender.notice();
            }
        })
    }
    pub fn start_interval(&self, between_ticks: Duration) {
        struct CallbackState {
            target_time: Instant,
            between_ticks: Duration,
            sender: nwg::NoticeSender,
            is_active: Arc<AtomicBool>,
            timer_thread: &'static TimerThread,
        }
        impl CallbackState {
            fn into_callback(mut self) -> impl FnOnce() + Send + 'static {
                move || {
                    if self.is_active.load(std::sync::atomic::Ordering::Acquire) {
                        self.sender.notice();

                        self.target_time += self.between_ticks;
                        let now = Instant::now();
                        if self.target_time < now {
                            // System might have been asleep or something, just restart
                            // interval from current time.
                            self.target_time = now + self.between_ticks;
                        }

                        let timer_thread = self.timer_thread;
                        let target_time = self.target_time;
                        timer_thread.notify_at(target_time, self.into_callback());
                    }
                }
            }
        }
        let target_time = Instant::now() + between_ticks;
        let timer_thread = TimerThread::get_global();
        let state = CallbackState {
            target_time,
            between_ticks,
            sender: self.notice.sender(),
            is_active: self.new_enable_signal(),
            timer_thread,
        };
        timer_thread.notify_at(target_time, state.into_callback());
    }
}
impl PartialEq for FastTimerControl {
    fn eq(&self, other: &Self) -> bool {
        self.notice == other.notice
    }
}
impl Eq for FastTimerControl {}
impl Drop for FastTimerControl {
    fn drop(&mut self) {
        self.cancel_last();
    }
}

pub struct FastTimerControlBuilder {
    parent: Option<ControlHandle>,
    interval: Option<Duration>,
}
impl FastTimerControlBuilder {
    pub fn parent<C: Into<ControlHandle>>(mut self, p: C) -> Self {
        self.parent = Some(p.into());
        self
    }
    pub fn interval(mut self, interval: Duration) -> Self {
        self.interval = Some(interval);
        self
    }
    pub fn build(self, out: &mut FastTimerControl) -> Result<(), nwg::NwgError> {
        out.cancel_last();

        let mut notice_builder = nwg::Notice::builder();
        if let Some(parent) = self.parent {
            notice_builder = notice_builder.parent(parent);
        }
        notice_builder.build(&mut out.notice)?;

        if let Some(interval) = self.interval {
            out.start_interval(interval);
        }

        Ok(())
    }
}
impl From<&FastTimerControl> for ControlHandle {
    fn from(control: &FastTimerControl) -> Self {
        control.notice.handle
    }
}
impl From<&mut FastTimerControl> for ControlHandle {
    fn from(control: &mut FastTimerControl) -> Self {
        control.notice.handle
    }
}
impl PartialEq<ControlHandle> for FastTimerControl {
    fn eq(&self, other: &ControlHandle) -> bool {
        self.notice.handle == *other
    }
}
impl PartialEq<FastTimerControl> for ControlHandle {
    fn eq(&self, other: &FastTimerControl) -> bool {
        *self == other.notice.handle
    }
}
