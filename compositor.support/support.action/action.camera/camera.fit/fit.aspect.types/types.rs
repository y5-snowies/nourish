pub const MIN_W: f32 = 400.0;

pub const MIN_H: f32 = 400.0;

pub const SNAP_THRESHOLD: f32 = 0.05;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Size {
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Origin {
    TopLeft,
    Center,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Flags {
    pub scale_to_perceived: bool,
    pub maximize: bool,
}
