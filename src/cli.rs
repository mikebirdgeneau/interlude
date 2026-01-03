use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[command(name = "interlude", about = "Wayland session-lock break enforcer")]
pub struct Cli {
    /// Minutes between breaks
    #[arg(long, default_value_t = 30)]
    pub interval_minutes: u64,

    /// Break duration in seconds
    #[arg(long, default_value_t = 60)]
    pub break_seconds: u64,

    /// Initial snooze duration in seconds (shrinks each snooze)
    #[arg(long, default_value_t = 300)]
    pub snooze_base_seconds: u64,

    /// Snooze decay multiplier applied each time you snooze (0 < decay < 1)
    #[arg(long, default_value_t = 0.6)]
    pub snooze_decay: f64,

    /// Minimum snooze duration in seconds
    #[arg(long, default_value_t = 30)]
    pub snooze_min_seconds: u64,

    /// Optional: after N snoozes in a cycle, disable snooze (0 = unlimited)
    #[arg(long, default_value_t = 0)]
    pub max_snoozes: u32,
}

