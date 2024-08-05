//! Monitor and manage virtual desktops. Calls into the [`winvd`] crate, either
//! using static or dynamic calls (using `VirtualDesktopAccessor.dll`).

use std::fmt;

use windows::{core::GUID, Win32::Foundation::HWND};

#[cfg(not(any(feature = "winvd_dynamic", feature = "winvd_static")))]
compile_error!("One of the features 'winvd_dynamic' and 'winvd_static' must be enabled; otherwise the program can't interact with virtual desktops at all.");

pub mod dynamic {
    #![cfg(feature = "winvd_dynamic")]

    use std::{fmt, sync::OnceLock};

    use libloading::{library_filename, Library, Symbol};
    use windows::{core::GUID, Win32::Foundation::HWND};

    static LIBRARY: OnceLock<Result<Library, libloading::Error>> = OnceLock::new();

    /// # Safety
    ///
    /// Must be safe to call `libloading::Library::new` with
    /// "VirtualDesktopAccessor.dll". This means any initialization code in that
    /// dynamic library must be safe to call.
    pub unsafe fn loaded_library() -> Result<&'static Library, &'static libloading::Error> {
        let res = LIBRARY.get_or_init(|| {
            let name = library_filename("VirtualDesktopAccessor");
            unsafe { Library::new(name) }
        });
        match &res {
            Ok(lib) => Ok(lib),
            Err(err) => Err(err),
        }
    }

    static SYMBOLS: OnceLock<Result<VdSymbols<'static>, &'static libloading::Error>> =
        OnceLock::new();

