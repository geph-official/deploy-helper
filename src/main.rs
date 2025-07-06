use std::{env, fs, io, path::PathBuf, process::Command};

use crate::config::{Config, parse_config};
use anyhow::Context;
use clap::{Parser, Subcommand};
use once_cell::sync::Lazy;

mod config;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// execute repo update commands in config
    Update { config: PathBuf },
    /// execute run commands
    Run { config: PathBuf },
}

static ARGS: Lazy<Args> = Lazy::new(Args::parse);

fn main() {
    env_logger::Builder::new()
        .filter_module("deploy-helper", log::LevelFilter::Debug)
        .init();

    match &ARGS.command {
        Commands::Update { config } => update(config),
        Commands::Run { config } => run(parse_config(config)),
    }
    .unwrap();
}

fn run(config: Config) -> anyhow::Result<()> {
    for cmd in &config.run.commands {
        log::debug!("Running: {}", cmd);
        let status = Command::new("bash")
            .arg("-c")
            .arg(cmd)
            .status()
            .unwrap_or_else(|e| panic!("Failed to spawn bash for `{}`: {}", cmd, e));

        if !status.success() {
            anyhow::bail!("Command `{}` exited with status {}", cmd, status);
        }
    }
    Ok(())
}

/// Perform the update commands, (re)generate systemd units, and activate them.
fn update(config_path: &PathBuf) -> anyhow::Result<()> {
    let config = parse_config(config_path);
    // 1. Immediate update – run all commands
    for cmd in &config.update.commands {
        let status = Command::new("bash").arg("-c").arg(cmd).status()?;
        if !status.success() {
            anyhow::bail!("`{}` exited with {}", cmd, status);
        }
    }

    // 2. Names & paths
    let deploy_helper_exe = env::current_exe()?;
    let cwd = env::current_dir()?;
    let repo = cwd
        .file_name()
        .and_then(|s| s.to_str())
        .context("cannot determine repo name from cwd")?;
    let lock_file = format!("/var/lock/update-{}.lock", repo);
    let update_svc = format!("update-{}.service", repo);
    let update_timer = format!("update-{}.timer", repo);
    let run_svc = format!("run-{}.service", repo);
    let sysd_dir = PathBuf::from("/etc/systemd/system");

    // 3a. Update service
    let update_unit = format!(
        r#"[Unit]
Description=deploy-helper update for {repo}
Wants={run_svc}
After=network-online.target

[Service]
Type=oneshot
WorkingDirectory={wd}
ExecStartPre=/bin/bash -c "exec 200>{lock}; flock -n 200"
ExecStart={deploy_helper_exe} update {config_path}
ExecStartPost=/bin/bash -c "systemctl daemon-reload && systemctl restart {run_svc}"
"#,
        repo = repo,
        wd = cwd.display(),
        lock = lock_file,
        run_svc = run_svc,
        deploy_helper_exe = deploy_helper_exe.display(),
        config_path = config_path.display(),
    );

    // 3b. Timer unit
    let timer_unit = format!(
        r#"[Unit]
Description=deploy-helper update timer for {repo}

[Timer]
OnBootSec=1min
OnUnitActiveSec={interval}
Unit={update_svc}

[Install]
WantedBy=timers.target
"#,
        repo = repo,
        interval = config.update.interval,
        update_svc = update_svc,
    );

    // 3c. Run service
    let run_unit = format!(
        r#"[Unit]
Description=deploy-helper run for {repo}

[Service]
Type=simple
WorkingDirectory={wd}
ExecStart={deploy_helper_exe} run {config_path}
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
"#,
        repo = repo,
        wd = cwd.display(),
        deploy_helper_exe = deploy_helper_exe.display(),
        config_path = config_path.display(),
    );

    // 4. Write unit files
    write_if_changed(&sysd_dir.join(&update_svc), &update_unit)?;
    write_if_changed(&sysd_dir.join(&update_timer), &timer_unit)?;
    write_if_changed(&sysd_dir.join(&run_svc), &run_unit)?;

    // 5. Reload and enable units
    Command::new("systemctl").arg("daemon-reload").status()?;
    Command::new("systemctl")
        .args(["enable", "--now", &update_timer])
        .status()?;
    Command::new("systemctl")
        .args(["enable", "--now", &run_svc])
        .status()?;

    log::debug!("✅ update complete – units written, daemon reloaded, timer & runner active");
    Ok(())
}

/// Overwrite the target only if contents differ.
fn write_if_changed(path: &PathBuf, data: &str) -> io::Result<()> {
    match fs::read_to_string(path) {
        Ok(existing) if existing == data => Ok(()),
        _ => fs::write(path, data),
    }
}
