//! `monoplan daemon install` / `uninstall` — run the daemon at login.
//!
//! A launchd agent on macOS (`~/Library/LaunchAgents/<label>.plist`,
//! loaded into the `gui/<uid>` domain) or a systemd user unit on Linux
//! (`~/.config/systemd/user/monoplan.service`). Both run a plain
//! `monoplan daemon` from *this* executable's resolved path, pinned to
//! the profile active at install time via `MONOPLAN_DATA_DIR`, with
//! stderr appended to the profile's `daemon.log`.
//!
//! Restart policy is "on failure only": `daemon stop` is a clean exit
//! and stays stopped until the next login (or another `install`, which
//! is idempotent and doubles as "start it now"). A client that finds
//! the daemon stale shuts it down the same way and spawns its own; the
//! agent takes over again at the next login, so pointing the agent at
//! the binary the CLI also runs (the app bundle's copy, symlinked into
//! `PATH`) is what keeps the two from trading places.
//!
//! The macOS app runs `install` on first launch (spec/daemon-plan.md),
//! and `logout` runs `uninstall`: a purged profile would otherwise
//! leave an agent that fails at every login.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::Profile;

/// launchd label and the plist's file stem.
pub const LAUNCHD_LABEL: &str = "app.monoplan.daemon";
/// systemd user unit name.
pub const SYSTEMD_UNIT: &str = "monoplan.service";

/// Where the agent / unit file lives for this user.
pub fn unit_path() -> anyhow::Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("no home directory"))?;
    if cfg!(target_os = "macos") {
        Ok(home
            .join("Library/LaunchAgents")
            .join(format!("{LAUNCHD_LABEL}.plist")))
    } else if cfg!(target_os = "linux") {
        Ok(home.join(".config/systemd/user").join(SYSTEMD_UNIT))
    } else {
        anyhow::bail!("daemon install is only supported on macOS and Linux")
    }
}

/// Is the agent / unit installed *for this profile*? The file pins one
/// profile dir, so a test profile never sees the user's real agent.
pub fn is_installed_for(profile_dir: &Path) -> bool {
    let Ok(path) = unit_path() else { return false };
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    text.contains(&profile_dir.to_string_lossy().to_string())
}

pub async fn install() -> anyhow::Result<()> {
    let profile = Profile::require_active()?;
    // Resolve symlinks (a `/usr/local/bin/monoplan` into the app
    // bundle) so the agent survives the link being moved.
    let exe = std::env::current_exe()?.canonicalize()?;
    let path = unit_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // The agent is the headless alternative to the app, which takes a
    // headless daemon over whenever it runs: installing beside it would
    // only start a daemon for the app to stop.
    if crate::client::hosted_by_app(&profile.dir).await? {
        anyhow::bail!(
            "Monoplan.app is hosting this profile; the login agent is for Macs without the app"
        );
    }
    // A daemon started by hand would hold the lock and make the
    // agent's instance fail; hand the profile over cleanly first.
    crate::client::stop_if_running(&profile.dir).await?;

    if cfg!(target_os = "macos") {
        // Reloading an already-bootstrapped label is an error, so
        // unload first; failure there just means it was not loaded.
        let _ = launchctl(&["bootout", &gui_domain_target()]);
        std::fs::write(&path, launchd_plist(&exe, &profile.dir))?;
        launchctl(&[
            "bootstrap",
            &gui_domain(),
            path.to_str().unwrap_or_default(),
        ])?;
    } else {
        std::fs::write(&path, systemd_unit(&exe, &profile.dir))?;
        systemctl(&["daemon-reload"])?;
        systemctl(&["enable", "--now", SYSTEMD_UNIT])?;
    }
    println!("installed  {}", path.display());
    println!(
        "runs `{} daemon` at login for {}",
        exe.display(),
        profile.dir.display()
    );
    Ok(())
}

pub async fn uninstall() -> anyhow::Result<()> {
    let path = unit_path()?;
    if !path.exists() {
        println!("not installed");
        return Ok(());
    }
    remove(&path)?;
    println!("uninstalled  {}", path.display());
    Ok(())
}

/// Unload and delete the agent if there is one. Used by `logout`,
/// which must not fail on a missing or half-installed agent.
pub fn uninstall_quietly() {
    if let Ok(path) = unit_path()
        && path.exists()
    {
        let _ = remove(&path);
    }
}

