//! The command line: argument parsing, the subcommands, and the GUI launch when there are none.

use std::collections::HashMap;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use clap::{Parser, Subcommand};
use tracing::{info, warn};

use crate::session::{self, LatencyProfile, SessionEvent, SessionSpec, Status};
use crate::settings::Settings;
use crate::{APP_TITLE, capture, controller, discovery, firewall, logging, single_instance, ui};

#[derive(Parser, Debug)]
#[command(
    name = "zephon",
    version,
    about = "Streams PC audio to AirPlay 2 receivers (HomePod). No arguments opens the GUI."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// List capture endpoints (Windows playback devices that loopback can record)
    Devices,
    /// Browse for AirPlay 2 receivers (mDNS `_airplay._tcp`, 3 s) and read their volume
    Receivers,
    /// Check (or install, with one UAC prompt) the inbound firewall rule for this exe
    Firewall {
        /// Show whether receivers can reach this exe (read-only)
        #[arg(long)]
        check: bool,
        /// Create the rule (UAC prompt), then check again
        #[arg(long)]
        install: bool,
    },
    /// Stream to one or more receivers (each its own session, fed from one shared capture)
    Stream {
        /// Receiver name, IP or IP:port. Repeat for several receivers.
        #[arg(long, required = true)]
        receiver: Vec<String>,
        /// Capture endpoint name (substring) or ID. Default: saved, else the Windows default.
        #[arg(long)]
        device: Option<String>,
        /// Start volume in percent (0-100). Default: the receiver's current volume, else 30.
        #[arg(long, value_parser = parse_volume)]
        volume: Option<f32>,
        /// Latency profile for all receivers. Default: each receiver's saved profile, else balanced.
        #[arg(long, value_enum)]
        profile: Option<LatencyProfile>,
        /// Stop after this many seconds (default: until Ctrl+C)
        #[arg(long)]
        duration: Option<u64>,
    },
    /// Headless check of the app controller: start it, wait, print the state the UI would show
    #[command(hide = true)]
    Status {
        #[arg(long, default_value_t = 6)]
        wait: u64,
    },
}

/// `--volume`: a number from 0 to 100. NaN in particular must not get through: it survives
/// clamping and reaches the receiver as full volume.
fn parse_volume(s: &str) -> Result<f32, String> {
    let v: f32 = s.parse().map_err(|e| format!("{e}"))?;
    if (0.0..=100.0).contains(&v) {
        Ok(v)
    } else {
        Err("must be a number from 0 to 100".into())
    }
}

