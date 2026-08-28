// Every spawn goes through the hygiene wrapper — see `process.child`.
use compositor_support_library_process_child_hygiene::hygiene;
use compositor_support_library_process_child_spawn::spawn as child_spawn;

pub fn init_logging() {
    if let Ok(env_filter) = tracing_subscriber::EnvFilter::try_from_default_env() {
        tracing_subscriber::fmt().with_env_filter(env_filter).init();
    } else {
        tracing_subscriber::fmt().init();
    }
}

pub fn spawn_client() {
    let mut args = std::env::args().skip(1);
    let flag = args.next();
    let arg = args.next();

    match (flag.as_deref(), arg) {
        (Some("-c") | Some("--command"), Some(program)) => {
            child_spawn::spawn(&mut hygiene::command(program)).ok();
        }
        _ => {
            child_spawn::spawn(&mut hygiene::command("foot")).ok();        }
        // std::process::Command::new("weston-terminal").spawn().ok();        }
    }
}
