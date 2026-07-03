//! Horizontal width-tiling of global-space output positions.
use compositor_kernel_graphic_preference_layout_output::output::{tile_positions, OutputPosition};

#[test]
fn tiles_left_to_right_by_width() {
    let p = tile_positions(&[1920, 2560, 1280]);
    assert_eq!(p, vec![OutputPosition(0, 0), OutputPosition(1920, 0), OutputPosition(4480, 0)]);
}

#[test]
fn single_output_is_origin() {
    assert_eq!(tile_positions(&[3840]), vec![OutputPosition(0, 0)]);
}

#[test]
fn empty_is_empty() {
    assert!(tile_positions(&[]).is_empty());
}

#[test]
fn non_positive_width_does_not_advance() {
    // A degenerate 0-width output must not push the next output left/over it.
    let p = tile_positions(&[0, 1920]);
    assert_eq!(p, vec![OutputPosition(0, 0), OutputPosition(0, 0)]);
}
