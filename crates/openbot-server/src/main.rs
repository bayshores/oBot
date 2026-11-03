//! `openbot-server`: the always-on "computer" for your bots.
//!
//! Run it on the PC that should host the bots 24/7 (for example a Windows PC). It needs Claude Code
//! installed and logged in on that PC. Then pair the openbot app with the address + token it prints.
//!
//! Usage:
//!   openbot-server                    start (listens on 0.0.0.0:7777)
//!   openbot-server --port 8080        use another port
//!   openbot-server --local-only       only accept connections from this same PC
//!   openbot-server --data-dir <dir>   keep data somewhere else
//!   openbot-server token              print the pairing token
//!   openbot-server new-token          make a new token (old pairings stop working)

use std::path::PathBuf;

use openbot_core::config::{default_data_dir, Config};
use openbot_core::util::{fix_path_env, lan_ip, random_token};
use openbot_core::{start, ServeOptions};

fn help() {
    println!(
        "openbot-server {}\n\n\
         Usage:\n  openbot-server [--port 7777] [--local-only] [--data-dir DIR]\n  \
         openbot-server token        show the pairing token\n  \
         openbot-server new-token    make a new pairing token",
        openbot_core::VERSION
    );
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // Claude Code starts us with `mcp-approve` to ask the user for permission.
    if args.get(1).map(String::as_str) == Some("mcp-approve") {
        return openbot_core::mcp::run_approval_mcp();
    }

    let mut port: u16 = 7777;
    let mut host = "0.0.0.0".to_string();
    let mut data_dir: Option<PathBuf> = None;
    let mut command: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--port" | "-p" => {
                i += 1;
                port = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(7777);
            }
            "--local-only" => host = "127.0.0.1".into(),
            "--host" => {
                i += 1;
                host = args.get(i).cloned().unwrap_or(host);
            }
            "--data-dir" => {
                i += 1;
                data_dir = args.get(i).map(PathBuf::from);
            }
            "-h" | "--help" => {
                help();
                return Ok(());
            }
            other if !other.starts_with('-') => command = Some(other.to_string()),
            other => {
                eprintln!("Unknown option: {other}\n");
                help();
                std::process::exit(2);
            }
        }
        i += 1;
    }

    let dir = data_dir.clone().unwrap_or_else(default_data_dir);
    match command.as_deref() {
        Some("token") => {
            let cfg = Config::load_or_create(&dir.join("config.json"))?;
            println!("{}", cfg.token);
            return Ok(());
        }
        Some("new-token") => {
            let path = dir.join("config.json");
            let mut cfg = Config::load_or_create(&path)?;
            cfg.token = random_token(32);
            cfg.save(&path)?;
            println!("New token: {}", cfg.token);
            return Ok(());
        }
        Some(other) => {
            eprintln!("Unknown command: {other}\n");
            help();
            std::process::exit(2);
        }
        None => {}
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    fix_path_env();

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async move {
        let server = start(ServeOptions { host: host.clone(), port, data_dir }).await?;
        let port = server.addr.port();
        let claude = openbot_core::computer::claude_info("").await;

        println!();
        println!("  openbot computer is running (version {})", openbot_core::VERSION);
        println!();
        if host == "0.0.0.0" {
            if let Some(ip) = lan_ip() {
                println!("  Address on your network:  http://{ip}:{port}");
            }
            if let Some(name) = sysinfo::System::host_name() {
                println!("  Address by name:          http://{}:{port}", name.to_lowercase());
            }
        }
        println!("  Address on this PC:       http://127.0.0.1:{port}");
        println!("  Pairing token:            {}", server.token);
        println!();
        println!("  In the openbot app: Settings > Computer > Remote computer, paste an address + the token.");
        println!("  Or open  http://127.0.0.1:{port}/?token={}  in a browser on this PC.", server.token);
        println!();
        if claude["installed"] != true {
            println!("  WARNING: Claude Code was not found. Install it, then run `claude` once and log in.");
        } else if claude["logged_in"] != true {
            println!("  WARNING: Claude Code is not logged in. Run `claude` once and log in,");
            println!("           or add an Anthropic API key in Settings > Claude Account.");
        } else {
            println!(
                "  Claude Code {} — logged in as {} ({})",
                claude["version"].as_str().unwrap_or(""),
                claude["email"].as_str().unwrap_or(""),
                claude["subscription"].as_str().unwrap_or("")
            );
        }
        println!();
        println!("  Keep this window open (or install it as a background task: see README). Ctrl+C stops it.");

        tokio::signal::ctrl_c().await?;
        println!("\nStopping openbot.");
        anyhow::Ok(())
    })
}
