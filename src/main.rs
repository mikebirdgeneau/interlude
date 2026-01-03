use anyhow::Result;
use clap::Parser;
use crossbeam_channel::{unbounded, Receiver};

mod cli;
mod scheduler;
mod tiny_font;
mod wayland_lock;

use cli::Cli;
use scheduler::{Config, Phase, Scheduler};
use wayland_lock::{Locker, UiEvent, UiMode};

fn recv_all(rx: &Receiver<UiEvent>) -> Vec<UiEvent> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        out.push(ev);
    }
    out
}

fn main() -> Result<()> {
    let args = Cli::parse();

    let cfg = Config {
        interval: std::time::Duration::from_secs(args.interval_minutes * 60),
        break_len: std::time::Duration::from_secs(args.break_seconds),
        snooze_base: std::time::Duration::from_secs(args.snooze_base_seconds),
        snooze_decay: args.snooze_decay,
        snooze_min: std::time::Duration::from_secs(args.snooze_min_seconds),
        max_snoozes: if args.max_snoozes == 0 { None } else { Some(args.max_snoozes) },
    };

    let mut sched = Scheduler::new(cfg);

    let (tx_ui, rx_ui) = unbounded();
    let mut locker = Locker::new(tx_ui)?;

    loop {
        // Tick core scheduler
        sched.tick();

        // Ensure lock/unlock based on phase
        match sched.phase {
            Phase::Working | Phase::Snoozing => {
                if locker.is_locked() {
                    locker.unlock();
                }
            }
            Phase::LockedAwaitingAction | Phase::OnBreak | Phase::BreakFinished => {
                if !locker.is_locked() {
                    locker.lock()?;
                }
            }
        }

        // Update overlay UI mode (only meaningful when locked)
        if locker.is_locked() {
            match sched.phase {
                Phase::LockedAwaitingAction => {
                    let snooze = sched.snooze_duration().as_secs();
                    locker.set_mode(UiMode::BreakDue {
                        snooze_secs: snooze,
                        can_snooze: sched.can_snooze(),
                    });
                }
                Phase::OnBreak => {
                    let left = sched.time_left().map(|d| d.as_secs()).unwrap_or(0);
                    locker.set_mode(UiMode::OnBreak { secs_left: left });
                }
                Phase::BreakFinished => {
                    locker.set_mode(UiMode::BreakFinished);
                }
                _ => {}
            }
        }

        // Handle key events
        for ev in recv_all(&rx_ui) {
            match (sched.phase, ev) {
                (Phase::LockedAwaitingAction, UiEvent::PressEnter) => {
                    sched.start_break();
                }
                (Phase::LockedAwaitingAction, UiEvent::PressZ) => {
                    if sched.can_snooze() {
                        let _d = sched.snooze();
                        locker.unlock();
                    }
                }
                (Phase::BreakFinished, UiEvent::PressEnter) => {
                    // User explicitly dismisses, then restart interval
                    sched.finish_and_restart();
                    locker.unlock();
                }
                // During OnBreak, ignore Enter (you asked: dismiss only after break is over)
                _ => {}
            }
        }

        // Pump Wayland events when locked (keyboard input, configure, etc.)
        if locker.is_locked() {
            locker.pump()?;
        }

        std::thread::sleep(std::time::Duration::from_millis(150));
    }
}