    /// # Safety
    ///
    /// Must be safe to call `libloading::Library::new` with
    /// "VirtualDesktopAccessor.dll". This means any initialization code in that
    /// dynamic library must be safe to call.
    ///
    /// Must also be safe to load the expected symbols from that library, so if
    /// a symbol exists then it must have the correct signature.
    pub unsafe fn loaded_symbols() -> Result<&'static VdSymbols<'static>, &'static libloading::Error>
    {
        let res = SYMBOLS.get_or_init(|| unsafe { Ok(VdSymbols::new(loaded_library()?)) });

        match &res {
            Ok(lib) => Ok(lib),
            Err(err) => Err(err),
        }
    }
    pub fn get_loaded_symbols(
    ) -> Option<Result<&'static VdSymbols<'static>, &'static libloading::Error>> {
        let res = SYMBOLS.get()?;
        Some(match &res {
            Ok(lib) => Ok(lib),
            Err(err) => Err(err),
        })
    }

    trait CheckError {
        fn is_error(&self) -> bool;
    }
    impl CheckError for i32 {
        fn is_error(&self) -> bool {
            *self == -1
        }
    }
    impl CheckError for GUID {
        fn is_error(&self) -> bool {
            *self == GUID::default()
        }
    }

    macro_rules! define_symbols {
        (
            $(
                $(#[no_error $(@ $no_error:tt)?])?
                $(#[optional $(@ $optional:tt)?])?
                $(unsafe $(@ $unsafe:tt)?)? fn $name:ident($($arg:ident: $t:ty),* $(,)?) -> $ret:ty {}
            )*
        ) => {
            #[derive(Debug, Clone)]
            pub enum DynamicError {
                $(
                    #[allow(dead_code)]
                    $name { $($arg: $t,)* },
                )*
                Missing(&'static str),
            }
            impl fmt::Display for DynamicError {
                fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    match self {
                        $(DynamicError::$name { $($arg,)* } => write!(
                            f, concat!(
                                "Failed to call \"{fn_name}\" in dynamic library with arguments [",
                                $(stringify!($arg), ": {:?}, ",)*
                                "]"
                            ),
                            $($arg,)*
                            fn_name = stringify!($name),
                        ),)*
                        DynamicError::Missing(method) => write!(f, "The method \"{method}\" was not found in the dynamic library"),
                    }
                }
            }

            #[allow(non_snake_case, dead_code)]
            pub struct VdSymbols<'lib> {
                $(
                    pub $name: Result<Symbol<'lib, $(unsafe $($unsafe)?)? extern "C" fn($($t),*) -> $ret>, libloading::Error>,
                )*
            }
            impl<'lib> VdSymbols<'lib> {
                /// Load symbols from the specified library.
                ///
                /// # Safety
                ///
                /// The symbol names must have the correct function signatures.
                pub unsafe fn new(lib: &'lib Library) -> Self {
                    Self {
                        $(
                            // Safety: the caller assures us that the symbol
                            // name matches the expected function signature
                            $name: unsafe {
                                lib.get(concat!(stringify!($name), "\0").as_bytes())
                            },
                        )*
                    }
                }
                pub fn ensure_required_methods_exists(&self) -> Result<(), DynamicError> {
                    $(
                        #[cfg(all($(any( $($optional)? ))?))]
                        if let Err(_) = &self.$name {
                            return Err(DynamicError::Missing(stringify!($name)));
                        }
                    )*
                    Ok(())
                }

                $(
                    /// Call the specified function in the dynamic library.
                    ///
                    /// # Safety
                    ///
                    /// Depends on the function. Likely any pointers must be
                    /// valid.
                    #[allow(non_snake_case)]
                    pub $(unsafe $($unsafe)?)? fn $name(&self, $($arg: $t),*) -> Result<$ret, DynamicError> {
                        tracing::trace!("Dynamic library call to {}", stringify!($name));
                        let sym = match &self.$name {
                            Ok(sym) => sym,
                            Err(_) => return Err(DynamicError::Missing(stringify!($name))),
                        };
                        let res = sym($($arg),*);
                        #[cfg(all($(any( $($no_error)? ))?))]
                        {
                            if CheckError::is_error(&res) {
                                return Err(DynamicError::$name { $($arg,)* })
                            }
                        }
                        Ok(res)
                    }
                )*
            }
        };
    }
    // These names were copied from:
    // https://github.com/Ciantic/VirtualDesktopAccessor/blob/126b9e04f4f01d434af06c20d8200d0659547774/README.md#reference-of-exported-dll-functions
    define_symbols!(
        fn GetCurrentDesktopNumber() -> i32 {}
        fn GetDesktopCount() -> i32 {}
        fn GetDesktopIdByNumber(number: i32) -> GUID {} // Untested
        fn GetDesktopNumberById(desktop_id: GUID) -> i32 {} // Untested
        fn GetWindowDesktopId(hwnd: HWND) -> GUID {}
        fn GetWindowDesktopNumber(hwnd: HWND) -> i32 {}
        fn IsWindowOnCurrentVirtualDesktop(hwnd: HWND) -> i32 {}
        fn MoveWindowToDesktopNumber(hwnd: HWND, desktop_number: i32) -> i32 {}
        fn GoToDesktopNumber(desktop_number: i32) -> i32 {}
        #[optional] // Win11 only
        unsafe fn SetDesktopName(desktop_number: i32, in_name_ptr: *const i8) -> i32 {}
        #[optional] // Win11 only
        unsafe fn GetDesktopName(
            desktop_number: i32,
            out_utf8_ptr: *mut u8,
            out_utf8_len: usize,
        ) -> i32 {
        }
        unsafe fn RegisterPostMessageHook(listener_hwnd: HWND, message_offset: u32) -> i32 {}
        unsafe fn UnregisterPostMessageHook(listener_hwnd: HWND) -> i32 {}
        fn IsPinnedWindow(hwnd: HWND) -> i32 {}
        fn PinWindow(hwnd: HWND) -> i32 {}
        fn UnPinWindow(hwnd: HWND) -> i32 {}
        fn IsPinnedApp(hwnd: HWND) -> i32 {}
        fn PinApp(hwnd: HWND) -> i32 {}
        fn UnPinApp(hwnd: HWND) -> i32 {}
        fn IsWindowOnDesktopNumber(hwnd: HWND, desktop_number: i32) -> i32 {}
        #[optional] // Win11 only
        fn CreateDesktop() -> i32 {}
        #[optional] // Win11 only
        fn RemoveDesktop(remove_desktop_number: i32, fallback_desktop_number: i32) -> i32 {}
    );

    impl From<DynamicError> for super::Error {
        fn from(err: DynamicError) -> Self {
            Self::DynamicCall(err)
        }
    }
}