/// GUI-subsystem exes get no console. When run with arguments from a terminal, attach to the
/// terminal's console so CLI output (and `--help`) is visible. Redirected handles (pipes,
/// files) are inherited as they are and keep working without this.
fn attach_parent_console() {
    #[cfg(windows)]
    {
        const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn AttachConsole(process_id: u32) -> i32;
        }
        // SAFETY: plain Win32 call; failure (no parent console) just leaves output unseen.
        unsafe {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

/// Runs the command given on the command line, or the GUI when there is none.
pub fn run() -> ExitCode {
    if std::env::args_os().len() > 1 {
        attach_parent_console();
    }
    let cli = Cli::parse();
    // Checked on the parsed command, not the argument count: `zephon --` also opens the GUI.
    if cli.command.is_none() && single_instance::focus_running_instance() {
        return ExitCode::SUCCESS;
    }
    let console_level = match &cli.command {
        // Nyx warns about every link-local (169.254.x) adapter it cannot query; not useful here.
        Some(Cmd::Stream { .. }) => "info,discovery=error",
        _ => "warn,discovery=error",
    };
    logging::init_logging(console_level);
    info!(
        "{APP_TITLE} {} starting: {:?}",
        env!("CARGO_PKG_VERSION"),
        cli.command
    );

    let result = match cli.command {
        None => {
            ui::run();
            Ok(ExitCode::SUCCESS)
        }
        Some(Cmd::Devices) => cmd_devices().map(|_| ExitCode::SUCCESS),
        Some(Cmd::Firewall { check, install }) => cmd_firewall(check, install),
        Some(Cmd::Status { wait }) => cmd_status(wait).map(|_| ExitCode::SUCCESS),
        Some(cmd) => tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(anyhow::Error::from)
            .and_then(|rt| {
                rt.block_on(async move {
                    match cmd {
                        Cmd::Receivers => cmd_receivers().await.map(|_| ExitCode::SUCCESS),
                        Cmd::Stream {
                            receiver,
                            device,
                            volume,
                            profile,
                            duration,
                        } => cmd_stream(receiver, device, volume, profile, duration).await,
                        _ => unreachable!(),
                    }
                })
            }),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            tracing::error!("{e:#}");
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_devices() -> anyhow::Result<()> {
    let endpoints = capture::list_endpoints()?;
    let chosen = capture::choose_endpoint(&endpoints, &Settings::load());
    println!("Capture endpoints (render devices for loopback):");
    for d in &endpoints {
        let is_chosen = chosen.as_ref().is_ok_and(|c| c.id == d.id);
        println!(
            "  {} {:<40} {}{}",
            if is_chosen { "*" } else { " " },
            d.friendly_name,
            d.id,
            if d.is_default {
                "  [Windows default]"
            } else {
                ""
            }
        );
    }
    match chosen {
        Ok(c) => println!("* = used by default: {}", c.friendly_name),
        Err(e) => println!("No default: {e}"),
    }
    Ok(())
}

async fn cmd_receivers() -> anyhow::Result<()> {
    let settings = Settings::load();
    let list = discovery::discover(settings.interface.clone()).await?;
    if list.is_empty() {
        println!(
            "No AirPlay receivers found in {} s.",
            discovery::BROWSE_TIME.as_secs()
        );
        return Ok(());
    }
    let volumes = futures_join(list.iter().map(|r| discovery::read_volume_pct(r.addr))).await;
    println!("AirPlay receivers (_airplay._tcp):");
    for (r, vol) in list.iter().zip(volumes) {
        let vol = match vol {
            Ok(Ok(p)) => format!("{p:.0}%"),
            _ => "?".into(),
        };
        let via = match r.addr.ip() {
            std::net::IpAddr::V4(ip) => discovery::lan_interface_for(ip)
                .map(|(name, local)| format!("{name} {local}"))
                .unwrap_or_else(|| "OS routing".into()),
            _ => "-".into(),
        };
        println!(
            "  {:<20} {:<20} {:<18} volume {vol:<5} via {via}",
            r.name,
            r.addr.to_string(),
            r.model.as_deref().unwrap_or("-")
        );
    }
    Ok(())
}

/// Awaits all futures concurrently (spawned on the runtime), keeping order. A task that
/// panics gives an error in its place (logged) rather than ending the whole command.
async fn futures_join<F, T>(futs: impl Iterator<Item = F>) -> Vec<Result<T, tokio::task::JoinError>>
where
    F: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let handles: Vec<_> = futs.map(tokio::spawn).collect();
    let mut out = Vec::with_capacity(handles.len());
    for h in handles {
        let r = h.await;
        if let Err(e) = &r {
            warn!("Task failed: {e}");
        }
        out.push(r);
    }
    out
}

fn cmd_firewall(check: bool, install: bool) -> anyhow::Result<ExitCode> {
    if !check && !install {
        println!("Use --check or --install.");
        return Ok(ExitCode::SUCCESS);
    }
    let exe = firewall::exe_path();
    println!("exe: {}", exe.display());
    let status = firewall::check().map_err(|e| anyhow!("firewall check failed: {e}"))?;
    println!("{}", firewall::describe(&status));
    if install {
        println!("Requesting administrator rights (UAC) to allow inbound traffic to this exe...");
        match firewall::install() {
            firewall::FirewallState::Ok => println!("Done -> OK"),
            firewall::FirewallState::Cancelled => {
                println!("Cancelled at the UAC prompt; nothing changed.")
            }
            other => bail!("rule not in place after install: {other:?}"),
        }
    } else if status.needs_fix() {
        println!(
            "Without the rule, receivers' timing requests are dropped and streams fail with 400 after about 30 s.\n\
             Fix: zephon firewall --install"
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_status(wait: u64) -> anyhow::Result<()> {
    let ctl = controller::Controller::spawn(Arc::new(|| {}));
    std::thread::sleep(Duration::from_secs(wait));
    let st = ctl.snapshot();
    println!("exe:       {}", st.exe_path);
    println!("firewall:  {:?}", st.firewall);
    println!(
        "profile:   {} (default for new receivers)",
        st.default_profile.name()
    );
    match (&st.endpoint, &st.endpoint_error) {
        (Some(e), _) => println!(
            "capture:   {} ({} endpoints listed)",
            e.name,
            st.endpoints.len()
        ),
        (None, Some(err)) => println!("capture:   none: {err}"),
        (None, None) => println!("capture:   none"),
    }
    println!(
        "receivers: {}",
        if st.discovering {
            "(still discovering)"
        } else {
            ""
        }
    );
    for r in &st.receivers {
        println!(
            "  [{}] {:<24} {:<22} {:<9} volume {:.0}%{} {:?}{}",
            if r.selected { "x" } else { " " },
            r.name,
            r.addr.to_string(),
            r.profile.name(),
            r.volume_pct,
            if r.volume_known { "" } else { " (default)" },
            r.status,
            if r.online { "" } else { " offline" }
        );
    }
    if let Some(m) = &st.message {
        println!("message:   {m}");
    }
    ctl.shutdown(Duration::from_secs(2));
    Ok(())
}

async fn cmd_stream(
    receivers: Vec<String>,
    device: Option<String>,
    volume: Option<f32>,
    profile: Option<LatencyProfile>,
    duration: Option<u64>,
) -> anyhow::Result<ExitCode> {
    let settings = Settings::load();

    let endpoints = capture::list_endpoints()?;
    let endpoint = match &device {
        Some(d) => endpoints
            .iter()
            .find(|e| &e.id == d)
            .or_else(|| capture::find_by_name(&endpoints, d))
            .cloned()
            .ok_or_else(|| {
                let names: Vec<_> = endpoints.iter().map(|e| e.friendly_name.as_str()).collect();
                anyhow!("no capture endpoint matches \"{d}\". Available: {names:?}")
            })?,
        None => capture::choose_endpoint(&endpoints, &settings).map_err(|e| anyhow!(e))?,
    };

    match firewall::check() {
        Ok(st) if st.needs_fix() => warn!(
            "No inbound firewall rule for this exe: the stream will likely fail (400 after about 30 s, \
             reported here after {} s). Run `zephon firewall --install` first.",
            session::START_TIMEOUT.as_secs()
        ),
        Ok(_) => info!("Firewall rule: OK"),
        Err(e) => warn!("Firewall check failed: {e}"),
    }

    let mut specs = Vec::new();
    for name in &receivers {
        let r = discovery::resolve(name, settings.interface.clone())
            .await
            .with_context(|| format!("receiver \"{name}\""))?;
        let volume_pct = match volume {
            Some(v) => v,
            None => discovery::read_volume_pct(r.addr)
                .await
                .unwrap_or_else(|e| {
                    info!(
                        "Volume of \"{}\" unknown ({e}); using {}%",
                        r.name,
                        controller::DEFAULT_VOLUME_PCT
                    );
                    controller::DEFAULT_VOLUME_PCT
                }),
        };
        specs.push(SessionSpec {
            profile: profile.unwrap_or_else(|| settings.profile_for(&r.name)),
            name: r.name,
            addr: r.addr,
            volume_pct,
            interface: settings.interface.clone(),
        });
    }

    let hub = capture::CaptureHub::open(&endpoint.id, &endpoint.friendly_name)?;
    let (ev_tx, mut ev_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut names = HashMap::new();
    let mut profiles = HashMap::new();
    let mut handles = Vec::new();
    for (i, spec) in specs.into_iter().enumerate() {
        let id = i as u64 + 1;
        names.insert(id, spec.name.clone());
        profiles.insert(id, spec.profile);
        let (src, queue) = hub.subscribe();
        handles.push(session::spawn(id, spec, src, queue, ev_tx.clone()));
    }
    drop(ev_tx);

    let deadline = async {
        match duration {
            Some(s) => tokio::time::sleep(Duration::from_secs(s)).await,
            None => std::future::pending().await,
        }
    };
    tokio::pin!(deadline);
    let mut stopping = false;
    let mut failed = false;
    let mut running = names.len();
    while running > 0 {
        tokio::select! {
            _ = &mut deadline, if !stopping => {
                info!("Duration elapsed; stopping");
                stopping = true;
                handles.iter().for_each(|h| h.stop());
            }
            _ = tokio::signal::ctrl_c(), if !stopping => {
                info!("Ctrl+C; stopping");
                stopping = true;
                handles.iter().for_each(|h| h.stop());
            }
            ev = ev_rx.recv() => {
                let Some((id, ev)) = ev else { break };
                let name = &names[&id];
                match ev {
                    SessionEvent::Status(Status::Error(e)) => {
                        failed = true;
                        eprintln!("[{name}] error: {e}");
                    }
                    SessionEvent::Status(s) => eprintln!("[{name}] {s:?}"),
                    SessionEvent::ReceiverVolume(p) => eprintln!("[{name}] receiver volume {p:.0}%"),
                    SessionEvent::Ended(st) => {
                        running -= 1;
                        println!("[{name}] {}", session::stats_line(profiles[&id], &st));
                    }
                    SessionEvent::Stats(_) | SessionEvent::FirewallSuspected => {}
                }
            }
        }
    }
    drop(hub);
    Ok(if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_argument_must_be_a_finite_percentage() {
        assert_eq!(parse_volume("0"), Ok(0.0));
        assert_eq!(parse_volume("42.5"), Ok(42.5));
        assert_eq!(parse_volume("100"), Ok(100.0));
        for bad in ["NaN", "nan", "inf", "-inf", "-1", "100.1", "loud"] {
            assert!(parse_volume(bad).is_err(), "{bad}");
        }
    }

    fn parse(args: &[&str]) -> Result<Option<Cmd>, clap::Error> {
        Cli::try_parse_from(std::iter::once("zephon").chain(args.iter().copied()))
            .map(|cli| cli.command)
    }

    #[test]
    fn command_line_definition_is_consistent() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }

    #[test]
    fn no_command_opens_the_gui() {
        assert!(parse(&[]).unwrap().is_none());
        assert!(parse(&["--"]).unwrap().is_none());
    }

    #[test]
    fn stream_arguments_are_parsed_and_checked() {
        let Some(Cmd::Stream {
            receiver,
            volume,
            profile,
            ..
        }) = parse(&[
            "stream",
            "--receiver",
            "Kitchen",
            "--receiver",
            "192.0.2.20",
            "--volume",
            "30",
            "--profile",
            "stable",
        ])
        .unwrap()
        else {
            panic!("not a stream command");
        };
        assert_eq!(receiver, ["Kitchen", "192.0.2.20"]);
        assert_eq!(volume, Some(30.0));
        assert_eq!(profile, Some(LatencyProfile::Stable));
        assert!(parse(&["stream"]).is_err());
        for bad in ["NaN", "101"] {
            assert!(
                parse(&["stream", "--receiver", "Kitchen", "--volume", bad]).is_err(),
                "{bad}"
            );
        }
    }
}
