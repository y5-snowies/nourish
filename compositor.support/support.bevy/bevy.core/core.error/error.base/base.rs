use compositor_support_bevy_core_fault_base::SurfaceError;
use compositor_support_bevy_core_handle_base::HandleId;
use thiserror;

#[derive(Debug, thiserror::Error)]
pub enum CreateError {
    #[error("surface allocation: {0}")]
    Surface(#[from] SurfaceError),
}

#[derive(Debug, thiserror::Error)]
pub enum ResizeError {
    #[error("surface resize: {0}")]
    Surface(#[from] SurfaceError),

    #[error("handle not registered: {0:?}")]
    UnknownHandle(HandleId),
}

#[derive(Debug, thiserror::Error)]
pub enum DispatchError {
    #[error("handle not registered: {0:?}")]
    UnknownHandle(HandleId),

    #[error("command dispatch: handle's scene type doesn't match the dispatched command type")]
    TypeMismatch,
}
