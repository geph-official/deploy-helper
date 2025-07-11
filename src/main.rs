use std::{
    env,
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

use crate::config::{Config, parse_config};
use anyhow::Context;
use atomicwrites::{AtomicFile, OverwriteBehavior::AllowOverwrite};
use clap::{Parser, Subcommand, builder::PathBufValueParser};
use fs2::FileExt;
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
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("deploy_helper=debug"),
    )
    .init();

    if let Err(e) = match &ARGS.command {
        Commands::Update { config } => update(config),
        Commands::Run { config } => run(config),
    } {
        log::error!("ERROR: {e}");
    };
}

fn run(config_path: &PathBuf) -> anyhow::Result<()> {
    let config = parse_config(config_path);
    let config_dir = config_path
        .parent()
        .context("config has no parent directory")?;
    env::set_current_dir(config_dir)?;

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
    let lock_path = format!("/var/lock/update-{}.lock", config.program_name);
    let lock_file = File::create(Path::new(&lock_path))?;
    if let Err(e) = lock_file.try_lock_exclusive() {
        log::error!("Another instance is already running: {}", e);
        anyhow::bail!("Could not acquire file lock")
    }
    log::debug!("1 - acquired lock");

    // 0. Set current dir as dir of the config file
    let config_dir = config_path
        .parent()
        .context("config has no parent directory")?;
    env::set_current_dir(config_dir)?;

    log::debug!("2 - set cwd to {}", config_dir.display());

    // 1. Immediate update – run all commands
    for cmd in &config.update.commands {
        let status = Command::new("bash").arg("-c").arg(cmd).status()?;
        if !status.success() {
            anyhow::bail!("`{}` exited with {}", cmd, status);
        }
    }

    log::debug!("3 - all update commands executed");

    // 2. Names & paths
    let deploy_helper_exe = env::current_exe()?;
    let program_name = config.program_name;
    let update_svc = format!("update-{}.service", program_name);
    let update_timer = format!("update-{}.timer", program_name);
    let run_svc = format!("run-{}.service", program_name);
    let sysd_dir = PathBuf::from("/etc/systemd/system");

    // 3a. Update service
    let update_unit = format!(
        r#"[Unit]
Description=deploy-helper update for {program_name}
Wants={run_svc}
After=network-online.target

[Service]
Type=oneshot 
ExecStart={deploy_helper_exe} update {config_path}
"#,
        run_svc = run_svc,
        deploy_helper_exe = deploy_helper_exe.display(),
        config_path = config_path.display(),
    );

    // 3b. Timer unit
    let timer_unit = format!(
        r#"[Unit]
Description=deploy-helper update timer for {program_name}

[Timer]
OnBootSec=1min
OnUnitActiveSec={interval}
Unit={update_svc}

[Install]
WantedBy=timers.target
"#,
        interval = config.update.interval,
        update_svc = update_svc,
    );

    // 3c. Run service
    let run_unit = format!(
        r#"[Unit]
Description=deploy-helper run for {program_name}

[Service]
Type=simple 
ExecStart={deploy_helper_exe} run {config_path}
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
"#,
        deploy_helper_exe = deploy_helper_exe.display(),
        config_path = config_path.display(),
    );

    // 4. Write unit files
    AtomicFile::new(&sysd_dir.join(&update_svc), AllowOverwrite)
        .write(|f| f.write_all(update_unit.as_bytes()))?;
    AtomicFile::new(&sysd_dir.join(&update_timer), AllowOverwrite)
        .write(|f| f.write_all(timer_unit.as_bytes()))?;
    AtomicFile::new(&sysd_dir.join(&run_svc), AllowOverwrite)
        .write(|f| f.write_all(run_unit.as_bytes()))?;

    log::debug!("4 - all unit files written");

    // 5. Reload and enable units
    Command::new("systemctl").arg("daemon-reload").status()?;
    Command::new("systemctl")
        .args(["enable", "--now", &update_timer])
        .status()?;
    restart_if_changed(&[config.binary_path], &run_svc)?;

    log::debug!("✅ update complete - daemon reloaded, timer & runner active");
    Ok(())
}

fn restart_if_changed(paths: &[PathBuf], run_svc: &str) -> anyhow::Result<()> {
    use sha2::{Digest, Sha256};
    use std::fs;

    fn digest(path: &PathBuf) -> anyhow::Result<Vec<u8>> {
        let bytes = fs::read(path)?;
        Ok(Sha256::digest(&bytes).to_vec())
    }

    let before: Vec<_> = paths.iter().map(digest).collect::<Result<_, _>>()?;
    // …run update commands here…
    let after: Vec<_> = paths.iter().map(digest).collect::<Result<_, _>>()?;

    if before != after {
        Command::new("systemctl")
            .args(["restart", run_svc])
            .status()?;
    } else {
        log::info!("No binaries changed – skipping restart");
    }
    Ok(())
}
