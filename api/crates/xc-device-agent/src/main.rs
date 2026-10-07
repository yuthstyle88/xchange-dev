//! CLI for xc-device-agent.

use std::time::Duration;

use xc_device_agent::client::{AgentApi, DrawerInfo, HttpAgentApi, PairRequest};
use xc_device_agent::config::{
    BUILD, BUILT_AT, ConfigFile, VERSION, chown_to_service_user, current_host, current_os,
    default_config_dir, install_service, load_config, load_token, save_config, save_token,
    uninstall_service,
};
use xc_device_agent::cups_transport::CupsTransport;
use xc_device_agent::journal::Journal;
use xc_device_agent::runner::{AgentConfig, AgentRunner, Step};
use xc_device_agent::tcp_transport::TcpTransport9100;
use xc_device_agent::transport::DrawerTransport;

enum CliCommand {
    Version,
    Install {
        server: String,
        code: String,
        drawer: DrawerInfo,
        channel: String,
        no_service: bool,
    },
    Run,
    Uninstall,
    Status,
}

fn print_version(channel: Option<&str>) {
    let ch = channel.unwrap_or("stable");
    println!("version: {VERSION}");
    println!("build: {BUILD}");
    println!("builtAt: {BUILT_AT}");
    println!("channel: {ch}");
}

fn print_help() {
    println!(
        "Usage: xc-device-agent <COMMAND> [OPTIONS]\n\n\
        Commands:\n  \
          install    Pair with server and install background service\n  \
          run        Run agent in foreground\n  \
          uninstall  Remove service and local credentials\n  \
          status     Show configuration and pairing status\n  \
          --version  Show version and build information\n\n\
        Install options:\n  \
          --server <URL>        Server URL (https://<host>)\n  \
          --code <CODE>         Pairing code (XXXXX-XXXXX)\n  \
          --cups-queue <QUEUE>  CUPS printer queue name\n  \
          --tcp <HOST:PORT>     Network printer address (e.g. 192.168.1.100:9100)\n  \
          --channel <CHANNEL>   Update channel (default: stable)\n  \
          --no-service          Do not install LaunchAgent or systemd unit"
    );
}

fn parse_cli_args(args: &[String]) -> Result<CliCommand, String> {
    if args.is_empty() {
        return Err("No command specified. Use --help for usage.".to_string());
    }

    match args[0].as_str() {
        "--version" | "-v" | "version" => Ok(CliCommand::Version),
        "--help" | "-h" | "help" => {
            print_help();
            std::process::exit(0);
        }
        "run" => Ok(CliCommand::Run),
        "uninstall" => Ok(CliCommand::Uninstall),
        "status" => Ok(CliCommand::Status),
        "install" => {
            let mut server = None;
            let mut code = None;
            let mut cups_queue = None;
            let mut tcp_addr = None;
            let mut channel = "stable".to_string();
            let mut no_service = false;

            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--server" => {
                        i += 1;
                        if i >= args.len() {
                            return Err("missing argument for --server".to_string());
                        }
                        server = Some(args[i].clone());
                    }
                    "--code" => {
                        i += 1;
                        if i >= args.len() {
                            return Err("missing argument for --code".to_string());
                        }
                        code = Some(args[i].clone());
                    }
                    "--cups-queue" => {
                        i += 1;
                        if i >= args.len() {
                            return Err("missing argument for --cups-queue".to_string());
                        }
                        cups_queue = Some(args[i].clone());
                    }
                    "--tcp" => {
                        i += 1;
                        if i >= args.len() {
                            return Err("missing argument for --tcp".to_string());
                        }
                        tcp_addr = Some(args[i].clone());
                    }
                    "--channel" => {
                        i += 1;
                        if i >= args.len() {
                            return Err("missing argument for --channel".to_string());
                        }
                        channel = args[i].clone();
                    }
                    "--no-service" => {
                        no_service = true;
                    }
                    other => return Err(format!("unknown option for install: {other}")),
                }
                i += 1;
            }

            let server = server.ok_or("missing required option --server")?;
            let code = code.ok_or("missing required option --code")?;

            let drawer = match (cups_queue, tcp_addr) {
                (Some(q), None) => DrawerInfo {
                    transport: "CUPS".to_string(),
                    target: q,
                },
                (None, Some(t)) => DrawerInfo {
                    transport: "TCP".to_string(),
                    target: t,
                },
                (Some(_), Some(_)) => {
                    return Err("specify either --cups-queue or --tcp, not both".to_string());
                }
                (None, None) => {
                    return Err("must specify either --cups-queue or --tcp".to_string());
                }
            };

            Ok(CliCommand::Install {
                server,
                code,
                drawer,
                channel,
                no_service,
            })
        }
        other => Err(format!("unknown command: {other}")),
    }
}

