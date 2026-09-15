use bluer::rfcomm::{Listener, SocketAddr};
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use log::{error, info};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::Message;

#[derive(Parser)]
#[command(name = "rfcomm-relay", about = "RFCOMM to Nostr relay bridge")]
struct Cli {
    /// Run in echo mode (reflect received data back)
    #[arg(long)]
    echo: bool,

    /// WebSocket relay URL (default mode)
    #[arg(long, default_value = "ws://localhost:7777")]
    relay: String,

    /// RFCOMM channel number (0 = auto-assign)
    #[arg(long, default_value = "3")]
    channel: u8,
}

#[tokio::main]
async fn main() -> bluer::Result<()> {
    env_logger::init();
    let cli = Cli::parse();

    let session = bluer::Session::new().await?;
    let adapter = session.default_adapter().await?;
    adapter.set_powered(true).await?;

    let addr = SocketAddr::new(adapter.address().await?, cli.channel);
    let listener = Listener::bind(addr).await?;
    let local = listener.as_ref().local_addr()?;

    let mode = if cli.echo { "echo" } else { "relay" };
    info!("Listening on {} channel {} in {} mode", local.addr, local.channel, mode);

    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(v) => v,
            Err(e) => {
                error!("Accept failed: {}", e);
                continue;
            }
        };

        info!("Connection from {}", peer);

        if cli.echo {
            tokio::spawn(handle_echo(stream, peer.addr));
        } else {
            let relay_url = cli.relay.clone();
            tokio::spawn(handle_relay(stream, peer.addr, relay_url));
        }
    }
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
    info!("[{}] Relay session started (JSONL framing)", device);

    let mut buf = vec![0u8; 65536];
    let mut line_buf = String::new();

    loop {
        tokio::select! {
            result = rfcomm.read(&mut buf) => {
                match result {
                    Ok(0) => {
                        info!("[{}] RFCOMM disconnected", device);
                        break;
                    }
                    Ok(n) => {
                        let data = String::from_utf8_lossy(&buf[..n]);
                        line_buf.push_str(&data);

                        while let Some(pos) = line_buf.find('\n') {
                            let line = line_buf[..pos].trim().to_string();
                            line_buf = line_buf[pos + 1..].to_string();

                            if line.is_empty() {
                                continue;
                            }

                            info!("[{}] RFCOMM -> WS: {}", device, line);
                            if let Err(e) = ws_sink.send(Message::Text(line)).await {
                                error!("[{}] WS send error: {}", device, e);
                                return;
                            }
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
                        info!("[{}] WS -> RFCOMM: {}", device, text);
                        let mut line = text;
                        line.push('\n');
                        if let Err(e) = rfcomm.write_all(line.as_bytes()).await {
                            error!("[{}] RFCOMM write error: {}", device, e);
                            break;
                        }
                    }
                    Some(Ok(Message::Binary(data))) => {
                        info!("[{}] WS -> RFCOMM: {} bytes (binary, dropped)", device, data.len());
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
