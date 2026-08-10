//! `y5-shader` — read and drive the running compositor's background shader.
//!
//! A thin client over the gRPC `Shader` service on the compositor's Unix socket.
//! Built for an agent: every command prints ONE JSON object on stdout and nothing
//! else, so the output parses without being scraped.
//!
//! ```text
//! y5-shader active                  what the focused world is running
//! y5-shader list                    every bundle that can be activated
//! y5-shader activate <selection>    switch to one ("" = the built-in)
//! y5-shader reload                  re-read the current bundle from disk
//! ```
//!
//! # Exit codes
//!
//! * `0` — the call completed. That is not "the shader works": read `error` in
//!   the JSON, which is the compositor's answer rather than this tool's.
//! * `1` — could not reach the compositor, or it refused the call.
//! * `2` — the command line was wrong.
//!
//! The split matters for a caller in a loop: `1` means try again or give up, and
//! a populated `error` at exit `0` means fix the shader.
//!
//! # The edit loop
//!
//! ```text
//! y5-shader active | jq -r .path     # where the sources are ("" ⇒ compiled in)
//! …edit the bundle…
//! y5-shader reload
//! y5-shader active | jq -r .error    # empty ⇒ it compiled
//! ```
//!
//! `activate` and `reload` return before the bundle is compiled — the load happens
//! on the compositor's next frame. So the compile result is always read back with
//! a second `active` call, never inferred from the first response.

use std::path::Path;
use tonic::transport::{Endpoint, Uri};

pub mod bind {
    tonic::include_proto!("y5.compositor.rpc.protocol.client.shader");
}
use bind::shader_client::ShaderClient;

/// Where the compositor binds. Matches `remote.transport::SOCKET_PATH`; override
/// with `Y5_RPC_SOCKET` for a nested or non-default session.
const SOCKET: &str = "/tmp/y5-compositor-rpc.sock";

fn socket_path() -> String {
    std::env::var("Y5_RPC_SOCKET").unwrap_or_else(|_| SOCKET.to_string())
}

/// Print a JSON object and leave. Errors go to stdout too, in the same shape, so
/// a caller has exactly one thing to parse whatever happened.
fn fail(code: i32, message: &str) -> ! {
    println!("{}", serde_json::json!({ "error": message }));
    std::process::exit(code)
}

fn emit<T: serde::Serialize>(value: &T) {
    match serde_json::to_string_pretty(value) {
        Ok(s) => println!("{s}"),
        Err(e) => fail(1, &format!("could not serialise the response: {e}")),
    }
}

const USAGE: &str = "usage: y5-shader <active | list | activate <selection> | reload>";

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("");

    // Validate the command line BEFORE dialling, so a typo does not present as a
    // connection problem.
    let selection = match command {
        "active" | "list" | "reload" => String::new(),
        // A missing argument is an error; an EMPTY one is meaningful — it clears
        // the world's override back to the built-in.
        "activate" => match args.get(1) {
            Some(s) => s.clone(),
            None => fail(2, "activate needs a selection (\"\" selects the built-in)"),
        },
        "-h" | "--help" | "help" => {
            println!("{}", serde_json::json!({ "usage": USAGE, "socket": socket_path() }));
            return;
        }
        other => fail(2, &format!("unknown command {other:?}. {USAGE}")),
    };

    let path = socket_path();
    if !Path::new(&path).exists() {
        fail(1, &format!("no compositor socket at {path} — is y5 running?"));
    }

    // The URI is ignored: `connect_with_connector` dials the Unix socket instead.
    // It still has to parse, and tonic still uses its authority for the HTTP/2
    // `:authority` header, so it cannot simply be blank.
    let channel = match Endpoint::from_static("http://[::1]:0")
        .connect_with_connector(tower::service_fn(move |_: Uri| {
            let path = path.clone();
            async move {
                let stream = tokio::net::UnixStream::connect(path).await?;
                Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(stream))
            }
        }))
        .await
    {
        Ok(c) => c,
        Err(e) => fail(1, &format!("could not connect to {}: {e}", socket_path())),
    };
    let mut client = ShaderClient::new(channel);

    let result = match command {
        "active" => client.active(bind::ActiveRequest {}).await.map(|r| emit(r.get_ref())),
        "list" => client.list(bind::ListRequest {}).await.map(|r| emit(r.get_ref())),
        "reload" => client.reload(bind::ReloadRequest {}).await.map(|r| emit(r.get_ref())),
        "activate" => client
            .activate(bind::ActivateRequest { selection })
            .await
            .map(|r| emit(r.get_ref())),
        _ => unreachable!("the command was validated above"),
    };
    if let Err(status) = result {
        fail(1, &format!("{}: {}", status.code(), status.message()));
    }
}
