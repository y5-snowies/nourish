// Compile the compositor's own `shader.proto`, in place. A copy here would be a
// second definition of the wire format and would eventually disagree with the
// service — the whole reason this tool lives in the same repository.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto = "../../compositor.expansion/compositor.remote/client.protocol";
    println!("cargo:rerun-if-changed={proto}");
    tonic_prost_build::configure()
        // Server codegen is on for the round-trip test, which stands a stub
        // service on a temporary socket and runs the real binary against it. That
        // is the only way to prove the wire end to end without a compositor, and
        // it is what caught nothing so far only because it exists.
        // Every message becomes JSON on stdout — that is the entire output format,
        // so the derive is not a convenience, it is the contract.
        .type_attribute(".", "#[derive(serde::Serialize)]")
        .compile_protos(&["shader/shader.proto"], &[proto])?;
    Ok(())
}
