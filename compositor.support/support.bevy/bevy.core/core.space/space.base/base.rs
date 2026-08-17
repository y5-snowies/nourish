use smithay::utils::{Physical, Point, Size};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BevySpace {
    World,
    Screen,
}

impl Default for BevySpace {
    fn default() -> Self {
        BevySpace::World
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub position: Point<f64, Physical>,
    pub zoom: f64,
}

impl Transform {
    pub fn identity() -> Self {
        Self {
            position: Point::from((0.0, 0.0)),
            zoom: 1.0,
        }
    }

    /// World point → screen point. Centered origin.
    pub fn world_to_screen(
        &self,
        output_size: Size<f64, Physical>,
        world: Point<f64, Physical>,
    ) -> Point<f64, Physical> {
        Point::from((
            (world.x - self.position.x) * self.zoom + output_size.w / 2.0,
            (world.y - self.position.y) * self.zoom + output_size.h / 2.0,
        ))
    }

    /// Screen point → world point. Inverse of `world_to_screen`.
    pub fn screen_to_world(
        &self,
        output_size: Size<f64, Physical>,
        screen: Point<f64, Physical>,
    ) -> Point<f64, Physical> {
        Point::from((
            (screen.x - output_size.w / 2.0) / self.zoom + self.position.x,
            (screen.y - output_size.h / 2.0) / self.zoom + self.position.y,
        ))
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::identity()
    }
}

/// Screen-space location of an item whose stored location is `location`.
pub fn item_screen_location(
    space: BevySpace,
    transform: &Transform,
    output_size: Size<f64, Physical>,
    location: Point<i32, Physical>,
) -> Point<i32, Physical> {
    match space {
        BevySpace::Screen => location,
        BevySpace::World => {
            let s = transform
                .world_to_screen(output_size, Point::from((location.x as f64, location.y as f64)));
            Point::from((s.x as i32, s.y as i32))
        }
    }
}

/// Screen-space size of an item whose natural size is `size`.
pub fn item_screen_size(
    space: BevySpace,
    transform: &Transform,
    size: Size<i32, Physical>,
) -> Size<i32, Physical> {
    match space {
        BevySpace::Screen => size,
        BevySpace::World => Size::from((
            (size.w as f64 * transform.zoom) as i32,
            (size.h as f64 * transform.zoom) as i32,
        )),
    }
}
