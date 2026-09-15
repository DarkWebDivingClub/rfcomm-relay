use bluer::rfcomm::{Profile, Role};
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use log::{error, info};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

const SERVICE_UUID: &str = "fe31ece6-dd02-4279-8c42-b199a212df2c";

#[derive(Parser)]
#[command(name = "rfcomm-relay", about = "RFCOMM to Nostr relay bridge")]
struct Cli {
    /// Run in echo mode (reflect received data back)
    #[arg(long)]
    echo: bool,

    /// WebSocket relay URL (default mode)
    #[arg(long, default_value = "ws://localhost:7777")]
    relay: String,
}

#[tokio::main]
async fn main() -> bluer::Result<()> {
    env_logger::init();
    let cli = Cli::parse();

    let session = bluer::Session::new().await?;
    let adapter = session.default_adapter().await?;
    adapter.set_powered(true).await?;
    adapter.set_discoverable(true).await?;

    let uuid: Uuid = SERVICE_UUID.parse().expect("invalid UUID");
    let profile = Profile {
        uuid,
        name: Some("rfcomm-relay".into()),
        role: Some(Role::Server),
        require_authentication: Some(false),
        require_authorization: Some(false),
        auto_connect: Some(true),
        ..Default::default()
    };

    let mut handle = session.register_profile(profile).await?;

    let mode = if cli.echo { "echo" } else { "relay" };
    info!("Registered RFCOMM service {} in {} mode", SERVICE_UUID, mode);
    info!("Waiting for connections...");

    while let Some(req) = handle.next().await {
        let device = req.device();
        info!("Connection from {}", device);

        let stream = match req.accept() {
            Ok(s) => s,
            Err(e) => {
                error!("Failed to accept connection from {}: {}", device, e);
                continue;
            }
        };

        if cli.echo {
            tokio::spawn(handle_echo(stream, device));
        } else {
            let relay_url = cli.relay.clone();
            tokio::spawn(handle_relay(stream, device, relay_url));
        }
    }

    Ok(())
}

async fn handle_echo(mut stream: bluer::rfcomm::Stream, device: bluer::Address) {
    info!("[{}] Echo session started", device);
    let mut buf = vec![0u8; 4096];

    loop {
        let n = match stream.read(&mut buf).await {
            Ok(0) => {
                info!("[{}] Disconnected", device);
                break;
            }
            Ok(n) => n,
            Err(e) => {
                error!("[{}] Read error: {}", device, e);
                break;
            }
        };

        let data = &buf[..n];
        info!("[{}] Echo {} bytes", device, n);

        if let Err(e) = stream.write_all(data).await {
            error!("[{}] Write error: {}", device, e);
            break;
        }
    }

    info!("[{}] Echo session ended", device);
}

async fn handle_relay(
    mut rfcomm: bluer::rfcomm::Stream,
    device: bluer::Address,
    relay_url: String,
) {
    info!("[{}] Connecting to relay {}", device, relay_url);

    let (ws, _) = match tokio_tungstenite::connect_async(&relay_url).await {
        Ok(v) => v,
        Err(e) => {
            error!("[{}] Failed to connect to relay: {}", device, e);
            return;
        }
    };

    let (mut ws_sink, mut ws_stream) = ws.split();
    info!("[{}] Relay session started", device);

    let mut buf = vec![0u8; 65536];

    loop {
        tokio::select! {
            result = rfcomm.read(&mut buf) => {
                match result {
                    Ok(0) => {
                        info!("[{}] RFCOMM disconnected", device);
                        break;
                    }
                    Ok(n) => {
                        let data = &buf[..n];
                        info!("[{}] RFCOMM -> WS: {} bytes", device, n);
                        if let Err(e) = ws_sink.send(Message::Text(
                            String::from_utf8_lossy(data).into_owned()
                        )).await {
                            error!("[{}] WS send error: {}", device, e);
                            break;
                        }
                    }
                    Err(e) => {
                        error!("[{}] RFCOMM read error: {}", device, e);
                        break;
                    }
                }
            }
            msg = ws_stream.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        info!("[{}] WS -> RFCOMM: {} bytes", device, text.len());
                        if let Err(e) = rfcomm.write_all(text.as_bytes()).await {
                            error!("[{}] RFCOMM write error: {}", device, e);
                            break;
                        }
                    }
                    Some(Ok(Message::Binary(data))) => {
                        info!("[{}] WS -> RFCOMM: {} bytes (binary)", device, data.len());
                        if let Err(e) = rfcomm.write_all(&data).await {
                            error!("[{}] RFCOMM write error: {}", device, e);
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        info!("[{}] WebSocket closed", device);
                        break;
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        error!("[{}] WS read error: {}", device, e);
                        break;
                    }
                }
            }
        }
    }

    info!("[{}] Relay session ended", device);
}
