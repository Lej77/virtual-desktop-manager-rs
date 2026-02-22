//! Fork of [`nwg::NumberSelect`] that fixes some issues. See [`NumberSelect2`]
//! for more info.

mod high_dpi {
    //! Fork of [`nwg::win32::high_dpi`].

    #[allow(deprecated, unused_imports)]
    pub use nwg::{dpi, scale_factor, set_dpi_awareness};

    #[cfg(not(feature = "nwg_high_dpi"))]
    pub unsafe fn logical_to_physical(x: i32, y: i32) -> (i32, i32) {
        (x, y)
    }

    #[cfg(feature = "nwg_high_dpi")]
    pub unsafe fn logical_to_physical(x: i32, y: i32) -> (i32, i32) {
        use muldiv::MulDiv;
        use windows::Win32::UI::WindowsAndMessaging::USER_DEFAULT_SCREEN_DPI;

        let dpi = dpi();
        let x = x
            .mul_div_round(dpi, USER_DEFAULT_SCREEN_DPI as i32)
            .unwrap_or(x);
        let y = y
            .mul_div_round(dpi, USER_DEFAULT_SCREEN_DPI as i32)
            .unwrap_or(y);
        (x, y)
    }

    #[cfg(not(feature = "nwg_high_dpi"))]
    pub unsafe fn physical_to_logical(x: i32, y: i32) -> (i32, i32) {
        (x, y)
    }

    #[cfg(feature = "nwg_high_dpi")]
    pub unsafe fn physical_to_logical(x: i32, y: i32) -> (i32, i32) {
        use muldiv::MulDiv;
        use windows::Win32::UI::WindowsAndMessaging::USER_DEFAULT_SCREEN_DPI;

        let dpi = dpi();
        let x = x
            .mul_div_round(USER_DEFAULT_SCREEN_DPI as i32, dpi)
            .unwrap_or(x);
        let y = y
            .mul_div_round(USER_DEFAULT_SCREEN_DPI as i32, dpi)
            .unwrap_or(y);
        (x, y)
    }
}

