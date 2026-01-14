use anyhow::{Context, Result};
use std::time::{Duration, Instant};
use zbus::blocking::{Connection, Proxy};

pub struct InhibitorWatcher {
    check_interval: Duration,
    last_check: Instant,
    cached: bool,
    connection: Option<Connection>,
}

impl InhibitorWatcher {
    pub fn new(check_interval: Duration) -> Self {
        Self {
            check_interval,
            last_check: Instant::now() - check_interval,
            cached: false,
            connection: None,
        }
    }

    pub fn is_active(&mut self) -> bool {
        let now = Instant::now();
        if now.duration_since(self.last_check) < self.check_interval {
            return self.cached;
        }
        self.last_check = now;
        if self.connection.is_none() {
            match Connection::system() {
                Ok(conn) => self.connection = Some(conn),
                Err(err) => {
                    eprintln!("inhibitor check skipped: connect to system bus failed: {err}");
                    return self.cached;
                }
            }
        }
        let Some(conn) = &self.connection else {
            return self.cached;
        };
        match list_inhibitors(conn) {
            Ok(active) => self.cached = active,
            Err(err) => {
                eprintln!("inhibitor check failed: {err}");
            }
        }
        self.cached
    }
}

fn list_inhibitors(conn: &Connection) -> Result<bool> {
    let manager = Proxy::new(
        conn,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .context("create login1 manager proxy")?;
    let inhibitors: Vec<(String, String, String, String, u32, u32)> = manager
        .call("ListInhibitors", &())
        .context("ListInhibitors failed")?;
    Ok(inhibitors.iter().any(|(what, _who, _why, mode, _uid, _pid)| {
        mode == "block"
            && what
                .split(':')
                .any(|what| matches!(what, "sleep" | "idle"))
    }))
}
