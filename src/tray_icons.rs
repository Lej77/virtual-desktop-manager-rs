//! Generate tray icons as needed.
//!
//! # References
//!
//! The idea for this feature came from other projects that use the tray icon to
//! display the current virtual desktop index:
//! - [m0ngr31/VirtualDesktopManager](https://github.com/m0ngr31/VirtualDesktopManager)
//!   - This is also where the icons used in this project was copied from.
//!   - Creates new icons on demand to support up to 999 desktops.
//! - [mzomparelli/zVirtualDesktop: Windows 10 Virtual Desktop Hotkeys, System
//!   Tray Icon, Wallpapers, and Task View
//!   replacement](https://github.com/mzomparelli/zVirtualDesktop?tab=readme-ov-file)
//!   - Hard coded to 9 images?
//! - [lutz/VirtualDesktopNameDeskband: Deskband for the windows taskbar to show
//!   the name of current virtual
//!   desktop](https://github.com/lutz/VirtualDesktopNameDeskband)
//!   - Shows the desktop index in a "deskband", a small widget shown next to
//!     the taskbar (next to where windows 10 would show the weather).
//! - [dankrusi/WindowsVirtualDesktopHelper: App to help manage Virtual Desktops
//!   for Windows 10 and Windows
//!   11](https://github.com/dankrusi/WindowsVirtualDesktopHelper)
//!   - This generates icons on demand and so can likely support "infinitely"
//!     many desktops.
//! - [sdias/win-10-virtual-desktop-enhancer: An application that enhances the
//!   Windows 10 multiple desktops feature by adding additional keyboard
//!   shortcuts and support for multiple
//!   wallpapers.](https://github.com/sdias/win-10-virtual-desktop-enhancer)
//!   - Supports icon packs for specifying tray icon on different desktops.

#![allow(dead_code)]

#[allow(unused_imports)]
use std::{borrow::Cow, io::Cursor, sync::OnceLock};

#[cfg(feature = "tray_icon_hardcoded")]
mod hardcoded {
    pub static ICON1: &[u8] = include_bytes!("icons/triangle1.ico");
    pub static ICON2: &[u8] = include_bytes!("icons/triangle2.ico");
    pub static ICON3: &[u8] = include_bytes!("icons/triangle3.ico");
    pub static ICON4: &[u8] = include_bytes!("icons/triangle4.ico");
    pub static ICON5: &[u8] = include_bytes!("icons/triangle5.ico");
    pub static ICON6: &[u8] = include_bytes!("icons/triangle6.ico");
    pub static ICON7: &[u8] = include_bytes!("icons/triangle7.ico");
    pub static ICON8: &[u8] = include_bytes!("icons/triangle8.ico");
    pub static ICON9: &[u8] = include_bytes!("icons/triangle9.ico");
}
#[cfg(feature = "tray_icon_hardcoded")]
pub use hardcoded::*;

pub static ICON_EMPTY: &[u8] = include_bytes!("icons/triangleEmpty.ico");
#[cfg(feature = "tray_icon_with_background")]
pub static IMAGE_EMPTY: &[u8] = include_bytes!("icons/triangleEmptyImage.png");

/// This font is only guaranteed to work on numbers.
///
/// # Font
///
/// - [Open Sans - Google Fonts](https://fonts.google.com/specimen/Open+Sans/about)
///   - Links to: <https://github.com/googlefonts/opensans>
///   - License:  Open Font License.
/// - Alternatively we could use [`DejaVuSans.ttf`](https://github.com/image-rs/imageproc/blob/4e6a5dc65485cd58c74f1d120657676831106c57/examples/DejaVuSans.ttf)
/// - The `text-to-png` includes a font if you don't chose one yourself, we could use that one.
///
/// # Minimize size
///
/// We only need to render digits so we remove unnecessary stuff, see:
/// [filesize - Way to reduce size of .ttf fonts? - Stack Overflow](https://stackoverflow.com/questions/2635423/way-to-reduce-size-of-ttf-fonts)
#[cfg(any(
    feature = "tray_icon_with_background",
    feature = "tray_icon_text_only",
    feature = "tray_icon_text_only_alt"
))]
static NUMBER_FONT: &[u8] = include_bytes!("./OpenSans-Bold-DigitsOnly.ttf");

pub fn get_included_icon(_number: u32) -> Option<&'static [u8]> {
    #[cfg(feature = "tray_icon_hardcoded")]
    {
        Some(match _number {
            1 => ICON1,
            2 => ICON2,
            3 => ICON3,
            4 => ICON4,
            5 => ICON5,
            6 => ICON6,
            7 => ICON7,
            8 => ICON8,
            9 => ICON9,
            _ => return None,
        })
    }
    #[cfg(not(feature = "tray_icon_hardcoded"))]
    {
        None
    }
}