mod wh {
    //! Adapted from [`nwg::win32::window_helper`].

    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::{BOOL, HWND, LPARAM, POINT, RECT, WPARAM},
            Graphics::Gdi::{InvalidateRect, ScreenToClient, UpdateWindow, HFONT},
            UI::{
                Input::KeyboardAndMouse::{GetFocus, SetFocus},
                WindowsAndMessaging::{
                    AdjustWindowRectEx, GetClientRect, GetParent, GetWindowLongW, GetWindowRect,
                    GetWindowTextLengthW, GetWindowTextW, SendMessageW, SetWindowPos,
                    SetWindowTextW, ShowWindow, GWL_EXSTYLE, GWL_STYLE, SWP_NOACTIVATE,
                    SWP_NOCOPYBITS, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER,
                    SW_HIDE, SW_SHOW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_GETFONT, WM_SETFONT,
                    WS_DISABLED,
                },
            },
        },
    };

    use crate::nwg_ext::{from_utf16, to_utf16};

    use super::high_dpi;

    #[inline(always)]
    #[cfg(target_pointer_width = "64")]
    pub fn get_window_long(handle: HWND, index: i32) -> isize {
        use windows::Win32::UI::WindowsAndMessaging::{GetWindowLongPtrW, WINDOW_LONG_PTR_INDEX};

        unsafe { GetWindowLongPtrW(handle, WINDOW_LONG_PTR_INDEX(index)) }
    }

    #[inline(always)]
    #[cfg(target_pointer_width = "32")]
    pub fn get_window_long2(handle: HWND, index: i32) -> i32 {
        use windows::Win32::UI::WindowsAndMessaging::{GetWindowLongW, WINDOW_LONG_PTR_INDEX};
        unsafe { GetWindowLongW(handle, WINDOW_LONG_PTR_INDEX(index)) }
    }

    #[inline(always)]
    #[cfg(target_pointer_width = "64")]
    pub fn set_window_long(handle: HWND, index: i32, v: usize) {
        use windows::Win32::UI::WindowsAndMessaging::{SetWindowLongPtrW, WINDOW_LONG_PTR_INDEX};

        unsafe {
            SetWindowLongPtrW(handle, WINDOW_LONG_PTR_INDEX(index), v as isize);
        }
    }

    #[inline(always)]
    #[cfg(target_pointer_width = "32")]
    pub fn set_window_long(handle: HWND, index: i32, v: usize) {
        use windows::Win32::UI::WindowsAndMessaging::{SetWindowLongW, WINDOW_LONG_PTR_INDEX};
        unsafe {
            SetWindowLongW(handle, WINDOW_LONG_PTR_INDEX(index), v as i32);
        }
    }

    /// Set the font of a window
    pub unsafe fn set_window_font(handle: HWND, font_handle: Option<HFONT>, redraw: bool) {
        let font_handle = font_handle.unwrap_or_default();

        SendMessageW(
            handle,
            WM_SETFONT,
            WPARAM(font_handle.0 as usize),
            LPARAM(redraw as isize),
        );
    }

    pub fn get_window_font(handle: HWND) -> HFONT {
        unsafe {
            let h = SendMessageW(handle, WM_GETFONT, WPARAM(0), LPARAM(0));
            HFONT(h.0 as *mut _)
        }
    }
    pub unsafe fn set_focus(handle: HWND) {
        _ = SetFocus(handle);
    }

    pub unsafe fn get_focus(handle: HWND) -> bool {
        GetFocus() == handle
    }
    pub unsafe fn get_window_enabled(handle: HWND) -> bool {
        let style = get_window_long(handle, GWL_STYLE.0) as u32;
        (style & WS_DISABLED.0) != WS_DISABLED.0
    }

    pub unsafe fn set_window_enabled(handle: HWND, enabled: bool) {
        let old_style = get_window_long(handle, GWL_STYLE.0) as usize;
        if enabled {
            set_window_long(handle, GWL_STYLE.0, old_style & (!WS_DISABLED.0 as usize));
        } else {
            set_window_long(handle, GWL_STYLE.0, old_style | (WS_DISABLED.0 as usize));
        }

        // Tell the control to redraw itself to show the new style.
        let _ = InvalidateRect(handle, None, BOOL::from(true));
        let _ = UpdateWindow(handle);
    }
    pub unsafe fn get_window_text(handle: HWND) -> String {
        let buffer_size = GetWindowTextLengthW(handle) as usize + 1;
        if buffer_size == 0 {
            return String::new();
        }

        let mut buffer: Vec<u16> = vec![0; buffer_size];

        if GetWindowTextW(handle, &mut buffer) == 0 {
            String::new()
        } else {
            from_utf16(&buffer[..])
        }
    }
    pub unsafe fn set_window_text(handle: HWND, text: &str) {
        let text = to_utf16(text);
        let _ = SetWindowTextW(handle, PCWSTR::from_raw(text.as_ptr()));
    }

    pub unsafe fn set_window_position(handle: HWND, x: i32, y: i32) {
        nwg::dpi();
        let (x, y) = high_dpi::logical_to_physical(x, y);
        let _ = SetWindowPos(
            handle,
            HWND::default(),
            x,
            y,
            0,
            0,
            SWP_NOZORDER | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        );
    }
    pub unsafe fn get_window_position(handle: HWND) -> (i32, i32) {
        let mut r = RECT::default();
        let _ = GetWindowRect(handle, &mut r);

        let parent = GetParent(handle);
        let (x, y) = if let Ok(parent) = parent {
            let mut pt = POINT {
                x: r.left,
                y: r.top,
            };
            let _ = ScreenToClient(parent, &mut pt);
            (pt.x, pt.y)
        } else {
            (r.left, r.top)
        };

        high_dpi::physical_to_logical(x, y)
    }

    pub unsafe fn set_window_size(handle: HWND, w: u32, h: u32, fix: bool) {
        let (mut w, mut h) = high_dpi::logical_to_physical(w as i32, h as i32);

        if fix {
            let flags = GetWindowLongW(handle, GWL_STYLE) as u32;
            let ex_flags = GetWindowLongW(handle, GWL_EXSTYLE) as u32;
            let mut rect = RECT {
                left: 0,
                top: 0,
                right: w,
                bottom: h,
            };
            let _ = AdjustWindowRectEx(
                &mut rect,
                WINDOW_STYLE(flags),
                BOOL::from(false),
                WINDOW_EX_STYLE(ex_flags),
            );

            w = rect.right - rect.left;
            h = rect.bottom - rect.top;
        }

        let _ = SetWindowPos(
            handle,
            HWND::default(),
            0,
            0,
            w,
            h,
            SWP_NOZORDER | SWP_NOMOVE | SWP_NOACTIVATE | SWP_NOCOPYBITS | SWP_NOOWNERZORDER,
        );
    }

    pub unsafe fn get_window_size(handle: HWND) -> (u32, u32) {
        get_window_size_impl(handle, false)
    }

    #[allow(unused)]
    pub unsafe fn get_window_physical_size(handle: HWND) -> (u32, u32) {
        get_window_size_impl(handle, true)
    }

    unsafe fn get_window_size_impl(handle: HWND, return_physical: bool) -> (u32, u32) {
        let mut r = RECT::default();
        let _ = GetClientRect(handle, &mut r);

        let (w, h) = if return_physical {
            (r.right, r.bottom)
        } else {
            high_dpi::physical_to_logical(r.right, r.bottom)
        };

        (w as u32, h as u32)
    }
    pub unsafe fn set_window_visibility(handle: HWND, visible: bool) {
        let visible = if visible { SW_SHOW } else { SW_HIDE };
        let _ = ShowWindow(handle, visible);
    }

    pub unsafe fn get_window_visibility(handle: HWND) -> bool {
        windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(handle).as_bool()
    }
}