/// Wrapper around [`winvd::Desktop`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Desktop {
    #[cfg(feature = "winvd_static")]
    Static(winvd::Desktop),
    Index(u32),
    Guid(GUID),
}
impl Desktop {
    pub fn get_index(&self) -> Result<u32, Error> {
        match self {
            #[cfg(feature = "winvd_static")]
            Self::Static(d) => Ok(d.get_index()?),
            Self::Index(i) => Ok(*i),
            Self::Guid(guid) => {
                #[cfg(feature = "winvd_dynamic")]
                if let Some(Ok(symbols)) = dynamic::get_loaded_symbols() {
                    return Ok(symbols.GetDesktopNumberById(*guid)? as u32);
                }
                #[cfg(feature = "winvd_static")]
                {
                    return Ok(winvd::get_desktop(*guid).get_index()?);
                }
                #[allow(unreachable_code)]
                {
                    Err(no_dynamic_library_error())
                }
            }
        }
    }
    pub fn get_name(&self) -> Result<String, Error> {
        match self {
            #[cfg(feature = "winvd_static")]
            Self::Static(d) => Ok(d.get_name()?),
            _ => {
                #[cfg(feature = "winvd_dynamic")]
                if let Some(Ok(symbols)) = dynamic::get_loaded_symbols() {
                    let mut buf = vec![0u8; 256];
                    let desktop_number = self.get_index()? as i32;
                    let out_utf8_len = buf.len();
                    let out_utf8_ptr = buf.as_mut_ptr();
                    // res is -1 if len was to short.
                    let res = unsafe {
                        symbols.GetDesktopName(desktop_number, out_utf8_ptr, out_utf8_len)?
                    };
                    if res == 0 {
                        // winvd::Desktop::get_name returned an error
                        return Err(Error::DynamicCall(dynamic::DynamicError::GetDesktopName {
                            desktop_number,
                            out_utf8_ptr,
                            out_utf8_len,
                        }));
                    }
                    // find first nul byte:
                    if let Some(first_nul) = buf.iter().position(|&byte| byte == b'\0') {
                        buf.truncate(first_nul + 1);
                    }
                    let mut name = std::ffi::CString::from_vec_with_nul(buf)
                        .map_err(|_| Error::DesktopNameWithoutNul)?
                        .into_string()
                        .map_err(|e| {
                            Error::NonUtf8DesktopName(
                                String::from_utf8_lossy(e.into_cstring().as_bytes()).into_owned(),
                            )
                        })?;
                    name.shrink_to_fit();
                    return Ok(name);
                }
                #[cfg(feature = "winvd_static")]
                {
                    return Ok(winvd::Desktop::from(*self).get_name()?);
                }
                #[allow(unreachable_code)]
                {
                    Err(no_dynamic_library_error())
                }
            }
        }
    }
}
#[cfg(feature = "winvd_static")]
impl From<winvd::Desktop> for Desktop {
    fn from(d: winvd::Desktop) -> Self {
        Self::Static(d)
    }
}
#[cfg(feature = "winvd_static")]
impl From<Desktop> for winvd::Desktop {
    fn from(d: Desktop) -> Self {
        match d {
            Desktop::Static(d) => d,
            Desktop::Index(i) => winvd::get_desktop(i),
            Desktop::Guid(g) => winvd::get_desktop(g),
        }
    }
}
impl From<u32> for Desktop {
    fn from(i: u32) -> Self {
        Self::Index(i)
    }
}
impl From<i32> for Desktop {
    fn from(i: i32) -> Self {
        Self::Index(i as u32)
    }
}
impl From<GUID> for Desktop {
    fn from(g: GUID) -> Self {
        Self::Guid(g)
    }
}

/// Get desktop by index or GUID (Same as [`winvd::get_desktop`]).
///
/// # Examples
/// * Get first desktop by index `get_desktop(0)`
/// * Get second desktop by index `get_desktop(1)`
/// * Get desktop by GUID `get_desktop(GUID(0, 0, 0, [0, 0, 0, 0, 0, 0, 0, 0]))`
///
/// Note: This function does not check if the desktop exists.
pub fn get_desktop<T>(desktop: T) -> Desktop
where
    T: Into<Desktop>,
{
    desktop.into()
}

