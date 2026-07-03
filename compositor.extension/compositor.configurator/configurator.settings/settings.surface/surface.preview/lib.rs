//! A live wgpu parallax preview for the Current-World settings panel: an
//! `iced::widget::shader` program that runs an embedded parallax WGSL pipeline,
//! driven by the current `@prop` params and mouse pan (drag) + zoom (scroll).

pub mod preview;
