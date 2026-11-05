//! Per-user login startup. launchd reads this file at login; syncing preferences
//! never bootouts the job that may currently own the GUI and its TUN session.
use anyhow::{bail, Context, Result};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const LABEL: &str = "com.mateoki.vproxy.login";

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn launch_agent_plist(executable: &Path) -> Result<String> {
    let executable = executable
        .to_str()
        .context("Login executable path is not UTF-8")?;
    if !Path::new(executable).is_absolute() || executable.contains('\0') {
        bail!("Login executable must have an absolute path");
    }
    // No node identifier, credentials, or serialized proxy configuration is
    // embedded in the plist. The login entry reads the current private DB.
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\"><dict>\n\
<key>Label</key><string>{LABEL}</string>\n\
<key>ProgramArguments</key><array><string>{}</string><string>--login-startup</string></array>\n\
<key>RunAtLoad</key><true/>\n\
<key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>\n\
<key>ThrottleInterval</key><integer>30</integer>\n\
<key>ProcessType</key><string>Interactive</string>\n\
<key>LimitLoadToSessionType</key><string>Aqua</string>\n\
<key>StandardOutPath</key><string>/dev/null</string>\n\
<key>StandardErrorPath</key><string>/dev/null</string>\n\
</dict></plist>\n",
        xml_escape(executable)
    ))
}

fn sync_at(home: &Path, executable: &Path, enabled: bool) -> Result<()> {
    if !home.is_absolute() {
        bail!("Login startup requires an absolute home directory");
    }
    let agents = home.join("Library/LaunchAgents");
    let target = agents.join(format!("{LABEL}.plist"));
    if !enabled {
        match fs::remove_file(&target) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("Remove VProxy login startup"),
        }
        // Do not bootout: if the current GUI was launched at login, that would
        // immediately stop the GUI/TUN when the user merely disables next login.
        // Any crash restart of the loaded job rechecks the saved setting first.
        return Ok(());
    }
    let desired = launch_agent_plist(executable)?;
    if fs::read(&target).ok().as_deref() == Some(desired.as_bytes()) {
        return Ok(());
    }
    fs::create_dir_all(&agents).context("Create per-user LaunchAgents directory")?;
    let staging = agents.join(format!(".{LABEL}.{}.tmp", Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&staging)?;
        file.write_all(desired.as_bytes())?;
        file.sync_all()?;
        fs::rename(&staging, &target).context("Save VProxy login startup")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&staging);
    }
    result
}

pub(super) fn sync(enabled: bool) -> Result<()> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is not available for login startup")?;
    let executable = std::env::current_exe()?.canonicalize()?;
    sync_at(&home, &executable, enabled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn login_agent_has_safe_arguments_and_restarts_only_after_failure() {
        let plist =
            launch_agent_plist(Path::new("/Applications/V & P.app/Contents/MacOS/vproxy")).unwrap();
        let fixture = std::env::temp_dir().join(format!("vproxy-login-plist-{}", Uuid::new_v4()));
        fs::write(&fixture, &plist).unwrap();
        let checked = std::process::Command::new("/usr/bin/plutil")
            .args(["-convert", "json", "-o", "-"])
            .arg(&fixture)
            .output()
            .unwrap();
        let _ = fs::remove_file(fixture);
        assert!(checked.status.success());
        let json: serde_json::Value = serde_json::from_slice(&checked.stdout).unwrap();
        assert_eq!(
            json["ProgramArguments"],
            serde_json::json!([
                "/Applications/V & P.app/Contents/MacOS/vproxy",
                "--login-startup"
            ])
        );
        assert_eq!(json["RunAtLoad"], true);
        assert_eq!(json["KeepAlive"]["SuccessfulExit"], false);
        assert_eq!(json["LimitLoadToSessionType"], "Aqua");
        assert!(json.get("EnvironmentVariables").is_none());
    }

    #[test]
    fn login_setting_reconciles_path_and_removes_only_its_own_file() {
        let home = std::env::temp_dir().join(format!("vproxy-login-test-{}", Uuid::new_v4()));
        let executable = Path::new("/Applications/VProxy.app/Contents/MacOS/vproxy");
        sync_at(&home, executable, false).unwrap();
        assert!(!home.exists());
        sync_at(&home, executable, true).unwrap();
        let agents = home.join("Library/LaunchAgents");
        let plist = agents.join(format!("{LABEL}.plist"));
        let before = fs::metadata(&plist).unwrap();
        assert_eq!(before.permissions().mode() & 0o777, 0o600);
        sync_at(&home, executable, true).unwrap();
        assert_eq!(
            before.modified().unwrap(),
            fs::metadata(&plist).unwrap().modified().unwrap()
        );
        let moved = Path::new("/Applications/Moved VProxy.app/Contents/MacOS/vproxy");
        sync_at(&home, moved, true).unwrap();
        assert_eq!(
            fs::read_to_string(&plist).unwrap(),
            launch_agent_plist(moved).unwrap()
        );
        let unrelated = agents.join("unrelated.plist");
        fs::write(&unrelated, "keep").unwrap();
        sync_at(&home, moved, false).unwrap();
        assert!(!plist.exists());
        assert_eq!(fs::read_to_string(&unrelated).unwrap(), "keep");
        assert_eq!(fs::read_dir(&agents).unwrap().count(), 1);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn login_agent_rejects_relative_executable_or_home() {
        assert!(launch_agent_plist(Path::new("vproxy")).is_err());
        assert!(sync_at(Path::new("relative"), Path::new("/vproxy"), true).is_err());
    }
}
