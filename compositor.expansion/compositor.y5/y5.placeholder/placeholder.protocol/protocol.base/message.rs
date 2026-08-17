use uuid::Uuid;
use compositor_introspection_launchplan_plan_base::LaunchPlan;

#[derive(Debug)]
pub struct PlaceholderMessage {
    pub uuid: Uuid,
    pub action: PlaceholderAction
}

#[derive(Debug)]
pub enum PlaceholderAction {
    Save(LaunchPlan),
    Erase(),
    Launch(),
    /// Launch after starting the plan's container, which the user confirmed in
    /// the prompt `Launch` raises when it finds the container stopped. Distinct
    /// from `Launch` so a plain Launch can never start a container implicitly.
    LaunchStartingContainer(),
}
