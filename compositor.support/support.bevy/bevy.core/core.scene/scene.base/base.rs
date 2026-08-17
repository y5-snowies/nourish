use bevy::asset::Handle;
use bevy::image::Image;
use bevy::prelude::{App, World};

/// One Bevy scene definition.
pub trait BevyScene: Send + Sync + 'static {
    /// Typed external command dispatched via `registry.dispatch_command(...)`.
    type Command: Send + Sync + std::fmt::Debug + 'static;

    /// Add Bevy systems, entities, resources.
    ///
    /// `output` is the `Handle<Image>` to assign to a `Camera`'s render
    /// target. Backed by a dmabuf the compositor samples.
    ///
    /// Any other texture inputs the scene needs should be carried on the
    /// scene struct itself, obtained from
    /// `BevyRegistry::import_dmabuf(...)` and passed into the scene's
    /// constructor.
    fn build(&self, app: &mut App, output: Handle<Image>);

    /// Apply a typed command to the world.
    fn apply_command(&self, world: &mut World, command: Self::Command);
}
