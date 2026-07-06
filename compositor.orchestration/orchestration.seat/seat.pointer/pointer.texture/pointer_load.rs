use std::collections::HashMap;
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::utils::{Physical, Point, Size, Transform};
use xcursor::CursorTheme;
use xcursor::parser::{Image, parse_xcursor};

pub struct Cursor {
    pub frames: Vec<CursorFrame>,
    pub hotspot: Point<i32, Physical>,
    pub size: Size<i32, Physical>,
}

pub struct CursorFrame {
    pub buffer: MemoryRenderBuffer,
    pub delay: Duration,
}

pub struct CursorThemeCache {
    theme: CursorTheme,
    fallback_size: u32,
    cache: Mutex<HashMap<String, Option<Arc<Cursor>>>>,
}

impl CursorThemeCache {
    pub fn new(theme_name: &str, size: u32) -> Self {
        let theme = CursorTheme::load(theme_name);

        let test_1 = theme.load_icon("default");
        let test_2 = theme.load_icon("text");
        let test_3 = theme.load_icon("pointer");
        info!(
            "theme_name={:?} load_icon('default')={:?} load_icon('text')={:?} load_icon('pointer')={:?}",
            theme_name, test_1, test_2, test_3
        );

        Self {
            theme,
            fallback_size: size,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Resolve a freedesktop cursor name to a loaded cursor.
    /// Walks the theme's inheritance chain, then falls back to "default"
    /// and finally "left_ptr". Returns None only if the theme is broken
    /// or no cursors are installed at all.
    pub fn get(&self, name: &str) -> Option<Arc<Cursor>> {
        if let Some(entry) = self.cache.lock().unwrap().get(name) {
            return entry.clone();
        }

        let loaded = self
            .load_shape(name)
            .or_else(|| self.load_shape("default"))
            .or_else(|| self.load_shape("left_ptr"));

        self.cache
            .lock()
            .unwrap()
            .insert(name.to_string(), loaded.clone());
        loaded
    }

    fn load_shape(&self, name: &str) -> Option<Arc<Cursor>> {
        // let path = self.theme.load_icon(name)?;
        // let mut file = std::fs::File::open(path).ok()?;
        // let mut buf = Vec::new();
        // file.read_to_end(&mut buf).ok()?;
        // let images = parse_xcursor(&buf)?;

        let path = self.theme.load_icon(name)?;

        let mut file = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(e) => {
                return None;
            }
        };
        let mut buf = Vec::new();
        if let Err(e) = file.read_to_end(&mut buf) {
            return None;
        }

        let images = match parse_xcursor(&buf) {
            Some(imgs) => imgs,
            None => {
                warn!("load_shape({}): parse_xcursor returned None", name);
                return None;
            }
        };

        // 1. Find the nominal size closest to what we want.
        let target = self.fallback_size;
        let chosen_size = images
            .iter()
            .map(|img| img.size)
            .min_by_key(|s| (*s as i32 - target as i32).abs())?;

        trace!("chosen_size={:?}", chosen_size);
        // 2. Collect every frame at that size, in file order.
        //    For static cursors this is one frame; for animated cursors
        //    (wait, progress) it's the full sequence.
        let frames: Vec<CursorFrame> = images
            .iter()
            .filter(|img| img.size == chosen_size)
            .map(image_to_frame)
            .collect();

        if frames.is_empty() {
            return None;
        }

        // Hotspot and dimensions come from the first frame.
        // All frames in a well-formed xcursor share these values.
        let first = images.iter().find(|img| img.size == chosen_size)?;

        let hotspot = Point::from((first.xhot as i32, first.yhot as i32));
        let size = Size::from((first.width as i32, first.height as i32));

        Some(Arc::new(Cursor {
            frames,
            hotspot,
            size,
        }))
    }
}

fn image_to_frame(img: &Image) -> CursorFrame {
    let width = img.width as i32;
    let height = img.height as i32;
    // let stride = width * 4;

    let buffer = MemoryRenderBuffer::from_slice(
        &img.pixels_rgba,
        Fourcc::Argb8888,
        Size::from((width, height)),
        1, // scale: 1 for non-HiDPI cursors
        Transform::Normal,
        None,
    );
    // let buffer = MemoryRenderBuffer::from_slice(
    //     &img.pixels_rgba,
    //     Fourcc::Argb8888,
    //     Size::from((width, height)),
    //     stride,
    //     Transform::Normal,
    //     None,
    // );

    // xcursor delay is in milliseconds. A delay of 0 on a multi-frame
    // cursor would loop forever on one frame, so floor to 1ms.
    let delay = if img.delay == 0 {
        Duration::from_millis(1)
    } else {
        Duration::from_millis(img.delay as u64)
    };

    CursorFrame { buffer, delay }
}
