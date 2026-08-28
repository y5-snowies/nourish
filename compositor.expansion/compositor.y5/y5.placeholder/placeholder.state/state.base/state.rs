use smithay::desktop::Window;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ops::DerefMut;
use std::rc::Rc;
use std::sync::Arc;
use uuid::Uuid;
use compositor_introspection_extraction_window_base::{HandlerRegistry, default_registry};
use compositor_introspection_launchplan_plan_base::SynthesizerRegistry;
use compositor_introspection_restoration_state_base::MatcherRegistry;
use compositor_y5_placeholder_record_base::placeholder::{Placeholder, PlaceholderVisible};
use compositor_y5_placeholder_surface_base::PlaceholderUi;
use compositor_monitor_compositor_iced_base::IcedHandle;

pub struct PlaceholderState {
    pub map: HashMap<Uuid, Rc<RefCell<Placeholder>>>,
    vec: Vec<Rc<RefCell<Placeholder>>>,
    pub visible: Vec<(PlaceholderVisible, IcedHandle<PlaceholderUi>)>,
    /// Placeholders rehydrated from disk, awaiting promotion to `visible` by the
    /// rim (which holds the renderer needed to build their iced surface). Drained
    /// on the first frame this world is the spawn-target — see
    /// `placeholder.interface::promote_restored`.
    pub pending_restore: Vec<Placeholder>,
    pub application_registry: Arc<HandlerRegistry>,
    pub synthesizer_registry: Arc<SynthesizerRegistry>,
    pub restoration_registry: Arc<MatcherRegistry>,
}

/// The placeholder storage slot tokens. Defined here (with the state) so both the
/// owning system and the persistence document can reference them without a crate
/// cycle; the system re-exports them for its legacy call sites.
pub static PLACEHOLDER: compositor_support_system_storage_token_base::base::Token<PlaceholderState> =
    compositor_support_system_storage_token_base::base::Token::new();
pub static PLACEHOLDER_MUT: compositor_support_system_storage_token_base::base::TokenMut<PlaceholderState> =
    compositor_support_system_storage_token_base::base::TokenMut::new(&PLACEHOLDER);

impl PlaceholderState {
    pub fn new() -> Self {
        Self {
            vec: vec![],
            map: HashMap::new(),
            visible: vec![],
            pending_restore: vec![],
            application_registry: Arc::new(default_registry()),
            synthesizer_registry: Arc::new(compositor_introspection_launchplan_plan_base::default_synthesizers()),
            restoration_registry: Arc::new(compositor_introspection_restoration_state_base::default_matchers()),
        }
    }

    pub fn insert(&mut self, placeholder: Placeholder, uuid: Uuid) {
        // println!("Add placeholder: {:?}", placeholder);
        let arc = Rc::new(RefCell::new(placeholder));
        let result = self.map.insert(uuid, arc.clone());
        if result.is_some() {
            abort!("Duplication placeholder set.");
        }

        self.vec.push(arc)
    }

    pub fn modify<F>(&mut self, uuid: &Uuid, mut action: F)
    where
        F: FnMut(&mut Placeholder),
    {
        let item = self.map.get(uuid).unwrap_or_else(|| abort!("record to exist"));
        let mut guard = item.borrow_mut();

        // Apply the closure to the internal data
        action(&mut *guard);

        // The guard is safely dropped here when the function ends
    }

    /// Like `modify`, but a no-op when no map record exists for `uuid` instead of
    /// aborting. Used by the placeholder system's buffer, where an announced
    /// geometry update is keyed by a uuid that may be a window (live → in `map`)
    /// OR a visible placeholder (in `visible`, not `map`) — the buffer applies both halves
    /// and this absorbs the miss for the namespace that doesn't match.
    pub fn modify_present<F>(&mut self, uuid: &Uuid, mut action: F)
    where
        F: FnMut(&mut Placeholder),
    {
        let Some(item) = self.map.get(uuid) else {
            return;
        };
        action(&mut *item.borrow_mut());
    }

    pub fn erase(&mut self, uuid: &Uuid) -> Placeholder {
        // Erase was called from an unknown window meaning a few things:
        // 1. window_destroyed was called before a top level was created.

        let target_rc = self.map.remove(uuid).unwrap_or_else(|| abort!("record to exist"));
        // 1. Remove the item from the HashMap first.
        // `remove` returns the value (the Rc) if it existed.
        let index = self
            .vec
            .iter()
            .position(|item| Rc::ptr_eq(item, &target_rc))
            .unwrap_or_else(|| abort!("record to exist"));

        // PERFORMANCE TIP:
        // Use `remove(index)` if you MUST keep the Vec elements in their original order. (O(N) operation)
        // Use `swap_remove(index)` if order doesn't matter. It swaps the item with the last element and pops it. (O(1) operation)
        let ph = self.vec.remove(index);
        return ph.as_ref().clone().into_inner();
    }

    pub fn push_visible(&mut self, placeholder: Placeholder, handle: IcedHandle<PlaceholderUi>) {
        self.visible.push((placeholder.into(), handle));
    }
    pub fn modify_visible<F>(&mut self, placeholder_uuid: &Uuid, mut action: F) -> Option<&mut( PlaceholderVisible, IcedHandle<PlaceholderUi>)>
    where
        F: FnOnce(&mut PlaceholderVisible),
    {
        let index = self
            .visible
            .iter()
            .position(|w| &w.0.uuid == placeholder_uuid);
        if index.is_none() {
            // OK due to buffer considerations
            return None;
        }

        let item = self.visible.get_mut(index.unwrap()).unwrap();
        action(&mut item.0);

        Some(item)
        // The guard is safely dropped here when the function ends
    }

    pub fn visible_with<F>(&mut self, placeholder_uuid: &Uuid, mut action: F)
    where
        F: FnOnce(&mut PlaceholderVisible),
    {
        let index = self
            .visible
            .iter()
            .position(|w| &w.0.uuid == placeholder_uuid);
        if index.is_none() {
            // OK due to buffer considerations
            return;
        }

        let item = self.visible.get_mut(index.unwrap()).unwrap();
        action(&mut item.0);

        // The guard is safely dropped here when the function ends
    }

    pub fn erase_visible(
        &mut self,
        placeholder_uuid: &Uuid,
    ) -> Option<(PlaceholderVisible, IcedHandle<PlaceholderUi>)> {
        let index = self
            .visible
            .iter()
            .position(|w| &w.0.uuid == placeholder_uuid);
        if index.is_none() {
            // OK due to buffer considerations. however should be on caller side
            return None;
        }
        let index = index.unwrap();

        // PERFORMANCE TIP:
        // Use `remove(index)` if you MUST keep the Vec elements in their original order. (O(N) operation)
        // Use `swap_remove(index)` if order doesn't matter. It swaps the item with the last element and pops it. (O(1) operation)
        let ph = self.visible.remove(index);
        return Some(ph);
    }
}

// TO determine which placeholders to render:
// 1. The window is no longer visible.
//    such window should be unmapped by uuid. panics should not be allowed really since it is the defined behavior.
// 2. It may be better to separate into visible placeholders- movement from vec into unmapped vec.
// 3. "Living" placeholders must be given a unique ID.
//   this unique ID must be used throughout the restoration process and for the storage of placeholder elements.
// the "living placeholder" struct must contain the initialized IcedRegistry handle.
