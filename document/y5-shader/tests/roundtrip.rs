//! The wire, end to end: a stub `Shader` service on a temporary socket, and the
//! REAL binary run against it.
//!
//! Everything else about this tool is testable by inspection; this is not. The
//! proto has to compile to the same types the compositor serves, the Unix-socket
//! dialling has to work through tonic's connector, and the output has to be one
//! parseable JSON object. A mistake in any of the three looks the same from
//! outside — "it printed nothing useful" — and none of them shows up until there
//! is a compositor to talk to.

use std::process::Command;
use tokio::net::UnixListener;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::Server;

pub mod bind {
    tonic::include_proto!("y5.compositor.rpc.protocol.client.shader");
}
use bind::shader_server::{Shader, ShaderServer};

#[derive(Default)]
struct Stub;

#[tonic::async_trait]
impl Shader for Stub {
    async fn active(
        &self,
        _r: tonic::Request<bind::ActiveRequest>,
    ) -> Result<tonic::Response<bind::ActiveResponse>, tonic::Status> {
        Ok(tonic::Response::new(bind::ActiveResponse {
            selection: "tb-storage-embers".into(),
            path: "/home/u/.local/share/y5/background/shader/tb-storage-embers".into(),
            builtin: false,
            error: String::new(),
            before_passes: 0,
            after_passes: 3,
            targets: 1,
            requires: vec!["composited_scene".into(), "pointer_state".into()],
            windows: "engine".into(),
            props: vec![bind::Prop {
                name: "pull".into(),
                kind: "float".into(),
                label: "Cursor pull".into(),
                group: "Embers".into(),
                value: 0.8,
                default: 0.55,
                min: 0.0,
                max: 2.0,
                step: 0.01,
                choices: vec![],
            }],
        }))
    }

    async fn list(
        &self,
        _r: tonic::Request<bind::ListRequest>,
    ) -> Result<tonic::Response<bind::ListResponse>, tonic::Status> {
        Ok(tonic::Response::new(bind::ListResponse {
            bundles: vec![bind::Bundle {
                selection: "builtin:mp-crt".into(),
                name: "mp-crt".into(),
                category: "Multipass".into(),
                builtin: true,
                path: String::new(),
                multipass: true,
            }],
        }))
    }

    async fn activate(
        &self,
        r: tonic::Request<bind::ActivateRequest>,
    ) -> Result<tonic::Response<bind::ActivateResponse>, tonic::Status> {
        // Echo the selection back through `error` so the test can prove the
        // REQUEST crossed the wire, not merely that a response came back.
        Ok(tonic::Response::new(bind::ActivateResponse {
            applied: true,
            error: r.into_inner().selection,
        }))
    }

    async fn reload(
        &self,
        _r: tonic::Request<bind::ReloadRequest>,
    ) -> Result<tonic::Response<bind::ReloadResponse>, tonic::Status> {
        Ok(tonic::Response::new(bind::ReloadResponse {
            applied: false,
            error: "this world is on the built-in shader; nothing to reload".into(),
        }))
    }
}

fn run(socket: &str, args: &[&str]) -> (String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_y5-shader"))
        .args(args)
        .env("Y5_RPC_SOCKET", socket)
        .output()
        .expect("the binary runs");
    (String::from_utf8_lossy(&out.stdout).into_owned(), out.status.code().unwrap_or(-1))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_command_round_trips_and_prints_one_json_object() {
    let socket = std::env::temp_dir().join("y5-shader-cli-test.sock");
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).expect("bind");
    tokio::spawn(async move {
        let _ = Server::builder()
            .add_service(ShaderServer::new(Stub))
            .serve_with_incoming(UnixListenerStream::new(listener))
            .await;
    });
    let path = socket.to_string_lossy().into_owned();

    let (out, code) = run(&path, &["active"]);
    assert_eq!(code, 0, "active exited {code}: {out}");
    let v: serde_json::Value = serde_json::from_str(&out).expect("active prints one JSON object");
    assert_eq!(v["selection"], "tb-storage-embers");
    // `path` is what an agent needs to find the files it must edit.
    assert!(v["path"].as_str().expect("path").ends_with("tb-storage-embers"));
    // Keys are the PROTO field names verbatim — `after_passes`, not `afterPasses`.
    // serde's derive over the prost types carries the Rust field names through, so
    // the schema a caller reads and the JSON it parses use one spelling.
    assert_eq!(v["after_passes"], 3, "graph shape must survive the wire");
    assert!(v.get("afterPasses").is_none(), "keys must not be camel-cased");
    assert_eq!(v["requires"][0], "composited_scene");
    assert_eq!(v["props"][0]["name"], "pull");
    assert_eq!(v["props"][0]["value"], 0.8, "the LIVE value, not the default");
    assert_eq!(v["props"][0]["default"], 0.55);

    let (out, code) = run(&path, &["list"]);
    assert_eq!(code, 0, "list exited {code}: {out}");
    let v: serde_json::Value = serde_json::from_str(&out).expect("list prints one JSON object");
    assert_eq!(v["bundles"][0]["selection"], "builtin:mp-crt");
    assert_eq!(v["bundles"][0]["builtin"], true);

    // The request direction: the stub echoes what it received.
    let (out, code) = run(&path, &["activate", "tb-chain-bleed"]);
    assert_eq!(code, 0, "activate exited {code}: {out}");
    let v: serde_json::Value = serde_json::from_str(&out).expect("activate prints JSON");
    assert_eq!(v["applied"], true);
    assert_eq!(v["error"], "tb-chain-bleed", "the selection did not reach the server");

    // An empty selection is a legal request, not a missing argument.
    let (out, code) = run(&path, &["activate", ""]);
    assert_eq!(code, 0, "activate \"\" exited {code}: {out}");
    let v: serde_json::Value = serde_json::from_str(&out).expect("activate prints JSON");
    assert_eq!(v["error"], "", "an empty selection must cross the wire as empty");

    // A refusal is still exit 0 — the CALL worked. The distinction is the whole
    // reason a caller can tell "try again" from "fix the shader".
    let (out, code) = run(&path, &["reload"]);
    assert_eq!(code, 0, "a compositor-side refusal must not be a transport failure");
    let v: serde_json::Value = serde_json::from_str(&out).expect("reload prints JSON");
    assert_eq!(v["applied"], false);
    assert!(v["error"].as_str().expect("error").contains("nothing to reload"));

    let _ = std::fs::remove_file(&socket);
}
