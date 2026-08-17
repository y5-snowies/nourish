use bevy::prelude::Vec3;

pub const CAMERA_DISTANCE: f32 = 1.25;

pub const CAMERA_FOV_RAD: f32 = 0.6981317; // ~40°

pub const SPIN_SPEED: f32 = 0.35;

pub const HERO_POSITION: Vec3 = Vec3::new(0.0, 0.30, 0.0);

pub const HERO_SCALE: f32 = 0.22;

pub const MESH_RESOLUTION: u32 = 256;

pub const SPHERE_RADIUS: f32 = 0.5;

pub const LIGHT_DIR: Vec3 = Vec3::new(-0.6, 0.8, 0.4);

pub const LIGHT_INTENSITY: f32 = 0.85;

pub const AMBIENT_INTENSITY: f32 = 0.15;

// Debug: camera orbit (0.0 disables orbit for production).
pub const CAMERA_ORBIT_SPEED: f32 = 0.0; // rad/s

pub const CAMERA_ELEVATION: f32 = 0.0; // height above origin in world units

pub const CAMERA_ORBIT_RADIUS: f32 = 1.0; // horizontal distance from origin
