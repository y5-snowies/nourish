//! CLI verifier + reference client: connect to the compositor's log unix socket and print
//! streamed records. Run while the compositor is up:
//!   cargo run -p compositor_model_log_process_instance_example
//! Optionally pass a max count as the first argument.

use compositor_model_log_process_instance::SOCKET;
use compositor_model_log_process_instance::bind;
use compositor_model_log_process_instance::bind::log_stream_client::LogStreamClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let max: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(usize::MAX);

    // tonic over a unix socket: the URI is ignored; the connector dials the socket.
    let path = SOCKET.to_string();
    let channel = tonic::transport::Endpoint::try_from("http://[::]:50051")?
        .connect_with_connector(tower::service_fn(move |_| {
            let path = path.clone();
            async move {
                let stream = tokio::net::UnixStream::connect(path).await?;
                Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(stream))
            }
        }))
        .await?;

    let mut client = LogStreamClient::new(channel);
    let mut stream = client.stream(bind::StreamRequest {}).await?.into_inner();

    let levels = ["ERROR", "WARN", "INFO", "TRACE"];
    let mut n = 0usize;
    while let Some(rec) = stream.message().await? {
        let lvl = levels.get(rec.level as usize).copied().unwrap_or("?");
        println!(
            "CLIENT [{:>5}.{:06}] {} {}::{}: {}",
            rec.elapsed_micros / 1_000_000,
            rec.elapsed_micros % 1_000_000,
            lvl,
            rec.crate_name,
            rec.function,
            rec.message,
        );
        n += 1;
        if n >= max {
            break;
        }
    }
    Ok(())
}
