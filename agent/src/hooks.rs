use crate::config::{parse_duration_secs, HookConfig};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

pub enum HookOutcome {
    /// No script configured, or it ran (or was launched, for non-blocking)
    /// without triggering an abort.
    Ok,
    /// The script failed or timed out and on_failure = "abort_to_rescue".
    Abort(String),
}

/// Runs a configured startup/shutdown hook script per its blocking/timeout/
/// on_failure settings. Non-blocking hooks are spawned and left running;
/// their stdio inherits ours (already redirected to the log file by main.rs).
pub fn run(hook: &HookConfig, label: &str) -> HookOutcome {
    let Some(script) = hook.script.as_deref() else {
        return HookOutcome::Ok;
    };
    if !Path::new(script).exists() {
        return HookOutcome::Ok;
    }

    let mut child = match Command::new("/bin/sh").arg(script).spawn() {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("{label} hook '{script}' failed to start: {e}");
            eprintln!("{msg}");
            return finish(hook, msg);
        }
    };

    if !hook.blocking {
        return HookOutcome::Ok;
    }

    let timeout = parse_duration_secs(&hook.timeout)
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(30));
    let deadline = Instant::now() + timeout;

    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return HookOutcome::Ok,
            Ok(Some(status)) => {
                let msg = format!("{label} hook '{script}' exited with {status}");
                eprintln!("{msg}");
                return finish(hook, msg);
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let msg = format!("{label} hook '{script}' timed out after {timeout:?}");
                    eprintln!("{msg}");
                    return finish(hook, msg);
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(e) => {
                let msg = format!("{label} hook '{script}': error waiting for it: {e}");
                eprintln!("{msg}");
                return finish(hook, msg);
            }
        }
    }
}

fn finish(hook: &HookConfig, msg: String) -> HookOutcome {
    if hook.on_failure == "abort_to_rescue" {
        HookOutcome::Abort(msg)
    } else {
        HookOutcome::Ok
    }
}
