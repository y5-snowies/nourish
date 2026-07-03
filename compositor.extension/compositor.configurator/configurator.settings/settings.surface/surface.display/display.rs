//! Display module: preferred-monitor picker + the selected monitor's advertised
//! modes. A **CHECK CHANGES** button (below the monitor list) provisionally
//! applies the selected monitor/mode through the fault gate; **APPLY** keeps it
//! and **REVERT** undoes it. All three are always rendered (no layout shift) and
//! enabled only when relevant. Selecting a different monitor + applying switches
//! the active output; selecting another mode changes the active monitor's mode.
use compositor_developer_environment_preference_base::base::LayoutPlacement;
use compositor_orchestration_driver_output_base::base::{DisplayInfo, ModeInfo};
use compositor_support_iced_core_engine_base::Renderer;
use compositor_configurator_settings_surface_message::message::{Applied, SettingsMessage};
use compositor_configurator_settings_surface_style::style;
use compositor_configurator_settings_surface_control::control;
use crate::layout_canvas::layout_canvas;
use iced_core::{Element, Length, Theme};
use iced_widget::{button, checkbox, column, row, scrollable, text, Column};

type El<'a> = Element<'a, SettingsMessage, Theme, Renderer>;

fn mode_label(m: &ModeInfo) -> String {
    format!("{}×{}   ·   {:.2} Hz", m.width, m.height, m.refresh_mhz as f32 / 1000.0)
}

