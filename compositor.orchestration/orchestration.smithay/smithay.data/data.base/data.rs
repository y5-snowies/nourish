use compositor_orchestration_core_state_base::Loop;
use compositor_support_smithay_dispatch_state_base::state::Dispatch;
use compositor_support_system_storage_slot_base::base::Storage;
use compositor_support_system_storage_token_base::base::{Token, TokenMut};
use smithay::input::keyboard::KeyboardHandle;
use smithay::input::pointer::PointerHandle;
use smithay::reexports::calloop::LoopHandle;
use smithay::reexports::wayland_server::DisplayHandle;

/// KernelData tokens: the smithay support wiring, behind the same token
/// pattern as world storage. Systems READ these (the handles are cloneable
/// "pointers" into the wiring); only the driving layer populates them via
/// [`populate`]. Operations needing `&mut D` (e.g. pointer.motion) remain
/// driver-applied — systems announce intent, the driver performs it.
pub static DISPLAY: Token<DisplayHandle> = Token::new();
pub(crate) static DISPLAY_MUT: TokenMut<DisplayHandle> = TokenMut::new(&DISPLAY);

pub static LOOP_HANDLE: Token<LoopHandle<'static, Loop>> = Token::new();
pub(crate) static LOOP_HANDLE_MUT: TokenMut<LoopHandle<'static, Loop>> = TokenMut::new(&LOOP_HANDLE);

pub static POINTER: Token<PointerHandle<Dispatch>> = Token::new();
pub(crate) static POINTER_MUT: TokenMut<PointerHandle<Dispatch>> = TokenMut::new(&POINTER);

pub static KEYBOARD: Token<KeyboardHandle<Dispatch>> = Token::new();
pub(crate) static KEYBOARD_MUT: TokenMut<KeyboardHandle<Dispatch>> = TokenMut::new(&KEYBOARD);

/// Per-frame screen context — the OUTPUT this pass draws: its physical size,
/// scale and key. Refreshed by the frame driver before every output's systems
/// tick, so a system that keeps something per monitor (a notification pill on
/// each) keys it by `output` without the trait carrying the output.
#[derive(Clone, Debug)]
pub struct ScreenContext {
    pub size: smithay::utils::Size<i32, smithay::utils::Physical>,
    pub scale: f64,
    pub output: std::sync::Arc<str>,
}

pub static SCREEN: Token<ScreenContext> = Token::new();
pub(crate) static SCREEN_MUT: TokenMut<ScreenContext> = TokenMut::new(&SCREEN);

/// Refresh the per-frame screen context (driver-called; upserts).
pub fn update_screen(kernel: &mut Storage, context: ScreenContext) {
    if kernel.contains(&SCREEN) {
        *kernel.get_mut(&SCREEN_MUT) = context;
    } else {
        kernel.insert(&SCREEN, context);
    }
}

/// Populate the kernel store at init (write tokens never leave this crate).
pub fn populate(
    kernel: &mut Storage,
    display: DisplayHandle,
    loop_handle: LoopHandle<'static, Loop>,
    pointer: PointerHandle<Dispatch>,
    keyboard: KeyboardHandle<Dispatch>,
) {
    kernel.insert(&DISPLAY, display);
    kernel.insert(&LOOP_HANDLE, loop_handle);
    kernel.insert(&POINTER, pointer);
    kernel.insert(&KEYBOARD, keyboard);
}
