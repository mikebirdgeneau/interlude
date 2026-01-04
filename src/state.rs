use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::scheduler::{Config, Phase, Scheduler};

const SAVE_INTERVAL: Duration = Duration::from_secs(1);
const STATE_FILE: &str = "state.txt";

fn state_dir() -> Option<PathBuf> {
    if let Some(dir) = env::var_os("XDG_STATE_HOME") {
        return Some(PathBuf::from(dir).join("interlude"));
    }
    env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state/interlude"))
}

fn state_path() -> Option<PathBuf> {
    state_dir().map(|dir| dir.join(STATE_FILE))
}

fn phase_to_str(phase: Phase) -> &'static str {
    match phase {
        Phase::Working => "Working",
        Phase::LockedAwaitingAction => "LockedAwaitingAction",
        Phase::OnBreak => "OnBreak",
        Phase::BreakFinished => "BreakFinished",
        Phase::Snoozing => "Snoozing",
    }
}

fn str_to_phase(s: &str) -> Option<Phase> {
    match s {
        "Working" => Some(Phase::Working),
        "LockedAwaitingAction" => Some(Phase::LockedAwaitingAction),
        "OnBreak" => Some(Phase::OnBreak),
        "BreakFinished" => Some(Phase::BreakFinished),
        "Snoozing" => Some(Phase::Snoozing),
        _ => None,
    }
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn save_scheduler(sched: &Scheduler) -> std::io::Result<()> {
    let Some(path) = state_path() else {
        return Ok(());
    };
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let remaining = sched.time_left().map(|d| d.as_secs());
    let content = format!(
        "phase={}\nremaining={}\nsnooze_count={}\nsaved_at={}\n",
        phase_to_str(sched.phase),
        remaining
            .map(|v| v.to_string())
            .unwrap_or_else(|| "none".to_string()),
        sched.snooze_count,
        now_unix_secs()
    );
    let tmp_path = path.with_extension("tmp");
    fs::write(&tmp_path, content)?;
    fs::rename(tmp_path, path)?;
    Ok(())
}

pub fn load_scheduler(cfg: &Config) -> Option<Scheduler> {
    let path = state_path()?;
    let data = fs::read_to_string(path).ok()?;

    let mut phase: Option<Phase> = None;
    let mut remaining: Option<u64> = None;
    let mut snooze_count: Option<u32> = None;
    let mut saved_at: Option<u64> = None;

    for line in data.lines() {
        let (key, value) = line.split_once('=')?;
        match key {
            "phase" => phase = str_to_phase(value.trim()),
            "remaining" => {
                if value.trim() == "none" {
                    remaining = None;
                } else {
                    remaining = value.trim().parse::<u64>().ok();
                }
            }
            "snooze_count" => snooze_count = value.trim().parse::<u32>().ok(),
            "saved_at" => saved_at = value.trim().parse::<u64>().ok(),
            _ => {}
        }
    }

    let phase = phase?;
    let snooze_count = snooze_count.unwrap_or(0);
    let saved_at = saved_at.unwrap_or(now_unix_secs());
    let elapsed = now_unix_secs().saturating_sub(saved_at);
    let remaining = remaining.map(|r| r.saturating_sub(elapsed));

    let mut sched = Scheduler::new(cfg.clone());
    sched.phase = phase;
    sched.snooze_count = snooze_count;
    sched.deadline = match sched.phase {
        Phase::Working => remaining.map(|r| std::time::Instant::now() + Duration::from_secs(r)),
        Phase::Snoozing => remaining.map(|r| std::time::Instant::now() + Duration::from_secs(r)),
        Phase::OnBreak => remaining.map(|r| std::time::Instant::now() + Duration::from_secs(r)),
        Phase::LockedAwaitingAction => None,
        Phase::BreakFinished => None,
    };

    if let Some(0) = remaining {
        match sched.phase {
            Phase::Working | Phase::Snoozing => {
                sched.phase = Phase::LockedAwaitingAction;
                sched.deadline = None;
            }
            Phase::OnBreak => {
                sched.phase = Phase::BreakFinished;
                sched.deadline = None;
            }
            _ => {}
        }
    }

    Some(sched)
}

pub fn save_interval() -> Duration {
    SAVE_INTERVAL
}

pub fn clear_saved_state() -> std::io::Result<()> {
    let Some(path) = state_path() else {
        return Ok(());
    };
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}
