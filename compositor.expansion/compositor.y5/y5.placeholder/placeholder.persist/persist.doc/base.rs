use compositor_support_system_persist_document_trait::base::Document;
use compositor_support_system_persist_document_trait::y5_document;
use compositor_y5_placeholder_persist_record::base::{to_launch_plan, to_persisted, PlaceholderRecord};
use compositor_y5_placeholder_record_base::placeholder::Placeholder;
use compositor_y5_placeholder_state_base::state::{PlaceholderState, PLACEHOLDER, PLACEHOLDER_MUT};
use std::time::Instant;

/// Projects a world's placeholders into the partitioned `world.placeholder` table.
pub struct PlaceholderDoc;

impl Document for PlaceholderDoc {
    type Slot = PlaceholderState;
    type Record = PlaceholderRecord;
    const TABLE: &'static str = "world.placeholder";
    const VERSION: u32 = 1;

    /// Persist BOTH sets: `map` (invisible, window-backed) AND `visible` (the
    /// launcher tiles shown after a window closes, incl. their dragged transform).
    /// A visible placeholder is no longer in `map`, so omitting it would delete its
    /// record the moment its window closed — exactly the state worth keeping.
    fn rows(s: &PlaceholderState) -> Vec<(String, Vec<(&'static str, String)>, PlaceholderRecord)> {
        let from_map = s.map.iter().map(|(id, rc)| {
            let p = rc.borrow();
            let launch = p.launch.as_ref().map(to_persisted).unwrap_or_default();
            let record = PlaceholderRecord {
                position: p.position, size: p.size, persistent: p.persistent, launch,
            };
            (id.to_string(), Vec::new(), record)
        });
        let from_visible = s.visible.iter().map(|(v, _)| {
            let record = PlaceholderRecord {
                position: v.position, size: v.size, persistent: true,
                launch: to_persisted(&v.launch),
            };
            (v.uuid.to_string(), Vec::new(), record)
        });
        from_map.chain(from_visible).collect()
    }

    /// Queue the restored placeholder for visible promotion: on restart no client
    /// window exists, so every persisted placeholder returns as a dormant launcher
    /// tile. The rim drains `pending_restore` once it has a renderer (the iced
    /// surface can't be built here, with only `&mut PlaceholderState`).
    fn apply(s: &mut PlaceholderState, id: &str, rec: PlaceholderRecord) {
        let Ok(uuid) = uuid::Uuid::parse_str(id) else { return };
        if s.map.contains_key(&uuid) || s.visible.iter().any(|(v, _)| v.uuid == uuid) {
            return;
        }
        s.pending_restore.push(Placeholder {
            position: rec.position,
            size: rec.size,
            launch: Some(to_launch_plan(&rec.launch)),
            launch_session: None,
            uuid,
            session_time: Instant::now(),
            persistent: rec.persistent,
        });
    }
}

y5_document!(PLACEHOLDER_DOC, PlaceholderDoc, PLACEHOLDER, PLACEHOLDER_MUT);
