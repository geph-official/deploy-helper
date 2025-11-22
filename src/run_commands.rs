use anyhow::Context;
use std::process::Command;

pub fn run_commands(commands: &[String]) -> anyhow::Result<()> {
    for cmd in commands {
        log::debug!("Running: {}", cmd);
        let status = Command::new("bash")
            .arg("-ic")
            .arg(cmd)
            .status()
            .with_context(|| format!("Failed to spawn bash for `{}`", cmd))?;

        if !status.success() {
            anyhow::bail!("Command `{}` exited with status {}", cmd, status);
        }
    }

    Ok(())
}
