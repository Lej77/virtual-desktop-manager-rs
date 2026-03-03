use std::ops::{Deref, DerefMut};
use winsafe::gui;

/// Utility for layout of controls inside a Winsafe window or control/panel.
#[derive(Clone, Ord, PartialOrd, PartialEq, Eq, Debug)]
pub struct LayoutArea {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub margin: i32,
}
impl LayoutArea {
    pub const DEFAULT_MARGIN: i32 = 10;

    pub fn new(width: i32, height: i32, margin: i32) -> Self {
        Self {
            x: 0,
            y: 0,
            width,
            height,
            margin,
        }
    }
    pub fn from_size(width: i32, height: i32) -> Self {
        Self::new(width, height, Self::DEFAULT_MARGIN)
    }
    pub fn with_min_size(mut self, width: i32, height: i32) -> Self {
        self.width = self.width.max(width);
        self.height = self.height.max(height);
        self
    }

    /// Ensure the layout area is surrounded with [`LayoutArea::margin`] free space on all sides.
    pub fn apply_margin(&mut self) {
        self.x += self.margin;
        self.y += self.margin;
        self.width -= self.margin * 2;
        self.height -= self.margin * 2;
    }

    pub fn with_temp_margin(&mut self, margin: i32) -> LayoutAreaTempMargin<'_> {
        let prev_margin = self.margin;
        self.margin = margin;
        LayoutAreaTempMargin {
            layout: self,
            prev_margin,
        }
    }

    pub fn take_top(&mut self, height: i32) -> LayoutArea {
        self.take_top_with_margin(height, self.margin)
    }
    pub fn take_top_with_margin(&mut self, height: i32, margin: i32) -> LayoutArea {
        let taken = LayoutArea {
            x: self.x,
            y: self.y,
            width: self.width,
            height,
            margin: self.margin,
        };
        let diff = height + margin;
        self.y += diff;
        self.height -= diff;
        taken
    }
    pub fn take_bottom(&mut self, height: i32) -> LayoutArea {
        self.take_bottom_with_margin(height, self.margin)
    }
    pub fn take_bottom_with_margin(&mut self, height: i32, margin: i32) -> LayoutArea {
        self.height -= height;
        let taken = LayoutArea {
            x: self.x,
            y: self.y + self.height,
            width: self.width,
            height,
            margin: self.margin,
        };
        self.height -= margin;
        taken
    }
    pub fn take_left(&mut self, width: i32) -> LayoutArea {
        self.take_left_with_margin(width, self.margin)
    }
    pub fn take_left_with_margin(&mut self, width: i32, margin: i32) -> LayoutArea {
        let taken = LayoutArea {
            x: self.x,
            y: self.y,
            width,
            height: self.height,
            margin: self.margin,
        };
        let diff = width + margin;
        self.x += diff;
        self.width -= diff;
        taken
    }
    pub fn take_right(&mut self, width: i32) -> LayoutArea {
        self.take_right_with_margin(width, self.margin)
    }
    pub fn take_right_with_margin(&mut self, width: i32, margin: i32) -> LayoutArea {
        self.width -= width;
        let taken = LayoutArea {
            x: self.x + self.width,
            y: self.y,
            width,
            height: self.height,
            margin: self.margin,
        };
        self.width -= margin;
        taken
    }

    pub fn split_horizontal<const N: usize>(self) -> [LayoutArea; N] {
        let margin = self.margin;
        self.split_horizontal_with_margin(margin)
    }
    pub fn split_horizontal_with_margin<const N: usize>(mut self, margin: i32) -> [LayoutArea; N] {
        if N == 0 {
            return std::array::from_fn(|_| unreachable!());
        }
        let with_without_margin = self.width - margin * ((N - 1) as i32);
        let width = with_without_margin / (N as i32);
        // Note: we divide the remaining space among as many areas as possible.
        let remaining = (with_without_margin % (N as i32)).abs() as usize;
        std::array::from_fn(|index| self.take_left(width + if index < remaining { 1 } else { 0 }))
    }

    pub fn top(&self) -> i32 {
        self.y
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.height
    }
    pub fn left(&self) -> i32 {
        self.x
    }
    pub fn right(&self) -> i32 {
        self.x + self.width
    }

    pub fn dpi_width(&self) -> i32 {
        gui::dpi_x(self.width)
    }
    pub fn dpi_height(&self) -> i32 {
        gui::dpi_y(self.height)
    }
    pub fn dpi_pos(&self) -> (i32, i32) {
        gui::dpi(self.x, self.y)
    }
    pub fn dpi_size(&self) -> (i32, i32) {
        gui::dpi(self.width, self.height)
    }
}
impl Default for LayoutArea {
    fn default() -> Self {
        Self::from_size(0, 0)
    }
}

pub struct LayoutAreaTempMargin<'a> {
    layout: &'a mut LayoutArea,
    prev_margin: i32,
}
impl Deref for LayoutAreaTempMargin<'_> {
    type Target = LayoutArea;

    fn deref(&self) -> &Self::Target {
        &self.layout
    }
}
impl DerefMut for LayoutAreaTempMargin<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.layout
    }
}
impl Drop for LayoutAreaTempMargin<'_> {
    fn drop(&mut self) {
        self.layout.margin = self.prev_margin;
    }
}
