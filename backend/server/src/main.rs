//! `openbot-server`: the always-on "computer" for your bots.
//!
//! Run it on the PC that should host the bots 24/7 (for example a Windows PC). It needs Claude Code
//! installed and logged in on that PC.
//!
//! For safety it only listens on this PC (127.0.0.1). Reach it from your other devices through
//! Tailscale Serve (private HTTPS on your tailnet) or an SSH tunnel, then pair each device with
//! a 6-digit code.
//!
//! Usage:
//!   openbot-server                    start (listens on 127.0.0.1:7777)
//!   openbot-server --port 8080        use another port
//!   openbot-server --host 0.0.0.0 --allow-network
//!                                     listen on the network too (not recommended)
//!   openbot-server --data-dir <dir>   keep data somewhere else
//!   openbot-server pair               make a 6-digit code to pair a new device (10 minutes, once)
//!   openbot-server token              print the owner token (for this PC only)
//!   openbot-server new-token          make a new owner token
//!   openbot-server devices            list paired devices
//!   openbot-server unpair <id>        remove a paired device

use std::path::PathBuf;

use openbot_core::config::{default_data_dir, Config};
use openbot_core::util::{fix_path_env, lan_ip, random_token};

fn print_pair_code(code: &str) {
    println!();
    println!("       Pairing code:  {} {}", &code[..3], &code[3..]);
    println!("       It works once, for 10 minutes. Need another? Run: openbot-server pair");
    println!();
}

fn fmt_ms(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}
use openbot_core::{start, ServeOptions};

fn help() {
    println!(
        "openbot-server {}\n\n\
         Usage:\n  openbot-server [--port 7777] [--data-dir DIR]\n  \
         openbot-server --host 0.0.0.0 --allow-network   also listen on the network (not recommended)\n  \
         openbot-server pair         make a 6-digit code to pair a new device\n  \
         openbot-server devices      list paired devices\n  \
         openbot-server unpair ID    remove a paired device\n  \
         openbot-server token        show the owner token (for this PC only)\n  \
         openbot-server new-token    make a new owner token\n\n\
         Reach it from other devices with Tailscale Serve:  tailscale serve --bg 7777",
        openbot_core::VERSION
    );
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // Claude Code starts us with `mcp-approve` to ask the user for permission.
    if args.get(1).map(String::as_str) == Some("mcp-approve") {
        return openbot_core::mcp::run_approval_mcp();
    }
    // ...and with `hook <event>` at certain moments of a run (see hooks.rs).
    if args.get(1).map(String::as_str) == Some("hook") {
        std::process::exit(openbot_core::hooks::run(&args[2..]));
    }

    let mut port: u16 = 7777;
    let mut host = "127.0.0.1".to_string();
    let mut allow_network = false;
    let mut rest: Vec<String> = vec![];
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
            "--allow-network" => allow_network = true,
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
            other if !other.starts_with('-') => {
                if command.is_none() {
                    command = Some(other.to_string());
                } else {
                    rest.push(other.to_string());
                }
            }
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
        Some("pair") => {
            let (code, _) = openbot_core::security::new_pairing_code(&dir)?;
            print_pair_code(&code);
            return Ok(());
        }
        Some("devices") => {
            let cfg = Config::load_or_create(&dir.join("config.json"))?;
            if cfg.devices.is_empty() {
                println!("No paired devices. Run `openbot-server pair` to add one.");
            }
            for d in &cfg.devices {
                println!("{}  {}  (paired {})", d.id, d.name, fmt_ms(d.created_at));
            }
            return Ok(());
        }
        Some("unpair") => {
            let Some(id) = rest.first() else {
                eprintln!("Usage: openbot-server unpair <device id>   (see `openbot-server devices`)");
                std::process::exit(2);
            };
            let path = dir.join("config.json");
            let mut cfg = Config::load_or_create(&path)?;
            let before = cfg.devices.len();
            cfg.devices.retain(|d| &d.id != id);
            if cfg.devices.len() == before {
                eprintln!("No device with id {id}");
                std::process::exit(1);
            }
            cfg.save(&path)?;
            println!("Removed. Restart openbot-server if it's running so the change takes effect now.");
            return Ok(());
        }
        Some(other) => {
            eprintln!("Unknown command: {other}\n");
            help();
            std::process::exit(2);
        }
        None => {}
    }

    let loopback = host == "localhost" || host.parse::<std::net::IpAddr>().map_or(false, |i| i.is_loopback());
    if !loopback {
        eprintln!();
        eprintln!("  !!  WARNING: --host {host} makes openbot reachable from other computers on the network.");
        eprintln!("  !!  Anyone who can reach this port can try to get in, and the traffic is NOT encrypted.");
        eprintln!("  !!  Safer: keep the default (127.0.0.1) and use Tailscale Serve:  tailscale serve --bg {port}");
        eprintln!();
        if !allow_network {
            eprintln!("  Not starting. If you really want this, add --allow-network.");
            std::process::exit(2);
        }
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
        if !loopback {
            if let Some(ip) = lan_ip() {
                println!("  Address on your network:  http://{ip}:{port}   (not encrypted!)");
            }
        }
        println!("  Address on this PC:       http://127.0.0.1:{port}");
        println!("  Open it on this PC:       http://127.0.0.1:{port}/?token={}", server.token);
        println!();
        println!("  To use it from your other devices:");
        println!("    1. Install Tailscale on this PC and the other device, and sign in to both.");
        println!("    2. On this PC run:  tailscale serve --bg {port}");
        println!("       It prints a private https://...ts.net address only your devices can open.");
        println!("    3. In the openbot app: Settings > Computer > Remote computer, enter that address");
        println!("       and this pairing code:");
        match openbot_core::security::new_pairing_code(&server.state.data_dir) {
            Ok((code, _)) => print_pair_code(&code),
            Err(e) => println!("       (couldn't make a pairing code: {e})"),
        }
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
