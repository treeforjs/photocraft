//! Preferences › Integrations at launch: whether and where the control channel (and so the MCP
//! bridge) serves. `--control` / `PHOTOCRAFT_CONTROL_PORT` pick the port for one launch; without
//! them a non-zero Control Port starts it every launch. Allow Agent Control off keeps it closed
//! either way.

use std::path::{Path, PathBuf};

use photocraft_engine::prefs::Integrations;
use serde_json::Value;

/// The Integrations preferences, read leniently before the window opens (defaults when the file
/// is missing or unreadable).
pub fn read(path: Option<&Path>) -> Integrations {
    let v: Value = path.and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    v.get("integrations").and_then(|i| serde_json::from_value::<Integrations>(i.clone()).ok()).unwrap_or_default()
}

/// Where to serve: (port, whether it came from the preference, a warning to log).
pub fn port(cli: Option<u16>, prefs: &Integrations) -> (Option<u16>, bool, Option<String>) {
    if !prefs.allow_agent_control {
        let warning = cli.map(|_| "agent control is off in Preferences › Integrations; the control server was not started".to_string());
        return (None, false, warning);
    }
    if cli.is_some() {
        return (cli, false, None);
    }
    match u16::try_from(prefs.control_port) {
        Ok(0) | Err(_) => (None, false, None),
        Ok(p) => (Some(p), true, None),
    }
}

/// The token file a preference-started server uses when none was given: `control.token` in the
/// settings folder, so a bridge can read it (`--control-token-file`). Created `0600` on first use.
pub fn default_token_file(config_dir: Option<PathBuf>) -> Option<PathBuf> {
    let dir = config_dir?;
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("control.token"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefs(allow: bool, port: u32) -> Integrations {
        Integrations { allow_agent_control: allow, control_port: port }
    }

    #[test]
    fn the_launch_flag_wins_then_the_preference() {
        assert_eq!(port(Some(7878), &prefs(true, 0)), (Some(7878), false, None));
        assert_eq!(port(Some(7878), &prefs(true, 9000)), (Some(7878), false, None));
        assert_eq!(port(None, &prefs(true, 9000)), (Some(9000), true, None));
        assert_eq!(port(None, &prefs(true, 0)), (None, false, None), "0: only with --control");
        assert_eq!(port(None, &prefs(true, 70_000)), (None, false, None), "out of range");
    }

    #[test]
    fn allow_agent_control_off_keeps_it_closed() {
        let (p, _, warning) = port(Some(7878), &prefs(false, 9000));
        assert_eq!(p, None);
        assert!(warning.is_some_and(|w| w.contains("Integrations")));
        assert_eq!(port(None, &prefs(false, 9000)), (None, false, None));
    }

    #[test]
    fn reads_the_preference_file_leniently() {
        let dir = std::env::temp_dir().join(format!("photocraft-agent-control-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("preferences.json");
        std::fs::write(&file, r#"{"integrations":{"allowAgentControl":false,"controlPort":7979}}"#).unwrap();
        assert_eq!(read(Some(&file)), prefs(false, 7979));
        std::fs::write(&file, "not json").unwrap();
        assert_eq!(read(Some(&file)), Integrations::default());
        assert_eq!(read(None), Integrations::default());
        assert_eq!(default_token_file(Some(dir.clone())), Some(dir.join("control.token")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
