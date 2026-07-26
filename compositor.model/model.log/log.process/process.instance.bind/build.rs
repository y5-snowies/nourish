// Compile the shared log streaming proto. The proto lives at the workspace root
// (`compositor.developer/protocol/`), three levels up from this crate.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=../../../protocol/logs.proto");
    tonic_prost_build::configure().compile_protos(&["logs.proto"], &["../../../protocol"])?;
    Ok(())
}
