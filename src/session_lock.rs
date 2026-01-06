use anyhow::{Context, Result, anyhow};
use crossbeam_channel::Sender;
use rustix::process::getuid;
use std::collections::HashMap;
use std::thread;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionLockEvent {
    Locked,
    Unlocked,
}

pub fn spawn_session_lock_watcher(tx: Sender<SessionLockEvent>) -> Result<()> {
    thread::Builder::new()
        .name("session-lock-watcher".to_string())
        .spawn(move || {
            if let Err(err) = watch_session_lock(tx) {
                eprintln!("session lock watcher failed: {err:?}");
            }
        })
        .context("spawn session lock watcher thread")?;
    Ok(())
}

fn watch_session_lock(tx: Sender<SessionLockEvent>) -> Result<()> {
    let connection = Connection::system().context("connect to system bus")?;
    let manager = Proxy::new(
        &connection,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .context("create login1 manager proxy")?;

    let session_path = get_session_path(&connection, &manager)?;
    let session = Proxy::new(
        &connection,
        "org.freedesktop.login1",
        session_path,
        "org.freedesktop.login1.Session",
    )
    .context("create login1 session proxy")?;

    let mut locked = session.get_property::<bool>("LockedHint").unwrap_or(false);
    let mut signals = session
        .receive_signal("PropertiesChanged")
        .context("subscribe to PropertiesChanged")?;

    for msg in signals.by_ref() {
        let (iface, changed, _invalidated): (String, HashMap<String, Value>, Vec<String>) =
            msg.body().context("decode PropertiesChanged signal")?;
        if iface != "org.freedesktop.login1.Session" {
            continue;
        }
        if let Some(new_locked) = extract_locked_hint(&changed) {
            if new_locked != locked {
                locked = new_locked;
                let _ = tx.send(if locked {
                    SessionLockEvent::Locked
                } else {
                    SessionLockEvent::Unlocked
                });
            }
            continue;
        }
        if let Some(new_locked) = extract_state_lock(&changed)
            && new_locked != locked {
                locked = new_locked;
                let _ = tx.send(if locked {
                    SessionLockEvent::Locked
                } else {
                    SessionLockEvent::Unlocked
                });
            }
    }

    Ok(())
}

fn get_session_path(connection: &Connection, manager: &Proxy) -> Result<OwnedObjectPath> {
    if let Ok(session_id) = std::env::var("XDG_SESSION_ID") {
        let path: OwnedObjectPath = manager
            .call("GetSession", &(session_id))
            .context("GetSession failed")?;
        return Ok(path);
    }
    let pid = std::process::id();
    let pid_err = match manager.call("GetSessionByPID", &(pid)) {
        Ok(path) => return Ok(path),
        Err(err) => err,
    };

    let paths = match list_user_session_paths(manager) {
        Ok(paths) => paths,
        Err(list_err) => {
            return Err(anyhow!(
                "GetSessionByPID failed: {pid_err}; ListSessions failed: {list_err}"
            ));
        }
    };

    if let Some(path) = find_active_session(connection, &paths) {
        return Ok(path);
    }

    if let Some(path) = paths.into_iter().next() {
        return Ok(path);
    }

    Err(anyhow!(
        "GetSessionByPID failed: {pid_err}; no login1 sessions found for uid {}",
        getuid().as_raw()
    ))
}

fn extract_locked_hint(changed: &HashMap<String, Value>) -> Option<bool> {
    let value = changed.get("LockedHint")?;
    bool::try_from(value.clone()).ok()
}

fn extract_state_lock(changed: &HashMap<String, Value>) -> Option<bool> {
    let value = changed.get("State")?;
    let state = String::try_from(value.clone()).ok()?;
    match state.as_str() {
        "active" | "online" => Some(false),
        _ => Some(true),
    }
}

fn list_user_session_paths(manager: &Proxy) -> Result<Vec<OwnedObjectPath>> {
    let uid = getuid().as_raw();
    let sessions: Vec<(String, u32, String, String, OwnedObjectPath)> = manager
        .call("ListSessions", &())
        .context("ListSessions failed")?;
    Ok(sessions
        .into_iter()
        .filter(|(_, sess_uid, _, _, _)| *sess_uid == uid)
        .map(|(_, _, _, _, path)| path)
        .collect())
}

fn find_active_session(
    connection: &Connection,
    paths: &[OwnedObjectPath],
) -> Option<OwnedObjectPath> {
    for path in paths {
        let session = Proxy::new(
            connection,
            "org.freedesktop.login1",
            path.clone(),
            "org.freedesktop.login1.Session",
        )
        .ok()?;
        if let Ok(true) = session.get_property::<bool>("Active") {
            return Some(path.clone());
        }
        if let Ok(state) = session.get_property::<String>("State") {
            if matches!(state.as_str(), "active" | "online") {
                return Some(path.clone());
            }
        }
    }
    None
}
