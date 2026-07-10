// Usage:
//   cargo run --bin client -- --user-id=alice --region=us-east --to=bob
//   cargo run --bin client -- --user-id=bob --region=eu-west --to=alice

use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::io::Write;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[derive(Deserialize, Debug)]
struct RouteResponse {
    region: String,
    ws_url: String,
}

#[derive(Serialize)]
struct OutgoingMessage<'a> {
    to: &'a str,
    content: &'a str,
}

#[derive(Deserialize, Debug)]
struct ServerFrame {
    #[serde(rename = "type")]
    kind: String,
    from: Option<String>,
    from_region: Option<String>,
    content: Option<String>,
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

    println!(
        "Querying Global Traffic Router at {} for region '{}'...",
        args.router, args.region
    );
    let route_url = format!("{}/route?region={}", args.router, args.region);

    let route_resp = reqwest::get(&route_url)
        .await
        .unwrap_or_else(|e| {
            eprintln!("Could not reach the router at {}: {}", args.router, e);
            std::process::exit(1);
        })
        .json::<RouteResponse>()
        .await
        .unwrap_or_else(|e| {
            eprintln!("Router returned an unexpected response: {}", e);
            std::process::exit(1);
        });

    println!(
        "Assigned Regional Server: {} ({})",
        route_resp.ws_url, route_resp.region
    );

    let ws_url = format!("{}?user_id={}", route_resp.ws_url, args.user_id);
    let (ws_stream, _) = connect_async(&ws_url).await.unwrap_or_else(|e| {
        eprintln!("WebSocket connection failed: {}", e);
        std::process::exit(1);
    });

    let (mut ws_sink, mut ws_stream) = ws_stream.split();

    let print_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_stream.next().await {
            if let Message::Text(text) = msg {
                match serde_json::from_str::<ServerFrame>(&text) {
                    Ok(frame) => match frame.kind.as_str() {
                        "connected" => {
                            println!("\n==============================================");
                            println!(" Connected to Regional Chat Network");
                            println!("   Your User ID : {}", frame.user_id.unwrap_or_default());
                            println!("   Your Region  : {}", frame.region.unwrap_or_default());
                            println!("==============================================");
                            println!("Type your message below and press Enter to send:\n");
                            print!("> ");
                            let _ = std::io::stdout().flush();
                        }
                        "message" => {
                            let sender = frame.from.unwrap_or_else(|| "unknown".into());
                            let region = frame
                                .from_region
                                .or(frame.region)
                                .unwrap_or_else(|| "unknown".into());
                            let content = frame.content.unwrap_or_default();
                            println!("\r[{} @ {}]: {}", sender, region, content);
                            print!("> ");
                            let _ = std::io::stdout().flush();
                        }
                        other => {
                            println!("\r[server event]: {}: {}", other, text);
                            print!("> ");
                            let _ = std::io::stdout().flush();
                        }
                    },
                    Err(_) => {
                        println!("\r[raw]: {}", text);
                        print!("> ");
                        let _ = std::io::stdout().flush();
                    }
                }
            }
        }
        println!("\n[disconnected] Connection closed by server.");
    });

    let stdin = BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();
    let to = args.to.clone();

    print!("> ");
    let _ = std::io::stdout().flush();

    while let Ok(Some(line)) = lines.next_line().await {
        let content = line.trim().to_string();
        if content.is_empty() {
            print!("> ");
            let _ = std::io::stdout().flush();
            continue;
        }

        let payload = serde_json::to_string(&OutgoingMessage {
            to: &to,
            content: &content,
        })
        .unwrap();

        if ws_sink.send(Message::Text(payload.into())).await.is_err() {
            eprintln!("Failed to send message: connection closed.");
            break;
        }

        println!("[you -> {}]: {}", to, content);
        print!("> ");
        let _ = std::io::stdout().flush();
    }

    print_task.abort();
}