pub fn build<'a>(
    displays: &'a [DisplayInfo],
    active_edid: &str,
    selected_display: &str,
    selected_mode: Option<ModeInfo>,
    confirming: bool,
    pending: Option<&Applied>,
    staged_active: Option<&(String, Option<ModeInfo>)>,
    layout: &'a [LayoutPlacement],
    selected_placement: Option<u64>,
    cyclic: bool,
    selected_inactive: bool,
) -> El<'a> {
    let head = column![
        text("DISPLAY").size(16).color(style::ACCENT),
        text("Preferred monitor, resolution and refresh rate.").size(11).color(style::MUTED),
    ].spacing(4);

    if displays.is_empty() {
        return column![
            head,
            text("No monitors detected (running nested / no DRM backend).").size(12).color(style::MUTED)
        ].spacing(14).into();
    }

    // Monitor picker (● = active output, accent = selected in the picker).
    let mut monitors: Vec<El<'a>> = vec![text("MONITOR").size(11).color(style::MUTED).into()];
    for d in displays {
        let mark = if d.edid_key == active_edid { "●" } else { "○" };
        let label = format!("{mark}  {}   ·   {}", d.name, d.edid_key);
        let on = d.edid_key == selected_display;
        let b = button(text(label)).width(Length::Fill).on_press(SettingsMessage::SelectDisplay(d.edid_key.clone()));
        monitors.push(if on { b.style(control::accent) } else { b.style(control::action) }.into());
    }

    // Action row directly below the monitor list — always present, conditionally
    // enabled (a button with no on_press is disabled, so the layout never shifts).
    // Multi-output: every connected monitor is independently driven, so changing a
    // resolution is always an in-place per-pipe mode change (`switch = false`) — NOT
    // an active-output switch (which tears the primary down and fails the modeset for
    // an already-lit secondary → the "reverts immediately" bug). A change is pending
    // when the picked mode differs from the SELECTED monitor's current mode.
    // The pending selection (Inactive, or a mode) is applied on CHECK CHANGES — NOT on
    // selecting the item — so deactivate/reactivate/resolution all go through the gate.
    let selected = displays.iter().find(|d| d.edid_key == selected_display);
    let cur_inactive = selected.map(|d| !d.enabled).unwrap_or(false);
    let cur_mode = selected.and_then(|d| d.current);
    let changed = if selected_inactive {
        !cur_inactive
    } else if let Some(m) = selected_mode {
        cur_inactive || Some(m) != cur_mode
    } else {
        false
    };
    // Deactivating the LAST active monitor is not allowed (all `displays` are
    // connected, so "active" = `enabled`).
    let active_count = displays.iter().filter(|d| d.enabled).count();
    // CHECK arms the confirm flow, resolved by APPLY/REVERT. BOTH a deactivate/
    // reactivate (`StageActive`) and a resolution change (`Apply`) apply
    // live-provisionally through the fault gate and auto-revert if not kept.
    let check_msg: Option<SettingsMessage> = if confirming || !changed {
        None
    } else if selected_inactive {
        (active_count > 1).then(|| SettingsMessage::StageActive(selected_display.to_string(), None))
    } else {
        selected_mode.map(|mode| {
            if cur_inactive {
                SettingsMessage::StageActive(selected_display.to_string(), Some(mode))
            } else {
                SettingsMessage::Apply(Applied { edid_key: selected_display.to_string(), mode })
            }
        })
    };
    let check = {
        let b = button(text("CHECK CHANGES")).width(Length::Fill);
        match check_msg {
            Some(msg) => b.style(control::action).on_press(msg),
            None => b.style(control::disabled),
        }
    };
    // APPLY commits whichever confirm flow is armed: a provisional activate/deactivate
    // is KEPT (`SetActive` disarms the auto-revert), like a resolution change (`Keep`).
    let apply = {
        let b = button(text("APPLY")).width(Length::Fill);
        if confirming {
            if let Some((edid, mode)) = staged_active {
                b.style(control::accent).on_press(SettingsMessage::SetActive(edid.clone(), *mode))
            } else if let Some(p) = pending {
                b.style(control::accent).on_press(SettingsMessage::Keep(p.clone()))
            } else {
                b.style(control::disabled)
            }
        } else {
            b.style(control::disabled)
        }
    };
    let revert = {
        let b = button(text("REVERT")).width(Length::Fill);
        if confirming { b.style(control::action).on_press(SettingsMessage::Revert) } else { b.style(control::disabled) }
    };
    let actions = row![check, apply, revert].spacing(8);

    // Modes for the selected monitor. The FIRST item is "Inactive". Selecting any row
    // (Inactive or a mode) is a UI-LOCAL selection; CHECK CHANGES applies it.
    let sel = displays.iter().find(|d| d.edid_key == selected_display);
    let mut modes: Vec<El<'a>> = vec![text("RESOLUTION").size(11).color(style::MUTED).into()];
    match sel {
        Some(d) if !d.available.is_empty() => {
            let inactive = button(text("Inactive")).width(Length::Fill).on_press(SettingsMessage::SelectInactive);
            modes.push(if selected_inactive { inactive.style(control::accent) } else { inactive.style(control::action) }.into());
            for m in &d.available {
                let on = !selected_inactive && Some(*m) == selected_mode;
                let b = button(text(mode_label(m))).width(Length::Fill).on_press(SettingsMessage::SelectMode(*m));
                modes.push(if on { b.style(control::accent) } else { b.style(control::action) }.into());
            }
        }
        _ => modes.push(text("No advertised modes for this monitor.").size(12).color(style::MUTED).into()),
    }

    // Cursor-teleport layout editor — only meaningful with more than one monitor.
    // The canvas draws each placed monitor as a draggable/resizable square (inner
    // box = true aspect); "＋" buttons add a monitor to the layout; APPLY LAYOUT
    // persists it. Clicking a square selects that monitor so the controls below
    // populate for it.
    let mut col = column![head].spacing(14);
    if displays.len() > 1 {
        let mut adds: Vec<El<'a>> =
            vec![text("ADD TO CURSOR LAYOUT").size(11).color(style::MUTED).into()];
        // Only ACTIVE monitors can be added to the map (inactive ones aren't driven,
        // so they aren't part of the cursor-crossing arrangement).
        for d in displays.iter().filter(|d| d.enabled) {
            adds.push(
                button(text(format!("＋ {}", d.name)))
                    .width(Length::Fill)
                    .style(control::action)
                    .on_press(SettingsMessage::LayoutPlace(d.edid_key.clone(), 0.0, 0.0))
                    .into(),
            );
        }
        let apply_layout = button(text("APPLY LAYOUT"))
            .width(Length::Fill)
            .style(control::accent)
            .on_press(SettingsMessage::LayoutCommit(layout.to_vec()));
        let remove: El<'a> = match selected_placement {
            Some(id) => button(text("REMOVE"))
                .style(control::action)
                .on_press(SettingsMessage::LayoutRemove(id))
                .into(),
            None => button(text("REMOVE")).style(control::disabled).into(),
        };
        // Cyclic (wrap-around) toggle for the teleport map.
        let cyclic_row = row![
            checkbox(cyclic).on_toggle(SettingsMessage::SetCyclic),
            text("Cyclic — wrap the cursor around the layout edges").size(12).color(style::MUTED),
        ]
        .spacing(8);
        col = col.push(
            column![
                text("CURSOR TELEPORT LAYOUT").size(11).color(style::MUTED),
                // Canvas (pan/zoom, fills) on the left; the add-monitor list is a
                // vertical column on its right.
                row![
                    layout_canvas(layout, displays, selected_placement),
                    Column::with_children(adds).spacing(6).width(Length::Fixed(200.0)),
                ]
                .spacing(12),
                cyclic_row,
                row![apply_layout, remove].spacing(8),
            ]
            .spacing(8),
        );
    }

    col.push(Column::with_children(monitors).spacing(6))
        .push(actions)
        .push(scrollable(Column::with_children(modes).spacing(6)).height(Length::Fill))
        .into()
}