fn remove(path: &Path) -> anyhow::Result<()> {
    if cfg!(target_os = "macos") {
        // bootout stops a running instance (SIGTERM, a clean exit).
        let _ = launchctl(&["bootout", &gui_domain_target()]);
    } else {
        let _ = systemctl(&["disable", "--now", SYSTEMD_UNIT]);
    }
    std::fs::remove_file(path)?;
    if cfg!(target_os = "linux") {
        let _ = systemctl(&["daemon-reload"]);
    }
    Ok(())
}

fn gui_domain() -> String {
    // SAFETY: getuid has no preconditions.
    format!("gui/{}", unsafe { libc::getuid() })
}

fn gui_domain_target() -> String {
    format!("{}/{LAUNCHD_LABEL}", gui_domain())
}

fn launchctl(args: &[&str]) -> anyhow::Result<()> {
    run_tool("launchctl", args)
}

fn systemctl(args: &[&str]) -> anyhow::Result<()> {
    let mut full = vec!["--user"];
    full.extend_from_slice(args);
    run_tool("systemctl", &full)
}

fn run_tool(tool: &str, args: &[&str]) -> anyhow::Result<()> {
    let out = Command::new(tool).args(args).output()?;
    if out.status.success() {
        return Ok(());
    }
    anyhow::bail!(
        "{tool} {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr).trim()
    )
}

/// The launchd agent. `KeepAlive.SuccessfulExit=false` restarts only
/// crashes, so `daemon stop` sticks; `ProcessType Background` keeps it
/// out of the way of foreground apps.
pub fn launchd_plist(exe: &Path, profile_dir: &Path) -> String {
    let exe = xml_escape(&exe.to_string_lossy());
    let dir = xml_escape(&profile_dir.to_string_lossy());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LAUNCHD_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
    <string>daemon</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>MONOPLAN_DATA_DIR</key>
    <string>{dir}</string>
  </dict>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <dict>
    <key>SuccessfulExit</key>
    <false/>
  </dict>
  <key>ProcessType</key>
  <string>Background</string>
  <key>StandardOutPath</key>
  <string>{dir}/daemon.log</string>
  <key>StandardErrorPath</key>
  <string>{dir}/daemon.log</string>
</dict>
</plist>
"#
    )
}

/// The systemd user unit, the same policy in systemd's terms.
pub fn systemd_unit(exe: &Path, profile_dir: &Path) -> String {
    let exe = exe.to_string_lossy();
    let dir = profile_dir.to_string_lossy();
    format!(
        "[Unit]\n\
         Description=Monoplan sync daemon\n\
         After=network-online.target\n\
         \n\
         [Service]\n\
         ExecStart=\"{exe}\" daemon\n\
         Environment=MONOPLAN_DATA_DIR={dir}\n\
         StandardError=append:{dir}/daemon.log\n\
         Restart=on-failure\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n"
    )
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launchd_plist_runs_this_exe_against_the_installed_profile() {
        let plist = launchd_plist(
            Path::new("/Applications/Monoplan.app/Contents/MacOS/monoplan"),
            Path::new("/Users/d/Library/Application Support/monoplan"),
        );
        assert!(plist.contains("<string>app.monoplan.daemon</string>"));
        assert!(plist.contains(
            "<string>/Applications/Monoplan.app/Contents/MacOS/monoplan</string>\n    <string>daemon</string>"
        ));
        assert!(plist.contains("<string>/Users/d/Library/Application Support/monoplan</string>"));
        assert!(plist.contains("<key>SuccessfulExit</key>\n    <false/>"));
        assert!(
            plist.contains(
                "<string>/Users/d/Library/Application Support/monoplan/daemon.log</string>"
            )
        );
    }

    #[test]
    fn launchd_plist_escapes_xml_in_paths() {
        let plist = launchd_plist(Path::new("/a&b/<m>"), Path::new("/p"));
        assert!(plist.contains("<string>/a&amp;b/&lt;m&gt;</string>"));
    }

    #[test]
    fn systemd_unit_is_on_failure_only() {
        let unit = systemd_unit(
            Path::new("/usr/local/bin/monoplan"),
            Path::new("/home/d/.local/share/monoplan"),
        );
        assert!(unit.contains("ExecStart=\"/usr/local/bin/monoplan\" daemon\n"));
        assert!(unit.contains("Environment=MONOPLAN_DATA_DIR=/home/d/.local/share/monoplan\n"));
        assert!(unit.contains("Restart=on-failure\n"));
    }
}