async fn run_install(
    server: String,
    code: String,
    drawer: DrawerInfo,
    channel: String,
    no_service: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let host = current_host();
    let os = current_os();

    println!("Connecting to server {server} to pair...");
    let api = HttpAgentApi::for_pairing(&server)?;

    let request = PairRequest {
        code,
        host,
        os,
        version: VERSION.to_string(),
        build: BUILD.to_string(),
    };

    let answer = api.pair(&request).await?;
    println!(
        "Successfully paired agent '{}' (ID: {}) for branch {}",
        answer.agent_name, answer.agent_id, answer.branch_id
    );

    let config_dir = default_config_dir();
    let config_path = config_dir.join("config.json");
    let token_path = config_dir.join("token");

    let config = ConfigFile {
        server,
        drawer,
        channel,
        agent_id: Some(answer.agent_id),
        branch_id: Some(answer.branch_id),
        agent_name: Some(answer.agent_name),
    };

    save_config(&config_path, &config)?;
    save_token(&token_path, &answer.token)?;
    let _ = chown_to_service_user(&config_path, "xc-agent");
    let _ = chown_to_service_user(&config_dir, "xc-agent");
    println!("Configuration and token saved to {}", config_dir.display());

    if no_service {
        println!("No service unit was installed (--no-service).");
    } else {
        let current_exe = std::env::current_exe()?;
        match install_service(&current_exe) {
            Ok(svc_path) => println!("Service installed at {}", svc_path.display()),
            Err(e) => eprintln!("Note: Could not install background service: {e}"),
        }
    }

    Ok(())
}

async fn run_daemon() -> Result<(), Box<dyn std::error::Error>> {
    let config_dir = default_config_dir();
    let config_path = config_dir.join("config.json");
    let token_path = config_dir.join("token");

    let config = load_config(&config_path)?;
    let token = load_token(&token_path)?;

    let journal_path = config_dir.join("journal.jsonl");
    let journal = Journal::open(journal_path)?;

    let api = HttpAgentApi::new(&config.server, &token)?;

    let agent_config = AgentConfig {
        version: VERSION.to_string(),
        build: BUILD.to_string(),
        os: current_os(),
        host: current_host(),
        drawer: config.drawer.clone(),
        wait_ms: 20_000,
    };

    if config.drawer.transport.eq_ignore_ascii_case("CUPS") {
        let transport = CupsTransport::new(&config.drawer.target);
        run_loop(api, transport, journal, agent_config).await;
    } else {
        let (host, port) = parse_tcp_target(&config.drawer.target);
        let transport = TcpTransport9100::new(host, port);
        run_loop(api, transport, journal, agent_config).await;
    }

    Ok(())
}

fn parse_tcp_target(target: &str) -> (String, u16) {
    if let Some((h, p)) = target.split_once(':') {
        let port = p.parse::<u16>().unwrap_or(9100);
        (h.to_string(), port)
    } else {
        (target.to_string(), 9100)
    }
}

async fn run_loop<A: AgentApi + 'static, T: DrawerTransport + 'static>(
    api: A,
    transport: T,
    journal: Journal,
    config: AgentConfig,
) {
    let mut runner = AgentRunner::new(api, transport, journal, config);
    runner.recover().await;

    loop {
        match runner.step().await {
            Step::Idle => {}
            Step::Done(outcome) => {
                eprintln!("job processed: outcome {outcome:?}");
            }
            Step::RePairNeeded => {
                eprintln!("re-pair needed");
                tokio::time::sleep(Duration::from_secs(600)).await;
            }
            Step::Wait(duration) => {
                tokio::time::sleep(duration).await;
            }
        }
    }
}

fn run_status() -> Result<(), Box<dyn std::error::Error>> {
    let config_dir = default_config_dir();
    let config_path = config_dir.join("config.json");
    let token_path = config_dir.join("token");

    println!("Config directory: {}", config_dir.display());
    if config_path.exists() {
        let cfg = load_config(&config_path)?;
        println!("Server: {}", cfg.server);
        println!("Drawer transport: {}", cfg.drawer.transport);
        println!("Drawer target: {}", cfg.drawer.target);
        println!("Channel: {}", cfg.channel);
        if let Some(id) = cfg.agent_id {
            println!("Agent ID: {id}");
        }
        if let Some(name) = cfg.agent_name {
            println!("Agent name: {name}");
        }
        if let Some(branch) = cfg.branch_id {
            println!("Branch ID: {branch}");
        }
    } else {
        println!("No config file found at {}", config_path.display());
    }

    if token_path.exists() {
        println!("Token: present (stored with restricted permissions)");
    } else {
        println!("Token: not found");
    }

    Ok(())
}

fn run_uninstall() -> Result<(), Box<dyn std::error::Error>> {
    let config_dir = default_config_dir();
    let token_path = config_dir.join("token");

    if token_path.exists() {
        let _ = std::fs::remove_file(&token_path);
        println!("Token file removed.");
    }

    let _ = uninstall_service();
    println!("Service uninstalled.");
    Ok(())
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = match parse_cli_args(&args) {
        Ok(cmd) => cmd,
        Err(err) => {
            eprintln!("Error: {err}");
            std::process::exit(1);
        }
    };

    match command {
        CliCommand::Version => {
            let config_dir = default_config_dir();
            let channel = load_config(&config_dir.join("config.json"))
                .ok()
                .map(|c| c.channel);
            print_version(channel.as_deref());
        }
        CliCommand::Install {
            server,
            code,
            drawer,
            channel,
            no_service,
        } => {
            if let Err(e) = run_install(server, code, drawer, channel, no_service).await {
                eprintln!("Install failed: {e}");
                std::process::exit(1);
            }
        }
        CliCommand::Run => {
            if let Err(e) = run_daemon().await {
                eprintln!("Agent failed: {e}");
                std::process::exit(1);
            }
        }
        CliCommand::Uninstall => {
            if let Err(e) = run_uninstall() {
                eprintln!("Uninstall failed: {e}");
                std::process::exit(1);
            }
        }
        CliCommand::Status => {
            if let Err(e) = run_status() {
                eprintln!("Status failed: {e}");
                std::process::exit(1);
            }
        }
    }
}
