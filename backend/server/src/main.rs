//! `obot-server`: the always-on "computer" for your bots.
//!
//! Run it on the PC that should host the bots 24/7 (for example a Windows PC). It needs Claude Code
//! installed and logged in on that PC.
//!
//! For safety it only listens on this PC (127.0.0.1). Reach it from your other devices through
//! Tailscale Serve (private HTTPS on your tailnet) or an SSH tunnel, then pair each device with
//! a 6-digit code.
//!
//! Usage:
//!   obot-server                    start (listens on 127.0.0.1:7777)
//!   obot-server --port 8080        use another port
//!   obot-server --host 0.0.0.0 --allow-network
//!                                     listen on the network too (not recommended)
//!   obot-server --data-dir <dir>   keep data somewhere else
//!   obot-server --supervise ...    keep it running, and start the new version after an update
//!                                     from the app (the Windows installer uses this)
//!   obot-server pair               make a 6-digit code to pair a new device (10 minutes, once)
//!   obot-server token              print the owner token (for this PC only)
//!   obot-server new-token          make a new owner token
//!   obot-server devices            list paired devices
//!   obot-server unpair <id>        remove a paired device

use std::path::PathBuf;

use obot_core::config::{default_data_dir, Config};
use obot_core::util::{fix_path_env, lan_ip, random_token};

fn print_pair_code(code: &str) {
    println!();
    println!("       Pairing code:  {} {}", &code[..3], &code[3..]);
    println!("       It works once, for 10 minutes. Need another? Run: obot-server pair");
    println!();
}

fn fmt_ms(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}
use obot_core::{start, ServeOptions};

fn help() {
    println!(
        "obot-server {}\n\n\
         Usage:\n  obot-server [--port 7777] [--data-dir DIR]\n  \
         obot-server --host 0.0.0.0 --allow-network   also listen on the network (not recommended)\n  \
         obot-server --supervise [options]          keep running; start the new version after an update\n  \
         obot-server pair         make a 6-digit code to pair a new device\n  \
         obot-server devices      list paired devices\n  \
         obot-server unpair ID    remove a paired device\n  \
         obot-server token        show the owner token (for this PC only)\n  \
         obot-server new-token    make a new owner token\n\n\
         Reach it from other devices with Tailscale Serve:  tailscale serve --bg 7777",
        obot_core::VERSION
    );
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // Claude Code starts us with `mcp-approve` to ask the user for permission.
    if args.get(1).map(String::as_str) == Some("mcp-approve") {
        return obot_core::mcp::run_approval_mcp();
    }
    // ...and with `hook <event>` at certain moments of a run (see hooks.rs).
    if args.get(1).map(String::as_str) == Some("hook") {
        std::process::exit(obot_core::hooks::run(&args[2..]));
    }

    // The startup task runs `obot-server --supervise ...`: keep the server running and start the
    // new version after an update from the app (see update.rs).
    if let Some(pos) = args.iter().position(|a| a == "--supervise") {
        let mut rest = args[1..].to_vec();
        rest.remove(pos - 1);
        obot_core::update::supervise(rest);
    }
    obot_core::update::clean_old();

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
            let (code, _) = obot_core::security::new_pairing_code(&dir)?;
            print_pair_code(&code);
            return Ok(());
        }
        Some("devices") => {
            let cfg = Config::load_or_create(&dir.join("config.json"))?;
            if cfg.devices.is_empty() {
                println!("No paired devices. Run `obot-server pair` to add one.");
            }
            for d in &cfg.devices {
                println!("{}  {}  (paired {})", d.id, d.name, fmt_ms(d.created_at));
            }
            return Ok(());
        }
        Some("unpair") => {
            let Some(id) = rest.first() else {
                eprintln!("Usage: obot-server unpair <device id>   (see `obot-server devices`)");
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
            obot_core::push::forget_device(&mut cfg, id);
            cfg.save(&path)?;
            println!("Removed. Restart obot-server if it's running so the change takes effect now.");
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
        eprintln!("  !!  WARNING: --host {host} makes oBot reachable from other computers on the network.");
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
        let claude = obot_core::computer::claude_info("").await;

        println!();
        println!("  oBot computer is running (version {})", obot_core::VERSION);
        println!();
        if !loopback {
            if let Some(ip) = lan_ip() {
                println!("  Address on your network:  http://{ip}:{port}   (not encrypted!)");
            }
        }
        println!("  Address on this PC:       http://127.0.0.1:{port}");
        // The sign-in link and pairing code are secrets: shown in a terminal, but not written to the
        // log file when obot-server runs in the background.
        let terminal = std::io::IsTerminal::is_terminal(&std::io::stdout());
        if terminal {
            println!("  Open it on this PC:       http://127.0.0.1:{port}/?token={}", server.token);
        } else {
            println!("  Open it on this PC:       run `obot-server token` for the sign-in key (it isn't written here)");
        }
        println!();
        println!("  To use it from your other devices:");
        println!("    1. Install Tailscale on this PC and the other device, and sign in to both.");
        println!("    2. On this PC run:  tailscale serve --bg {port}");
        println!("       It prints a private https://...ts.net address only your devices can open.");
        println!("    3. In the oBot app: Settings > Computer > Remote computer, enter that address");
        println!("       and this pairing code:");
        if terminal {
            match obot_core::security::new_pairing_code(&server.state.data_dir) {
                Ok((code, _)) => print_pair_code(&code),
                Err(e) => println!("       (couldn't make a pairing code: {e})"),
            }
        } else {
            println!("       a pairing code from `obot-server pair` (or Settings > Computer > Pair a new device).");
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
        println!("\nStopping oBot.");
        anyhow::Ok(())
    })
}
