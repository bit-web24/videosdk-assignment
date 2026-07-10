// Usage:
//   cargo run --bin client -- --user-id=alice --region=us-east --to=bob
//   cargo run --bin client -- --user-id=bob --region=eu-west --to=alice
//
// The client asks the router which WebSocket server handles the requested region,
// then connects and lets you exchange messages in real time.

use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[derive(Deserialize, Debug)]
struct RouteResponse {
    region: String,
    ws_url: String,
}

// JSON shape the server expects for every outgoing message
#[derive(Serialize)]
struct OutgoingMessage<'a> {
    to: &'a str,
    content: &'a str,
}

// JSON shapes the server sends back
#[derive(Deserialize, Debug)]
struct ServerFrame {
    #[serde(rename = "type")]
    kind: String,
    // present on "message" frames
    from: Option<String>,
    content: Option<String>,
    // present on "connected" frames
    user_id: Option<String>,
    region: Option<String>,
}

struct Args {
    user_id: String,
    region: String,
    to: String,
    router: String,
}

fn parse_args() -> Result<Args, String> {
    let raw: Vec<String> = std::env::args().skip(1).collect();

    let mut user_id = None;
    let mut region = None;
    let mut to = None;
    let mut router = "http://localhost:8080".to_string();

    for arg in &raw {
        if let Some(v) = arg.strip_prefix("--user-id=") {
            user_id = Some(v.to_string());
        } else if let Some(v) = arg.strip_prefix("--region=") {
            region = Some(v.to_string());
        } else if let Some(v) = arg.strip_prefix("--to=") {
            to = Some(v.to_string());
        } else if let Some(v) = arg.strip_prefix("--router=") {
            router = v.to_string();
        }
    }

    Ok(Args {
        user_id: user_id.ok_or("--user-id is required")?,
        region: region.ok_or("--region is required")?,
        to: to.ok_or("--to is required. Example: --to=bob")?,
        router,
    })
}

#[tokio::main]
async fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Error: {}", e);
            eprintln!(
                "Usage: client --user-id=<id> --region=<region> --to=<recipient> [--router=<url>]"
            );
            std::process::exit(1);
        }
    };

    // Step 1: ask the router which WebSocket server handles the requested region
    let route_url = format!("{}/route?region={}", args.router, args.region);
    println!("Asking router: {}", route_url);

    let route_resp = reqwest::get(&route_url)
        .await
        .unwrap_or_else(|e| {
            eprintln!("Could not reach the router at {}: {}", args.router, e);
            eprintln!("Is the router running? Try: docker-compose up router");
            std::process::exit(1);
        })
        .json::<RouteResponse>()
        .await
        .unwrap_or_else(|e| {
            eprintln!("Router returned an unexpected response: {}", e);
            std::process::exit(1);
        });

    println!(
        "Router assigned region '{}' -> {}",
        route_resp.region, route_resp.ws_url
    );

    // Step 2: connect to the regional WebSocket server
    let ws_url = format!("{}?user_id={}", route_resp.ws_url, args.user_id);
    println!("Connecting to {}\n", ws_url);

    let (ws_stream, _) = connect_async(&ws_url).await.unwrap_or_else(|e| {
        eprintln!("WebSocket connection failed: {}", e);
        eprintln!("Is the regional server running?");
        std::process::exit(1);
    });

    let (mut ws_sink, mut ws_stream) = ws_stream.split();

    // Step 3: spawn a task that prints every message received from the server
    let print_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_stream.next().await {
            if let Message::Text(text) = msg {
                match serde_json::from_str::<ServerFrame>(&text) {
                    Ok(frame) => match frame.kind.as_str() {
                        "connected" => {
                            println!(
                                "[connected] You are '{}' in region '{}'",
                                frame.user_id.unwrap_or_default(),
                                frame.region.unwrap_or_default()
                            );
                            println!("Type your messages below (press Ctrl+C to exit):\n");
                        }
                        "message" => {
                            println!(
                                "[{}]: {}",
                                frame.from.unwrap_or_else(|| "unknown".into()),
                                frame.content.unwrap_or_default()
                            );
                        }
                        other => {
                            println!("[server] {}: {}", other, text);
                        }
                    },
                    Err(_) => println!("[server] {}", text),
                }
            }
        }
        println!("[disconnected] Server closed the connection.");
    });

    // Step 4: read stdin and send each line as a message to --to user
    let stdin = BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();
    let to = args.to.clone();

    while let Ok(Some(line)) = lines.next_line().await {
        let content = line.trim().to_string();
        if content.is_empty() {
            continue;
        }

        let payload = serde_json::to_string(&OutgoingMessage {
            to: &to,
            content: &content,
        })
        .unwrap();

        if ws_sink.send(Message::Text(payload.into())).await.is_err() {
            eprintln!("Connection lost while sending.");
            break;
        }
    }

    print_task.abort();
}
