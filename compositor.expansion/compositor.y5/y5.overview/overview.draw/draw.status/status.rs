//! Per-frame HUD pushes for the overview: the menu-bar clock (local HH:MM, pushed
//! once a minute) and the Display-panel FPS (EMA, pushed ~every 30 frames). The
//! throttles keep the iced surfaces from re-rendering every frame.
use std::cell::RefCell;
use std::time::Instant;
use smithay::utils::{Physical, Size};
use compositor_orchestration_core_state_base::Loop;
use compositor_monitor_compositor_iced_base::{HandleId, IcedHandle};
use compositor_monitor_overview_ui_base::base::{OverviewMenu, OverviewMessage};
use compositor_configurator_settings_surface_message::message::SettingsMessage;
use compositor_configurator_settings_surface_view::Settings;
use compositor_orchestration_driver_settings_base::base::SETTINGS;
use compositor_y5_overview_state_base::base::MENU_BAR_HEIGHT;

const DOW: [&str; 7] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];
const MON: [&str; 12] = ["JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"];

thread_local! {
    static CLOCK_MIN: RefCell<i64> = const { RefCell::new(-1) };
    static FPS: RefCell<(Option<Instant>, f32, u64)> = const { RefCell::new((None, 0.0, 0)) };
    /// Last battery push, keyed by the menu surface id so a freshly-(re)opened menu
    /// always receives the current reading. `None` reading = desktop (no battery).
    static BATTERY: RefCell<Option<(HandleId, Option<(u8, bool)>)>> = const { RefCell::new(None) };
    /// Output width the menu bar was last sized to — re-size the full-width header
    /// when the output width drifts (mode/resolution/monitor change), mirroring the
    /// settings panel's resize (the menu bar is created once at the open-time width).
    static MENU_W: RefCell<Option<i32>> = const { RefCell::new(None) };
}

/// Called from the overview GLES prepare while the overlay is open.
pub fn per_frame(state: &mut Loop, size: Size<i32, Physical>) {
    // This runs once per output in the render loop; the menu bar is a screen-space
    // surface on the ACTIVE monitor only. Act only on the active output's pass so
    // `size` is that monitor's — otherwise every OTHER output's width thrashes the
    // bar's resize each frame. (render_output None = single/non-loop pass → run.)
    // Mirrors the gate in overview.draw/draw.settings/settings.rs.
    if let Some(k) = &state.inner.render_output {
        if *k != state.inner.active_output_key() {
            return;
        }
    }
    resize_menu(state, size);
    clock(state);
    battery(state);
    fps(state);
}

/// Keep the full-width menu/tab bar spanning the output: re-size it when the output
/// width changes. (Position is fixed at the top-left.)
fn resize_menu(state: &mut Loop, size: Size<i32, Physical>) {
    let Some(menu) = state.inner.overview().menu else { return };
    let drifted = MENU_W.with(|w| { let mut w = w.borrow_mut(); if *w != Some(size.w) { *w = Some(size.w); true } else { false } });
    if drifted {
        if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
            reg.request_resize_by_id(menu, Size::from((size.w, MENU_BAR_HEIGHT)));
        }
    }
}

fn now_tm() -> libc::tm {
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm
    }
}

fn clock(state: &mut Loop) {
    let Some(menu) = state.inner.overview().menu else { return };
    let tm = now_tm();
    let key = tm.tm_hour as i64 * 60 + tm.tm_min as i64;
    let changed = CLOCK_MIN.with(|c| { let mut c = c.borrow_mut(); if *c != key { *c = key; true } else { false } });
    if !changed {
        return;
    }
    let dow = DOW.get(tm.tm_wday as usize).copied().unwrap_or("");
    let mon = MON.get(tm.tm_mon as usize).copied().unwrap_or("");
    let label = format!("{dow} {:02} {mon}   ·   {:02}:{:02}", tm.tm_mday, tm.tm_hour, tm.tm_min);
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        let _ = reg.dispatch_message(IcedHandle::<OverviewMenu>::from_id(menu), OverviewMessage::Clock(label));
    }
}

/// Push the laptop battery indicator to the menu bar. On desktops (no battery)
/// nothing is shown. Only pushes when the level or charging state changes.
fn battery(state: &mut Loop) {
    let Some(menu) = state.inner.overview().menu else { return };
    let now = compositor_configurator_hardware_battery_base::base::read().map(|b| (b.capacity, b.charging));
    let changed = BATTERY.with(|c| {
        let mut c = c.borrow_mut();
        if *c != Some((menu, now)) {
            *c = Some((menu, now));
            true
        } else {
            false
        }
    });
    if !changed {
        return;
    }
    let label = now.map(|(cap, charging)| format!("{} {cap}%", if charging { "CHG" } else { "BAT" }));
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        let _ = reg.dispatch_message(IcedHandle::<OverviewMenu>::from_id(menu), OverviewMessage::Battery(label));
    }
}

fn fps(state: &mut Loop) {
    let now = Instant::now();
    let (ema, n) = FPS.with(|f| {
        let mut f = f.borrow_mut();
        if let Some(prev) = f.0 {
            let dt = now.duration_since(prev).as_secs_f32();
            if dt > 0.0 {
                let cur = 1.0 / dt;
                f.1 = if f.1 == 0.0 { cur } else { f.1 * 0.9 + cur * 0.1 };
            }
        }
        f.0 = Some(now);
        f.2 = f.2.wrapping_add(1);
        (f.1, f.2)
    });
    if n % 30 != 0 {
        return;
    }
    // Only push while the Performance tab is the visible module (gate set by the
    // forwarded Tab message) — other tabs shouldn't buffer per-frame FPS updates.
    let (wanted, handle) = {
        let st = state.inner.kernel.get(&SETTINGS);
        (st.fps_wanted, st.handle)
    };
    if !wanted {
        return;
    }
    let Some(handle) = handle else { return };
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        let _ = reg.dispatch_message(IcedHandle::<Settings>::from_id(handle), SettingsMessage::Fps(ema.round() as u32));
    }
}