use std::{cell::RefCell, cmp::Ordering, rc::Rc};

use nwg::{
    bind_raw_event_handler, unbind_raw_event_handler, Button, ButtonFlags, ControlBase,
    ControlHandle, Font, Notice, NumberSelectData, NumberSelectFlags, NwgError, RawEventHandler,
    TextInput, TextInputFlags,
};
use windows::Win32::{
    Foundation::HWND,
    Graphics::Gdi::HFONT,
    UI::WindowsAndMessaging::{
        BN_CLICKED, WM_COMMAND, WS_BORDER, WS_CHILD, WS_CLIPCHILDREN, WS_EX_CONTROLPARENT,
        WS_TABSTOP, WS_VISIBLE,
    },
};

const NOT_BOUND: &str = "UpDown is not yet bound to a winapi object";
const BAD_HANDLE: &str = "INTERNAL ERROR: UpDown handle is not HWND!";

/// Adapted from [`native_windows_gui::win32::base_helper::check_hwnd`].
fn check_hwnd(handle: &ControlHandle, not_bound: &str, bad_handle: &str) -> HWND {
    if handle.blank() {
        panic!("{}", not_bound);
    }
    match handle.hwnd() {
        Some(hwnd) => {
            if unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindow(HWND(hwnd.cast())) }
                .as_bool()
            {
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

/**
Fork of [`nwg::NumberSelect`] that has some improvements.

# Differences

- Up and Down arrow keys will increment and decrement the number.
- Scroll events on the select control will increment or decrement the number.
- Manual text edits in the field will be validated and used to update the number data.
- Event that will be used whenever the number data is changed by the UI.
   - Listen to `OnNotice` event to see changes.

# Original docs

A NumberSelect control is a pair of arrow buttons that the user can click to increment or decrement a value.
NumberSelect is implemented as a custom control because the one provided by winapi really sucks.

Requires the `number-select` feature.

**Builder parameters:**
  * `parent`:   **Required.** The number select parent container.
  * `value`:    The default value of the number select
  * `size`:     The number select size.
  * `position`: The number select position.
  * `enabled`:  If the number select can be used by the user. It also has a grayed out look if disabled.
  * `flags`:    A combination of the NumberSelectFlags values.
  * `font`:     The font used for the number select text

**Control events:**
  * `MousePress(_)`: Generic mouse press events on the button
  * `OnMouseMove`: Generic mouse mouse event

```rust
use virtual_desktop_manager::nwg_ext;

fn build_number_select(num_select: &mut nwg_ext::NumberSelect2, window: &nwg::Window, font: &nwg::Font) {
    nwg_ext::NumberSelect2::builder()
        .font(Some(font))
        .parent(window)
        .build(num_select);
}
```

*/
#[derive(Default)]
pub struct NumberSelect2 {
    pub handle: ControlHandle,
    data: Rc<RefCell<NumberSelectData>>,
    edit: TextInput,
    btn_up: Button,
    btn_down: Button,
    notice: Notice,
    handler: Option<RawEventHandler>,
    edit_handler: Option<RawEventHandler>,
}

impl NumberSelect2 {
    pub fn builder<'a>() -> NumberSelectBuilder<'a> {
        NumberSelectBuilder {
            size: (100, 25),
            position: (0, 0),
            data: NumberSelectData::default(),
            enabled: true,
            flags: None,
            font: None,
            parent: None,
        }
    }

    /// Returns inner data specifying the possible input of a number select
    /// See [NumberSelectData](enum.NumberSelectData.html) for the possible values
    pub fn data(&self) -> NumberSelectData {
        *self.data.borrow()
    }

    /// Sets the inner data specifying the possible input of a number select. Also update the value display.
    /// See [NumberSelectData](enum.NumberSelectData.html) for the possible values
    pub fn set_data(&self, v: NumberSelectData) {
        *self.data.borrow_mut() = v;
        self.edit.set_text(&v.formatted_value());
    }

    /// Returns the font of the control
    pub fn font(&self) -> Option<Font> {
        let handle = check_hwnd(&self.handle, NOT_BOUND, BAD_HANDLE);
        let font_handle = wh::get_window_font(handle);
        if font_handle.0.is_null() {
            None
        } else {
            Some(Font {
                handle: font_handle.0 as *mut _,
            })
        }
    }

    /// Sets the font of the control
    pub fn set_font(&self, font: Option<&Font>) {
        let handle = check_hwnd(&self.handle, NOT_BOUND, BAD_HANDLE);
        unsafe {
            wh::set_window_font(handle, font.map(|f| HFONT(f.handle.cast())), true);
        }
    }

    /// Returns true if the control currently has the keyboard focus
    pub fn focus(&self) -> bool {
        let handle = check_hwnd(&self.handle, NOT_BOUND, BAD_HANDLE);
        unsafe { wh::get_focus(handle) }
    }

    /// Sets the keyboard focus on the button.
    pub fn set_focus(&self) {
        let handle = check_hwnd(&self.handle, NOT_BOUND, BAD_HANDLE);
        unsafe {
            wh::set_focus(handle);
        }
    }

    /// Returns true if the control user can interact with the control, return false otherwise
    pub fn enabled(&self) -> bool {
        let handle = check_hwnd(&self.handle, NOT_BOUND, BAD_HANDLE);
        unsafe { wh::get_window_enabled(handle) }
    }

    /// Enable or disable the control
    pub fn set_enabled(&self, v: bool) {
        let handle = check_hwnd(&self.handle, NOT_BOUND, BAD_HANDLE);
        unsafe { wh::set_window_enabled(handle, v) }
    }

    /// Returns true if the control is visible to the user. Will return true even if the
    /// control is outside of the parent client view (ex: at the position (10000, 10000))
    pub fn visible(&self) -> bool {
        let handle = check_hwnd(&self.handle, NOT_BOUND, BAD_HANDLE);
        unsafe { wh::get_window_visibility(handle) }
    }

    /// Show or hide the control to the user
    pub fn set_visible(&self, v: bool) {
        let handle = check_hwnd(&self.handle, NOT_BOUND, BAD_HANDLE);
        unsafe { wh::set_window_visibility(handle, v) }
    }

    /// Returns the size of the control in the parent window
    pub fn size(&self) -> (u32, u32) {
        let handle = check_hwnd(&self.handle, NOT_BOUND, BAD_HANDLE);
        unsafe { wh::get_window_size(handle) }
    }

    /// Sets the size of the control in the parent window
    pub fn set_size(&self, x: u32, y: u32) {
        let handle = check_hwnd(&self.handle, NOT_BOUND, BAD_HANDLE);
        unsafe { wh::set_window_size(handle, x, y, false) }
    }

    /// Returns the position of the control in the parent window
    pub fn position(&self) -> (i32, i32) {
        let handle = check_hwnd(&self.handle, NOT_BOUND, BAD_HANDLE);
        unsafe { wh::get_window_position(handle) }
    }

    /// Sets the position of the control in the parent window
    pub fn set_position(&self, x: i32, y: i32) {
        let handle = check_hwnd(&self.handle, NOT_BOUND, BAD_HANDLE);
        unsafe { wh::set_window_position(handle, x, y) }
    }

    /// Winapi class name used during control creation
    pub fn class_name(&self) -> &'static str {
        "NativeWindowsGuiWindow"
    }

    /// Winapi base flags used during window creation
    pub fn flags(&self) -> u32 {
        WS_VISIBLE.0
    }

    /// Winapi flags required by the control
    pub fn forced_flags(&self) -> u32 {
        (WS_CHILD | WS_BORDER | WS_CLIPCHILDREN).0
    }
}

impl Drop for NumberSelect2 {
    fn drop(&mut self) {
        if let Some(h) = self.handler.as_ref() {
            drop(unbind_raw_event_handler(h));
        }

        self.handle.destroy();
    }
}

pub struct NumberSelectBuilder<'a> {
    size: (i32, i32),
    position: (i32, i32),
    data: NumberSelectData,
    enabled: bool,
    flags: Option<NumberSelectFlags>,
    font: Option<&'a Font>,
    parent: Option<ControlHandle>,
}

impl<'a> NumberSelectBuilder<'a> {
    pub fn flags(mut self, flags: NumberSelectFlags) -> NumberSelectBuilder<'a> {
        self.flags = Some(flags);
        self
    }

    pub fn size(mut self, size: (i32, i32)) -> NumberSelectBuilder<'a> {
        self.size = size;
        self
    }

    pub fn position(mut self, pos: (i32, i32)) -> NumberSelectBuilder<'a> {
        self.position = pos;
        self
    }

    pub fn enabled(mut self, e: bool) -> NumberSelectBuilder<'a> {
        self.enabled = e;
        self
    }

    pub fn font(mut self, font: Option<&'a Font>) -> NumberSelectBuilder<'a> {
        self.font = font;
        self
    }

    // Int values
    pub fn value_int(mut self, v: i64) -> NumberSelectBuilder<'a> {
        match &mut self.data {
            NumberSelectData::Int { value, .. } => {
                *value = v;
            }
            data => {
                *data = NumberSelectData::Int {
                    value: v,
                    step: 1,
                    max: i64::MAX,
                    min: i64::MIN,
                }
            }
        }
        self
    }

    pub fn step_int(mut self, v: i64) -> NumberSelectBuilder<'a> {
        match &mut self.data {
            NumberSelectData::Int { step, .. } => {
                *step = v;
            }
            data => {
                *data = NumberSelectData::Int {
                    value: 0,
                    step: v,
                    max: i64::MAX,
                    min: i64::MIN,
                }
            }
        }
        self
    }

    pub fn max_int(mut self, v: i64) -> NumberSelectBuilder<'a> {
        match &mut self.data {
            NumberSelectData::Int { max, .. } => {
                *max = v;
            }
            data => {
                *data = NumberSelectData::Int {
                    value: 0,
                    step: 1,
                    max: v,
                    min: i64::MIN,
                }
            }
        }
        self
    }

    pub fn min_int(mut self, v: i64) -> NumberSelectBuilder<'a> {
        match &mut self.data {
            NumberSelectData::Int { min, .. } => {
                *min = v;
            }
            data => {
                *data = NumberSelectData::Int {
                    value: 0,
                    step: 1,
                    max: i64::MAX,
                    min: v,
                }
            }
        }
        self
    }

    // Float values
    pub fn value_float(mut self, v: f64) -> NumberSelectBuilder<'a> {
        match &mut self.data {
            NumberSelectData::Float { value, .. } => {
                *value = v;
            }
            data => {
                *data = NumberSelectData::Float {
                    value: v,
                    step: 1.0,
                    max: 1000000.0,
                    min: -1000000.0,
                    decimals: 2,
                }
            }
        }
        self
    }

    pub fn step_float(mut self, v: f64) -> NumberSelectBuilder<'a> {
        match &mut self.data {
            NumberSelectData::Float { step, .. } => {
                *step = v;
            }
            data => {
                *data = NumberSelectData::Float {
                    value: 0.0,
                    step: v,
                    max: 1000000.0,
                    min: -1000000.0,
                    decimals: 2,
                }
            }
        }
        self
    }

    pub fn max_float(mut self, v: f64) -> NumberSelectBuilder<'a> {
        match &mut self.data {
            NumberSelectData::Float { max, .. } => {
                *max = v;
            }
            data => {
                *data = NumberSelectData::Float {
                    value: 0.0,
                    step: 1.0,
                    max: v,
                    min: -1000000.0,
                    decimals: 2,
                }
            }
        }
        self
    }

    pub fn min_float(mut self, v: f64) -> NumberSelectBuilder<'a> {
        match &mut self.data {
            NumberSelectData::Float { min, .. } => {
                *min = v;
            }
            data => {
                *data = NumberSelectData::Float {
                    value: 0.0,
                    step: 1.0,
                    max: 1000000.0,
                    min: v,
                    decimals: 2,
                }
            }
        }
        self
    }

    pub fn decimals(mut self, v: u8) -> NumberSelectBuilder<'a> {
        match &mut self.data {
            NumberSelectData::Float { decimals, .. } => {
                *decimals = v;
            }
            data => {
                *data = NumberSelectData::Float {
                    value: 0.0,
                    step: 1.0,
                    max: 1000000.0,
                    min: -1000000.0,
                    decimals: v,
                }
            }
        }
        self
    }

    pub fn parent<C: Into<ControlHandle>>(mut self, p: C) -> NumberSelectBuilder<'a> {
        self.parent = Some(p.into());
        self
    }

    pub fn build(self, out: &mut NumberSelect2) -> Result<(), NwgError> {
        let flags = self.flags.map(|f| f.bits()).unwrap_or(out.flags());
        let (btn_flags, text_flags) = if flags & WS_TABSTOP.0 == WS_TABSTOP.0 {
            (
                ButtonFlags::VISIBLE | ButtonFlags::TAB_STOP,
                TextInputFlags::VISIBLE | TextInputFlags::TAB_STOP,
            )
        } else {
            (ButtonFlags::VISIBLE, TextInputFlags::VISIBLE)
        };

        let parent = match self.parent {
            Some(p) => Ok(p),
            None => Err(NwgError::no_parent("NumberSelect")),
        }?;

        *out = Default::default();

        let (w, h) = self.size;

        if out.handler.is_some() {
            unbind_raw_event_handler(out.handler.as_ref().unwrap())?;
        }

        *out = NumberSelect2::default();
        *out.data.borrow_mut() = self.data;

        out.handle = ControlBase::build_hwnd()
            .class_name(out.class_name())
            .forced_flags(out.forced_flags())
            .ex_flags(WS_EX_CONTROLPARENT.0)
            .flags(flags)
            .size(self.size)
            .position(self.position)
            .parent(Some(parent))
            .build()?;

        TextInput::builder()
            .text(&self.data.formatted_value())
            .size((w - 19, h))
            .parent(out.handle)
            .flags(text_flags)
            .build(&mut out.edit)?;

        Button::builder()
            .text("▴") // Alt: ▲ +
            .size((20, h / 2 + 1))
            .position((w - 20, -1))
            .parent(out.handle)
            .flags(btn_flags)
            .build(&mut out.btn_up)?;

        Button::builder()
            .text("▾") // Alt: ▼ -
            .size((20, h / 2 + 1))
            .position((w - 20, (h / 2) - 1))
            .parent(out.handle)
            .flags(btn_flags)
            .build(&mut out.btn_down)?;

        Notice::builder()
            .parent(out.handle)
            .build(&mut out.notice)?;

        if self.font.is_some() {
            out.btn_up.set_font(self.font);
            out.btn_down.set_font(self.font);
            out.edit.set_font(self.font);
        } else {
            let font = Font::global_default();
            let font_ref = font.as_ref();
            out.btn_up.set_font(font_ref);
            out.btn_down.set_font(font_ref);
            out.edit.set_font(font_ref);
        }

        let plus_button = out.btn_up.handle;
        let minus_button = out.btn_down.handle;
        let text_handle = out.edit.handle;

        let set_text = move |text: &str| {
            let handle = text_handle.hwnd().unwrap();
            unsafe {
                wh::set_window_text(HWND(handle.cast()), text);
            }
        };

        let handler = bind_raw_event_handler(&out.handle, 0xA4545, {
            let notifier = out.notice.sender();
            let handler_data = out.data.clone();
            move |_hwnd, msg, w, l| {
                if WM_COMMAND == msg {
                    let handle = ControlHandle::Hwnd(l as _);
                    let message = w as u32 >> 16;
                    if message == windows::Win32::UI::WindowsAndMessaging::EN_CHANGE {
                        // Corresponds to `nwg::Event::OnTextInput`
                        let handle = text_handle.hwnd().unwrap();
                        let text = unsafe { wh::get_window_text(HWND(handle.cast())) };
                        let mut data = handler_data.borrow_mut();
                        let mut valid = false;
                        match &mut *data {
                            NumberSelectData::Int {
                                value, max, min, ..
                            } => {
                                if let Ok(new) = text.parse::<i64>() {
                                    if *min <= new && new <= *max {
                                        *value = new;
                                        valid = true;
                                    }
                                }
                            }
                            NumberSelectData::Float {
                                value, max, min, ..
                            } => {
                                if let Ok(new) = text.parse::<f64>() {
                                    if *min <= new && new <= *max {
                                        *value = new;
                                        valid = true;
                                    }
                                }
                            }
                        }
                        if valid {
                            drop(data);
                            notifier.notice();
                        } else {
                            let text = data.formatted_value();
                            drop(data);
                            set_text(&text);
                        }
                        return None;
                    }
                    let text = if message == BN_CLICKED && handle == plus_button {
                        let mut data = handler_data.borrow_mut();
                        data.increase();
                        data.formatted_value()
                    } else if message == BN_CLICKED && handle == minus_button {
                        let mut data = handler_data.borrow_mut();
                        data.decrease();
                        data.formatted_value()
                    } else {
                        return None;
                    };
                    set_text(&text);
                    notifier.notice();
                } else if msg == windows::Win32::UI::WindowsAndMessaging::WM_MOUSEWHEEL {
                    let scroll = (w as u32 >> 16) as i16;
                    let mut data = handler_data.borrow_mut();
                    match scroll.cmp(&0) {
                        Ordering::Equal => return None,
                        Ordering::Less => data.decrease(),
                        Ordering::Greater => data.increase(),
                    }
                    let text = data.formatted_value();
                    drop(data);
                    set_text(&text);
                    notifier.notice();
                }
                None
            }
        });
        let edit_handler = bind_raw_event_handler(&out.edit.handle, 0xA4545, {
            let notifier = out.notice.sender();
            let handler_data = out.data.clone();
            move |_hwnd, msg, w, _l| {
                if msg == windows::Win32::UI::WindowsAndMessaging::WM_KEYDOWN {
                    // https://learn.microsoft.com/en-us/windows/win32/inputdev/wm-keydown
                    let keycode = w as u32;
                    let text = if keycode == 38 {
                        let mut data = handler_data.borrow_mut();
                        data.increase();
                        data.formatted_value()
                    } else if keycode == 40 {
                        let mut data = handler_data.borrow_mut();
                        data.decrease();
                        data.formatted_value()
                    } else {
                        return None;
                    };
                    set_text(&text);
                    notifier.notice();
                    // Suppress default action:
                    return Some(0);
                }
                None
            }
        });

        out.handler = Some(handler.unwrap());
        out.edit_handler =
            Some(edit_handler.expect("should create event handler for number select's text box"));

        if !self.enabled {
            out.set_enabled(self.enabled);
        }

        Ok(())
    }
}

/// Adapted from the [`native_windows_gui::controls::handle_from_control`] module.
macro_rules! handles {
    ($control:ty) => {
        #[allow(deprecated)]
        impl From<&$control> for ControlHandle {
            fn from(control: &$control) -> Self {
                control.handle
            }
        }

        #[allow(deprecated)]
        impl From<&mut $control> for ControlHandle {
            fn from(control: &mut $control) -> Self {
                control.handle
            }
        }

        #[allow(deprecated)]
        impl PartialEq<ControlHandle> for $control {
            fn eq(&self, other: &ControlHandle) -> bool {
                self.handle == *other || self.notice.handle == *other
            }
        }

        #[allow(deprecated)]
        impl PartialEq<$control> for ControlHandle {
            fn eq(&self, other: &$control) -> bool {
                *self == other.handle || *self == other.notice.handle
            }
        }
    };
}
handles!(NumberSelect2);