pub enum IconType {
    WithBackground {
        allow_hardcoded: bool,
        light_theme: bool,
    },
    NoBackground {
        light_theme: bool,
    },
    NoBackgroundAlt,
}
impl IconType {
    // TODO: maybe return errors from this in case image generation fails.
    pub fn generate_icon(&self, number: u32) -> Cow<'static, [u8]> {
        match self {
            // TODO: support light theme with hardcoded icons
            Self::WithBackground {
                allow_hardcoded: true,
                light_theme,
            } => match get_included_icon(number).filter(|_| !light_theme) {
                Some(d) => Cow::Borrowed(d),
                #[cfg(feature = "tray_icon_with_background")]
                None => Cow::Owned(generate_icon_with_background(number, *light_theme)),
                #[cfg(not(feature = "tray_icon_with_background"))]
                None => Cow::Borrowed(ICON_EMPTY),
            },
            Self::WithBackground {
                allow_hardcoded: false,
                light_theme,
            } => {
                #[cfg(feature = "tray_icon_with_background")]
                {
                    generate_icon_with_background(number, *light_theme).into()
                }
                #[cfg(not(feature = "tray_icon_with_background"))]
                {
                    Cow::Borrowed(ICON_EMPTY)
                }
            }
            Self::NoBackground { light_theme } => {
                #[cfg(feature = "tray_icon_text_only")]
                {
                    generate_icon_without_background(number, *light_theme).into()
                }
                #[cfg(not(feature = "tray_icon_text_only"))]
                {
                    Cow::Borrowed(ICON_EMPTY)
                }
            }
            Self::NoBackgroundAlt => {
                #[cfg(feature = "tray_icon_text_only_alt")]
                {
                    generate_icon_without_background_alt(number).into()
                }
                #[cfg(not(feature = "tray_icon_text_only_alt"))]
                {
                    Cow::Borrowed(ICON_EMPTY)
                }
            }
        }
    }
}

#[cfg(any(feature = "tray_icon_with_background", feature = "tray_icon_text_only"))]
fn get_number_font() -> &'static ab_glyph::FontRef<'static> {
    static CACHED: OnceLock<ab_glyph::FontRef<'static>> = OnceLock::new();
    CACHED.get_or_init(|| {
        ab_glyph::FontRef::try_from_slice(NUMBER_FONT).expect("Valid font embedded in binary")
    })
}

#[cfg(feature = "tray_icon_with_background")]
fn get_empty_image() -> &'static image::DynamicImage {
    static CACHED: OnceLock<image::DynamicImage> = OnceLock::new();
    CACHED.get_or_init(|| {
        let image =
            image::io::Reader::with_format(Cursor::new(IMAGE_EMPTY), image::ImageFormat::Png)
                .decode()
                .expect("Failed to load embedded PNG");
        // Embedded image is 258 pixels but the `image` crate can only convert
        // to ico when the initial image is max 256x256.
        image.crop_imm(0, 0, 256, 256)
    })
}

/// Generate an icon with a background using the `imageproc` crate to draw text.
#[cfg(feature = "tray_icon_with_background")]
pub fn generate_icon_with_background(number: u32, light_theme: bool) -> Vec<u8> {
    let text = number.to_string();

    let font = get_number_font();
    let mut canvas = get_empty_image().clone();
    if light_theme {
        canvas.invert();
    }
    imageproc::drawing::draw_text_mut(
        &mut canvas,
        imageproc::image::Rgba(if light_theme {
            [0, 0, 0, 255]
        } else {
            [255, 255, 255, 255]
        }),
        if text.len() >= 2 { 110 } else { 130 },
        56,
        ab_glyph::PxScale { x: 150.0, y: 180.0 },
        font,
        &text,
    );
    // canvas = image::imageops::contrast(&canvas, 10.0).into();
    let mut data = Vec::new();
    canvas
        .write_to(&mut Cursor::new(&mut data), image::ImageFormat::Ico)
        .expect("Failed to convert generated tray image to ICO format");
    data
}

/// Generate icon without any background using the `imageproc` crate to draw text.
#[cfg(feature = "tray_icon_text_only")]
pub fn generate_icon_without_background(number: u32, light_theme: bool) -> Vec<u8> {
    let text = number.to_string();

    let font = get_number_font();
    let mut canvas = image::ImageBuffer::from_pixel(256, 256, image::Rgba([0_u8, 0, 0, 0]));

    imageproc::drawing::draw_text_mut(
        &mut canvas,
        imageproc::image::Rgba(if light_theme {
            [0, 0, 0, 255]
        } else {
            [255, 255, 255, 255]
        }),
        if text.len() >= 2 { -8 } else { 0 },
        -130,
        ab_glyph::PxScale {
            x: match text.len() {
                0 | 1 => 660.0,
                2 => 330.0,
                _ => 210.0,
            },
            y: 490.0,
        },
        font,
        &text,
    );
    // canvas = image::imageops::contrast(&canvas, 10.0).into();
    let mut data = Vec::new();
    canvas
        .write_to(&mut Cursor::new(&mut data), image::ImageFormat::Ico)
        .expect("Failed to convert generated tray image to ICO format");
    data
}

/// Generate icon without any background using the `text-to-png` crate to draw
/// text.
#[cfg(feature = "tray_icon_text_only_alt")]
pub fn generate_icon_without_background_alt(number: u32) -> Vec<u8> {
    let renderer = text_to_png::TextRenderer::try_new_with_ttf_font_data(NUMBER_FONT)
        .expect("Failed to load embedded font");

    let text_png = renderer
        .render_text_to_png_data(number.to_string(), 128, "Dark Turquoise")
        .expect("Failed to render text to PNG");

    // Convert from PNG to ICO:
    let mut data = Vec::new();
    image::io::Reader::with_format(Cursor::new(&text_png.data), image::ImageFormat::Png)
        .decode()
        .expect("Failed to read generated PNG")
        .write_to(&mut Cursor::new(&mut data), image::ImageFormat::Ico)
        .expect("Failed to convert tray image to ICO format");
    data
}
