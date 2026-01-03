use anyhow::Result;
use clap::Parser;
use crossbeam_channel::{Receiver, unbounded};

mod cli;
mod audio;
mod scheduler;
mod tiny_font;
mod wayland_lock;

use cli::Cli;
use scheduler::{Config, Phase, Scheduler};
use wayland_lock::{Locker, UiColors, UiEvent, UiMode};
use audio::Audio;

fn recv_all(rx: &Receiver<UiEvent>) -> Vec<UiEvent> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        out.push(ev);
    }
    out
}

fn fmt_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    let m = secs / 60;
    let s = secs % 60;
    format!("{:02}:{:02}", m, s)
}

fn main() -> Result<()> {
    let args = Cli::parse();

    let cfg = Config {
        interval: std::time::Duration::from_secs(args.interval_minutes * 60),
        break_len: std::time::Duration::from_secs(args.break_seconds),
        snooze_base: std::time::Duration::from_secs(args.snooze_base_seconds),
        snooze_decay: args.snooze_decay,
        snooze_min: std::time::Duration::from_secs(args.snooze_min_seconds),
        max_snoozes: if args.max_snoozes == 0 {
            None
        } else {
            Some(args.max_snoozes)
        },
    };

    let mut sched = Scheduler::new(cfg);
    let mut last_phase = sched.phase;
    if args.immediate {
        sched.phase = Phase::LockedAwaitingAction;
        sched.deadline = None;
        last_phase = Phase::Working;
    }

    let (tx_ui, rx_ui) = unbounded();
    let colors = UiColors {
        background: parse_color(&args.background).unwrap_or([0, 0, 0, 0xCC]),
        foreground: parse_color(&args.foreground).unwrap_or([0xFF, 0xFF, 0xFD, 0xDD]),
    };
    let mut locker = Locker::new(tx_ui, colors)?;
    let audio = Audio::new();

    loop {
        // Tick core scheduler
        sched.tick();

        // Handle key events
        if !locker.is_fading() {
            for ev in recv_all(&rx_ui) {
                match (sched.phase, ev) {
                    (Phase::LockedAwaitingAction, UiEvent::PressZ)
                    | (Phase::OnBreak, UiEvent::PressZ) => {
                        if sched.can_snooze() {
                            let _d = sched.snooze();
                            if locker.is_locked() {
                                locker.start_fade_out();
                            }
                        }
                    }
                    (Phase::BreakFinished, UiEvent::PressEnter)
                    | (Phase::BreakFinished, UiEvent::PointerClick)
                    | (Phase::BreakFinished, UiEvent::AnyKey) => {
                        if locker.is_locked() {
                            locker.start_fade_out();
                        }
                    }
                    _ => {}
                }
            }
        }

        // Ensure lock/unlock based on phase
        match sched.phase {
            Phase::Working | Phase::Snoozing => {}
            Phase::LockedAwaitingAction | Phase::OnBreak | Phase::BreakFinished => {}
        }

        if matches!(sched.phase, Phase::LockedAwaitingAction | Phase::OnBreak | Phase::BreakFinished)
            && !locker.is_locked()
        {
            locker.lock()?;
        }

        if sched.phase == Phase::LockedAwaitingAction && last_phase != Phase::LockedAwaitingAction {
            locker.start_fade_in();
        }

        if sched.phase == Phase::OnBreak && last_phase != Phase::OnBreak {
            if let Some(audio) = &audio {
                audio.play_start();
            }
        }

        if sched.phase == Phase::BreakFinished && last_phase != Phase::BreakFinished {
            if let Some(audio) = &audio {
                audio.play_end();
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

        // Fade updates and auto-dismiss when finished.
        if locker.is_locked() {
            if sched.phase == Phase::LockedAwaitingAction && locker.take_fade_in_complete() {
                sched.start_break();
            }
            if matches!(sched.phase, Phase::LockedAwaitingAction | Phase::OnBreak)
                && !locker.is_fading()
            {
                locker.ensure_input_capture();
            }
            let fade_out_done = locker.update_fade();
            if fade_out_done {
                locker.unlock();
                if sched.phase == Phase::BreakFinished {
                    sched.finish_and_restart();
                }
            }
        } else if matches!(sched.phase, Phase::Working | Phase::Snoozing) && !locker.is_fading() {
            locker.unlock();
        }

        // Pump Wayland events when locked (keyboard input, configure, etc.)
        if locker.is_locked() {
            locker.pump()?;
        }

        if sched.phase != last_phase {
            match sched.phase {
                Phase::LockedAwaitingAction => {
                    println!(
                        "Break Starting (duration {})",
                        fmt_duration(sched.cfg.break_len)
                    );
                }
                Phase::Snoozing => {
                    let next = sched.time_left().unwrap_or(sched.cfg.snooze_min);
                    println!("Snoozed (break in {})", fmt_duration(next));
                }
                Phase::BreakFinished => {
                    println!(
                        "Break Complete (next in {})",
                        fmt_duration(sched.cfg.interval)
                    );
                }
                _ => {}
            }
        }

        last_phase = sched.phase;
        let sleep_ms = if locker.is_fading() { 16 } else { 150 };
        std::thread::sleep(std::time::Duration::from_millis(sleep_ms));
    }
}

fn parse_color(input: &str) -> Option<[u8; 4]> {
    let hex = input.trim();
    if !hex.starts_with('#') {
        return None;
    }
    let hex = &hex[1..];
    let expanded = match hex.len() {
        3 => {
            let chars: Vec<char> = hex.chars().collect();
            format!(
                "{}{}{}{}{}{}FF",
                chars[0], chars[0], chars[1], chars[1], chars[2], chars[2]
            )
        }
        6 => format!("{hex}FF"),
        8 => hex.to_string(),
        _ => return None,
    };
    if expanded.len() != 8 {
        return None;
    }
    let r = u8::from_str_radix(&expanded[0..2], 16).ok()?;
    let g = u8::from_str_radix(&expanded[2..4], 16).ok()?;
    let b = u8::from_str_radix(&expanded[4..6], 16).ok()?;
    let a = u8::from_str_radix(&expanded[6..8], 16).ok()?;
    Some([r, g, b, a])
}