/// Same as [`winvd::DesktopEvent`]
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum DesktopEvent {
    DesktopCreated(Desktop),
    DesktopDestroyed {
        destroyed: Desktop,
        fallback: Desktop,
    },
    DesktopChanged {
        new: Desktop,
        old: Desktop,
    },
    DesktopNameChanged(Desktop, String),
    DesktopWallpaperChanged(Desktop, String),
    DesktopMoved {
        desktop: Desktop,
        old_index: i64,
        new_index: i64,
    },
    WindowChanged(HWND),
}
#[cfg(feature = "winvd_static")]
impl From<winvd::DesktopEvent> for DesktopEvent {
    fn from(event: winvd::DesktopEvent) -> Self {
        match event {
            winvd::DesktopEvent::DesktopCreated(d) => Self::DesktopCreated(d.into()),
            winvd::DesktopEvent::DesktopDestroyed {
                destroyed,
                fallback,
            } => Self::DesktopDestroyed {
                destroyed: destroyed.into(),
                fallback: fallback.into(),
            },
            winvd::DesktopEvent::DesktopChanged { new, old } => Self::DesktopChanged {
                new: new.into(),
                old: old.into(),
            },
            winvd::DesktopEvent::DesktopNameChanged(d, name) => {
                Self::DesktopNameChanged(d.into(), name)
            }
            winvd::DesktopEvent::DesktopWallpaperChanged(d, path) => {
                Self::DesktopWallpaperChanged(d.into(), path)
            }
            winvd::DesktopEvent::DesktopMoved {
                desktop,
                old_index,
                new_index,
            } => Self::DesktopMoved {
                desktop: desktop.into(),
                old_index,
                new_index,
            },
            winvd::DesktopEvent::WindowChanged(hwnd) => Self::WindowChanged(hwnd),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Error {
    #[cfg(feature = "winvd_dynamic")]
    DynamicCall(dynamic::DynamicError),
    #[cfg(feature = "winvd_dynamic")]
    FailedToLoadDynamicLibrary(&'static libloading::Error),
    /// Need to call `load_dynamic_library`.
    NotLoadedDynamicLibrary,
    #[cfg(feature = "winvd_static")]
    StaticCall(winvd::Error),
    NonUtf8DesktopName(String),
    DesktopNameWithoutNul,
}
#[cfg(feature = "winvd_static")]
impl From<winvd::Error> for Error {
    fn from(value: winvd::Error) -> Self {
        Self::StaticCall(value)
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            #[cfg(feature = "winvd_dynamic")]
            Self::DynamicCall(err) => fmt::Display::fmt(err, f),
            #[cfg(feature = "winvd_dynamic")]
            Self::FailedToLoadDynamicLibrary(err) => {
                write!(f, "Failed to load dynamic library VirtualDesktopAccessor.dll: {err}")
            }
            Self::NotLoadedDynamicLibrary => write!(
                f,
                "Tried to call a virtual desktop function before loading the dynamic library VirtualDesktopAccessor.dll"
            ),
            #[cfg(feature = "winvd_static")]
            Self::StaticCall(err) => write!(
                f,
                "Failed to call virtual desktop function in static library: {err:?}"
            ),
            Self::NonUtf8DesktopName(name) => write!(f, "Non-UTF8 desktop name: {name}"),
            Self::DesktopNameWithoutNul => write!(f, "Invalid virtual desktop name"),
        }
    }
}
impl std::error::Error for Error {}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Load the dynamic library "VirtualDesktopAccessor.dll".
///
/// # Errors
///
/// If the dynamic library wasn't loaded and **no** static library was included
/// when the executable was built. The program can then not interact with
/// virtual desktops and should probably exit.
///
/// # Safety
///
/// Must be safe to call `libloading::Library::new` with
/// "VirtualDesktopAccessor.dll". This means any initialization code in that
/// dynamic library must be safe to call.
///
/// Must also be safe to load the expected symbols from that library, so if a
/// symbol exists then it must have the correct signature.
///
/// tl;dr; If a "VirtualDesktopAccessor.dll" file exists then it has a correct
/// implementation.
pub unsafe fn load_dynamic_library() -> Result<(), Error> {
    #[cfg(feature = "winvd_dynamic")]
    {
        let res = unsafe { dynamic::loaded_symbols() };
        let res = match res {
            Err(e) => {
                tracing::warn!("Failed to load VirtualDesktopAccessor.dll: {e}");
                Err(Error::FailedToLoadDynamicLibrary(e))
            }
            Ok(symbols) => {
                if let Err(e) = symbols.ensure_required_methods_exists() {
                    tracing::error!("Failed to load VirtualDesktopAccessor.dll: {e}");
                    Err(Error::DynamicCall(e))
                } else {
                    tracing::info!("Successfully loaded VirtualDesktopAccessor.dll");
                    Ok(())
                }
            }
        };
        if cfg!(feature = "winvd_static") {
            // Fallback to static library included in the executable
            Ok(())
        } else {
            res
        }
    }
    #[cfg(not(feature = "winvd_dynamic"))]
    {
        Ok(())
    }
}

pub fn has_loaded_dynamic_library_successfully() -> bool {
    #[cfg(feature = "winvd_dynamic")]
    {
        matches!(dynamic::get_loaded_symbols(), Some(Ok(symbols)) if symbols.ensure_required_methods_exists().is_ok())
    }
    #[cfg(not(feature = "winvd_dynamic"))]
    {
        false
    }
}

/// Returns an error that indicates why no dynamic library was loaded.
///
/// # Panics
///
/// If the dynamic library was loaded.
fn no_dynamic_library_error() -> Error {
    #[cfg(feature = "winvd_dynamic")]
    {
        match dynamic::get_loaded_symbols() {
            Some(Err(e)) => return Error::FailedToLoadDynamicLibrary(e),
            Some(Ok(_)) => panic!("Should have called the loaded function instead of reporting that no library was loaded"),
            None => (),
        }
    }
    Error::NotLoadedDynamicLibrary
}

/// Wrapper around [`winvd::get_desktop_count`] (but prefers dynamic loaded
/// library if it exists).
pub fn get_desktop_count() -> Result<u32> {
    #[cfg(feature = "winvd_dynamic")]
    {
        if let Some(Ok(symbols)) = dynamic::get_loaded_symbols() {
            return Ok(symbols.GetDesktopCount()? as u32);
        }
    }
    #[cfg(feature = "winvd_static")]
    {
        return Ok(winvd::get_desktop_count()?);
    }
    #[allow(unreachable_code)]
    Err(no_dynamic_library_error())
}

/// Wrapper around [`winvd::get_current_desktop`] (but prefers dynamic loaded
/// library if it exists).
pub fn get_current_desktop() -> Result<Desktop> {
    #[cfg(feature = "winvd_dynamic")]
    {
        if let Some(Ok(symbols)) = dynamic::get_loaded_symbols() {
            return Ok(Desktop::Index(symbols.GetCurrentDesktopNumber()? as u32));
        }
    }
    #[cfg(feature = "winvd_static")]
    {
        return Ok(winvd::get_current_desktop()?.into());
    }
    #[allow(unreachable_code)]
    Err(no_dynamic_library_error())
}

/// Wrapper around [`winvd::move_window_to_desktop`] (but prefers dynamic loaded
/// library if it exists).
pub fn move_window_to_desktop(desktop: Desktop, hwnd: &HWND) -> Result<()> {
    #[cfg(feature = "winvd_dynamic")]
    {
        if let Some(Ok(symbols)) = dynamic::get_loaded_symbols() {
            symbols.MoveWindowToDesktopNumber(*hwnd, desktop.get_index()? as i32)?;
            return Ok(());
        }
    }
    #[cfg(feature = "winvd_static")]
    {
        winvd::move_window_to_desktop(winvd::Desktop::from(desktop), hwnd)?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err(no_dynamic_library_error())
}

/// Wrapper around [`winvd::pin_window`] (but prefers dynamic loaded library if
/// it exists).
pub fn pin_window(hwnd: HWND) -> Result<()> {
    #[cfg(feature = "winvd_dynamic")]
    {
        if let Some(Ok(symbols)) = dynamic::get_loaded_symbols() {
            symbols.PinWindow(hwnd)?;
            return Ok(());
        }
    }
    #[cfg(feature = "winvd_static")]
    {
        winvd::pin_window(hwnd)?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err(no_dynamic_library_error())
}

/// Wrapper around [`winvd::unpin_window`] (but prefers dynamic loaded library
/// if it exists).
pub fn unpin_window(hwnd: HWND) -> Result<()> {
    #[cfg(feature = "winvd_dynamic")]
    {
        if let Some(Ok(symbols)) = dynamic::get_loaded_symbols() {
            symbols.UnPinWindow(hwnd)?;
            return Ok(());
        }
    }
    #[cfg(feature = "winvd_static")]
    {
        winvd::unpin_window(hwnd)?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err(no_dynamic_library_error())
}

/// Wrapper around [`winvd::switch_desktop`] (but prefers dynamic loaded
/// library if it exists).
pub fn switch_desktop(desktop: Desktop) -> Result<()> {
    #[cfg(feature = "winvd_dynamic")]
    {
        if let Some(Ok(symbols)) = dynamic::get_loaded_symbols() {
            symbols.GoToDesktopNumber(desktop.get_index()? as i32)?;
            return Ok(());
        }
    }
    #[cfg(feature = "winvd_static")]
    {
        winvd::switch_desktop(winvd::Desktop::from(desktop))?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err(no_dynamic_library_error())
}

/// Wrapper around [`winvd::remove_desktop`] (but prefers dynamic loaded
/// library if it exists).
pub fn remove_desktop(desktop: Desktop, fallback_desktop: Desktop) -> Result<()> {
    #[cfg(feature = "winvd_dynamic")]
    {
        if let Some(Ok(symbols)) = dynamic::get_loaded_symbols() {
            symbols.RemoveDesktop(
                desktop.get_index()? as i32,
                fallback_desktop.get_index()? as i32,
            )?;
            return Ok(());
        }
    }
    #[cfg(feature = "winvd_static")]
    {
        winvd::remove_desktop(
            winvd::Desktop::from(desktop),
            winvd::Desktop::from(fallback_desktop),
        )?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err(no_dynamic_library_error())
}

/// Wrapper around [`winvd::create_desktop`] (but prefers dynamic loaded
/// library if it exists).
pub fn create_desktop() -> Result<Desktop> {
    #[cfg(feature = "winvd_dynamic")]
    {
        if let Some(Ok(symbols)) = dynamic::get_loaded_symbols() {
            return Ok(Desktop::Index(symbols.CreateDesktop()? as u32));
        }
    }
    #[cfg(feature = "winvd_static")]
    {
        return Ok(Desktop::Static(winvd::create_desktop()?));
    }
    #[allow(unreachable_code)]
    Err(no_dynamic_library_error())
}

/// Wrapper around [`winvd::get_desktops`] (but prefers dynamic loaded
/// library if it exists).
pub fn get_desktops() -> Result<Vec<Desktop>> {
    #[cfg(feature = "winvd_dynamic")]
    {
        if let Some(Ok(symbols)) = dynamic::get_loaded_symbols() {
            return Ok((0..symbols.GetDesktopCount()?)
                .map(|i| Desktop::Index(i as u32))
                .collect());
        }
    }
    #[cfg(feature = "winvd_static")]
    {
        return Ok(winvd::get_desktops()?
            .into_iter()
            .map(Desktop::from)
            .collect());
    }
    #[allow(unreachable_code)]
    Err(no_dynamic_library_error())
}

pub fn get_window_desktop(hwnd: HWND) -> Result<Desktop> {
    #[cfg(feature = "winvd_dynamic")]
    {
        if let Some(Ok(symbols)) = dynamic::get_loaded_symbols() {
            return Ok(Desktop::Guid(symbols.GetWindowDesktopId(hwnd)?));
        }
    }
    #[cfg(feature = "winvd_static")]
    {
        return Ok(winvd::get_desktop_by_window(hwnd)?.into());
    }
    #[allow(unreachable_code)]
    Err(no_dynamic_library_error())
}

pub fn is_pinned_window(hwnd: HWND) -> Result<bool> {
    #[cfg(feature = "winvd_dynamic")]
    {
        if let Some(Ok(symbols)) = dynamic::get_loaded_symbols() {
            return Ok(symbols.IsPinnedWindow(hwnd)? != 0);
        }
    }
    #[cfg(feature = "winvd_static")]
    {
        return Ok(winvd::is_pinned_window(hwnd)?);
    }
    #[allow(unreachable_code)]
    Err(no_dynamic_library_error())
}

pub fn is_pinned_app(hwnd: HWND) -> Result<bool> {
    #[cfg(feature = "winvd_dynamic")]
    {
        if let Some(Ok(symbols)) = dynamic::get_loaded_symbols() {
            return Ok(symbols.IsPinnedApp(hwnd)? != 0);
        }
    }
    #[cfg(feature = "winvd_static")]
    {
        return Ok(winvd::is_pinned_app(hwnd)?);
    }
    #[allow(unreachable_code)]
    Err(no_dynamic_library_error())
}

/// Start flashing a window's icon in the taskbar.
pub fn start_flashing_window(hwnd: HWND) {
    use windows::Win32::UI::WindowsAndMessaging::{
        FlashWindowEx, FLASHWINFO, FLASHW_TIMERNOFG, FLASHW_TRAY,
    };

    let info = FLASHWINFO {
        cbSize: std::mem::size_of::<FLASHWINFO>() as u32,
        // A handle to the window to be flashed. The window can be either opened or minimized.
        hwnd,
        dwFlags: FLASHW_TIMERNOFG | FLASHW_TRAY,
        // The number of times to flash the window.
        uCount: 0,
        // The rate at which the window is to be flashed, in milliseconds. If zero, the function uses the default cursor blink rate.
        dwTimeout: 0,
    };
    // The return value specifies the window's state before the new flash
    // information is applied. If the window caption/title was drawn as active
    // before the call, the return value is true. Otherwise, the return value is
    // false.
    let _ = unsafe { FlashWindowEx(&info) };
}

/// Calls [`stop_flashing_window`] using the simple async runtime provided by
/// [`crate::block_on`].
///
/// # Cancellation
///
/// If the program exits before this function completes then some windows might
/// remain hidden and never become visible again.
pub fn stop_flashing_windows_blocking(
    windows: Vec<(HWND, Option<Desktop>)>,
) -> Result<(), Box<dyn std::error::Error>> {
    tracing::debug!(?windows, "stop_flashing_windows_blocking");
    if windows.is_empty() {
        return Ok(());
    }
    let error = std::cell::OnceCell::new();
    crate::block_on::block_on(crate::block_on::simple_join(windows.into_iter().map(
        |(hwnd, target)| {
            let error = &error;
            async move {
                if let Err(e) = stop_flashing_window(hwnd, target).await {
                    let _ = error.set(e);
                }
            }
        },
    )));
    if let Some(e) = error.into_inner() {
        Err(e)
    } else {
        Ok(())
    }
}

/// Stop a window from flashing orange in the Windows taskbar.
///
/// # Timeline
///
/// 1. Stop the flashing, but the taskbar icon might remain visible even when
///    the window isn't on that virtual desktop.
/// 2. Hide the window so that the taskbar icon is removed.
/// 3. Show the window again, this will move the window to the current virtual
///    desktop.
/// 4. Move the window to the target desktop.
///
/// # Cancellation
///
/// If the program exits before this future completes or is canceled then some
/// windows might remain hidden and never become visible again.
pub async fn stop_flashing_window(
    hwnd: HWND,
    target_desktop: Option<Desktop>,
) -> Result<(), Box<dyn std::error::Error>> {
    use crate::nwg_ext::TimerThread;
    use std::time::{Duration, Instant};
    use windows::Win32::UI::WindowsAndMessaging::{
        FlashWindowEx, GetWindowInfo, ShowWindow, FLASHWINFO, FLASHW_STOP, SW_HIDE, SW_SHOWNA,
        WINDOWINFO, WS_VISIBLE,
    };

    // This move might be canceled by later operations but that might take a
    // while so move window to give the user immediate feedback:
    if let Some(target_desktop) = target_desktop {
        move_window_to_desktop(target_desktop, &hwnd)?;
    };

    // Stop Taskbar Icon Flashing:

    // Flashes the specified window. It does not change the active state of the
    // window.
    let info = FLASHWINFO {
        cbSize: std::mem::size_of::<FLASHWINFO>() as u32,
        // A handle to the window to be flashed. The window can be either opened or minimized.
        hwnd,
        dwFlags: FLASHW_STOP,
        // The number of times to flash the window.
        uCount: 0,
        // The rate at which the window is to be flashed, in milliseconds. If zero, the function uses the default cursor blink rate.
        dwTimeout: 0,
    };
    // The return value specifies the window's state before the new flash
    // information is applied. If the window caption/title was drawn as active
    // before the call, the return value is true. Otherwise, the return value is
    // false.
    let _ = unsafe { FlashWindowEx(&info) };

    // Hide window and then show it again (fixes always visible taskbar icons):
    let was_visible;
    {
        /// Safeguard to make absolutely sure the window is shown again.
        struct ShowGuard {
            hwnd: Option<HWND>,
            hidden_at: Instant,
        }
        impl Drop for ShowGuard {
            fn drop(&mut self) {
                let Some(hwnd) = self.hwnd else {
                    return;
                };
                let wake_at = self.hidden_at + Duration::from_millis(1000);
                let wait = wake_at.saturating_duration_since(Instant::now());
                if !wait.is_zero() {
                    std::thread::sleep(wait);
                }
                let _ = unsafe { ShowWindow(hwnd, SW_SHOWNA) };
            }
        }
        let mut show_guard = ShowGuard {
            hwnd: Some(hwnd),
            hidden_at: Instant::now(),
        };

        // After sending flash stop: wait for flashing to stop otherwise it is reapplied when window is shown.
        TimerThread::get_global()
            .delay_future(Duration::from_millis(1000))
            .await;

        // Hide (and Later Show) to update taskbar visibility (fixes always visible taskbar icons):
        was_visible = unsafe { ShowWindow(hwnd, SW_HIDE) }.as_bool();
        if was_visible {
            // Wait needed before showing window again to stop flashing windows:
            // Wait time Minimum: 30ms is quite relaible. Under 20ms nearly always fails.
            // 100ms can fail if system is under heavy load.

            // Wait for window to become hidden:
            let retry_times = [
                // Delay,  Total Wait
                100,    // 100    ms
                400,    // 500    ms
                500,    // 1_000   ms
                1_000,  // 2_000   ms
                3_000,  // 5_000   ms
                5_000,  // 10_000  ms
                5_000,  // 15_000  ms
                5_000,  // 20_000  ms
                10_000, // 30_000  ms
                30_000, // 60_000  ms
                        //60_000,      // 120_000 ms
                        //120_000,     // 240_000 ms
                        //120_000,     // 360_000 ms
            ]
            .map(Duration::from_millis);
            for time in retry_times {
                TimerThread::get_global().delay_future(time).await;

                let mut info = WINDOWINFO {
                    cbSize: std::mem::size_of::<WINDOWINFO>() as u32,
                    ..Default::default()
                };
                if unsafe { GetWindowInfo(hwnd, &mut info) }.is_ok()
                    && (info.dwStyle.0 & WS_VISIBLE.0 == 0)
                {
                    // Window is hidden
                    break;
                }
            }

            // Then re-show it:
            let _ = unsafe { ShowWindow(hwnd, SW_SHOWNA) };

            // Cancel show safeguard:
            show_guard.hwnd = None;
        }
    }

    // Move (back) to wanted virtual desktop:
    {
        if !was_visible {
            // Can't move a hidden window to another virtual desktop.
            return Ok(());
        }
        let Some(target_desktop) = target_desktop else {
            // Leave the window at the current virtual desktop.
            return Ok(());
        };

        // After hiding and then showing the window it can either be visible on the
        // taskbar for all virtual desktops or the window might have been moved to
        // the current desktop.
        //
        // Reapply virtual desktop info to ensure it is moved to the right place:

        // Note that too many attempts will cause windows taskbar and virtual
        // desktop switching to slow down and maybe freeze.
        // - 1000 attempts per window for 15 windows will causes explorer to freeze.
        // - 100 attempts per window for 15 windows will cause a slight slowdown.
        // - On newer Windows versions this has gotten dramatically slower.

        let retry_times = [
            // Delay, Total Wait
            0,   // 0 ms   if there is no lag then this might actually work.
            25,  // 25 ms  20% of windows are shown before 25 ms.
            25,  // 50 ms  75% of windows are shown before 50 ms.
            50,  // 100 ms
            400, // 500 ms
        ]
        .map(Duration::from_millis);

        for time in retry_times {
            if !time.is_zero() {
                // Note: could use std::thread::sleep for times that are less than 50ms...
                TimerThread::get_global().delay_future(time).await;
            }

            let Ok(current) = get_window_desktop(hwnd) else {
                // Not shown yet...
                continue;
            };
            if current == target_desktop {
                // Is at the right place!
                break;
            }
            // For some of these move attempts the window might still be hidden and
            // so impossible to move:
            let _ = move_window_to_desktop(target_desktop, &hwnd);
        }
    }

    Ok(())
}
