//! A tiny software ARGB8888 canvas over a borrowed shm slot.
//!
//! Wayland `wl_shm` `Argb8888` is a 32-bit native-endian pixel `0xAARRGGBB`. On the
//! little-endian targets this tool runs on, that is the byte order `[B, G, R, A]`, which is
//! what [`Canvas::put`] writes. Colors throughout the harness are `u32` `0xAARRGGBB`.

/// Borrowed drawing surface over a `width * height * 4` byte slot.
pub struct Canvas<'a> {
    data: &'a mut [u8],
    width: i32,
    height: i32,
    stride: i32,
}

/// Common opaque colors (`0xAARRGGBB`).
pub mod color {
    pub const BLACK: u32 = 0xFF000000;
    pub const WHITE: u32 = 0xFFFFFFFF;
    pub const RED: u32 = 0xFFE0_4040;
    pub const GREEN: u32 = 0xFF40_C040;
    pub const BLUE: u32 = 0xFF40_60E0;
    pub const YELLOW: u32 = 0xFFE0_C040;
    pub const CYAN: u32 = 0xFF40_C0E0;
    pub const MAGENTA: u32 = 0xFFE0_40C0;
    pub const GREY: u32 = 0xFF80_8080;
    pub const DKGREY: u32 = 0xFF30_3030;
    pub const LTGREY: u32 = 0xFFB0_B0B0;
    pub const PANEL: u32 = 0xFF20_2428;
    pub const BTN: u32 = 0xFF3A_4048;
    pub const BTN_HOT: u32 = 0xFF55_6070;
}

impl<'a> Canvas<'a> {
    /// Wrap a slot. `data` must be at least `width * height * 4` bytes.
    pub fn new(data: &'a mut [u8], width: i32, height: i32) -> Self {
        let stride = width * 4;
        Canvas { data, width, height, stride }
    }

    pub fn width(&self) -> i32 {
        self.width
    }
    pub fn height(&self) -> i32 {
        self.height
    }

    /// Write one pixel (alpha-ignored / overwrite). Out-of-bounds is a no-op.
    pub fn put(&mut self, x: i32, y: i32, argb: u32) {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return;
        }
        let i = (y * self.stride + x * 4) as usize;
        self.data[i] = (argb & 0xFF) as u8; // B
        self.data[i + 1] = ((argb >> 8) & 0xFF) as u8; // G
        self.data[i + 2] = ((argb >> 16) & 0xFF) as u8; // R
        self.data[i + 3] = ((argb >> 24) & 0xFF) as u8; // A
    }

    /// Fill the whole canvas.
    pub fn clear(&mut self, argb: u32) {
        for y in 0..self.height {
            for x in 0..self.width {
                self.put(x, y, argb);
            }
        }
    }

    /// Filled rectangle (clipped).
    pub fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, argb: u32) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.put(xx, yy, argb);
            }
        }
    }

    /// 1px (or `t`px) rectangle outline (clipped).
    pub fn frame(&mut self, x: i32, y: i32, w: i32, h: i32, t: i32, argb: u32) {
        self.rect(x, y, w, t, argb); // top
        self.rect(x, y + h - t, w, t, argb); // bottom
        self.rect(x, y, t, h, argb); // left
        self.rect(x + w - t, y, t, h, argb); // right
    }

    /// A crosshair marker centered at (x, y) — used to show the pointer location.
    pub fn crosshair(&mut self, x: i32, y: i32, arm: i32, argb: u32) {
        self.rect(x - arm, y, arm * 2 + 1, 1, argb);
        self.rect(x, y - arm, 1, arm * 2 + 1, argb);
        self.frame(x - 3, y - 3, 7, 7, 1, argb);
    }

    /// A 1px circle outline (midpoint algorithm, clipped) centered at (cx, cy).
    pub fn ring(&mut self, cx: i32, cy: i32, r: i32, argb: u32) {
        if r <= 0 {
            self.put(cx, cy, argb);
            return;
        }
        let (mut x, mut y, mut err) = (r, 0i32, 1 - r);
        while x >= y {
            for (px, py) in [
                (x, y), (y, x), (-x, y), (-y, x),
                (x, -y), (y, -x), (-x, -y), (-y, -x),
            ] {
                self.put(cx + px, cy + py, argb);
            }
            y += 1;
            if err < 0 {
                err += 2 * y + 1;
            } else {
                x -= 1;
                err += 2 * (y - x) + 1;
            }
        }
    }
}
