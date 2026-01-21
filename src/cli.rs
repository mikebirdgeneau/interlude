use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[command(name = "interlude", about = "Wayland session-lock break enforcer")]
pub struct Cli {
    /// Minutes between breaks after the first one
    #[arg(long, default_value_t = 30)]
    pub interval_minutes: u64,

    /// Minutes before the first break
    #[arg(long, default_value_t = 60)]
    pub initial_interval_minutes: u64,

    /// Break duration in seconds after the first break
    #[arg(long, default_value_t = 180)]
    pub break_seconds: u64,

    /// Initial break duration in seconds
    #[arg(long, default_value_t = 300)]
    pub initial_break_seconds: u64,

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

    /// Immediately start a break sequence (for testing)
    #[arg(long, default_value_t = false)]
    pub immediate: bool,

    /// Background overlay color in hex (#RGB, #RRGGBB, or #RRGGBBAA)
    #[arg(long, default_value = "#000000CC")]
    pub background: String,

    /// Foreground text/icon color in hex (#RGB, #RRGGBB, or #RRGGBBAA)
    #[arg(long, default_value = "#FFFFFDDD")]
    pub foreground: String,

    /// Target FPS during fade animations (lower = less compositor load)
    #[arg(long, default_value_t = 60)]
    pub fade_fps: u32,

    /// Ignore any saved timer state and start fresh
    #[arg(long, default_value_t = false)]
    pub reset_state: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_defaults() {
        let cli = Cli::try_parse_from(["interlude"]).expect("default parse");
        assert_eq!(cli.interval_minutes, 30);
        assert_eq!(cli.initial_interval_minutes, 60);
        assert_eq!(cli.break_seconds, 180);
        assert_eq!(cli.initial_break_seconds, 300);
        assert_eq!(cli.snooze_base_seconds, 300);
        assert_eq!(cli.snooze_decay, 0.6);
        assert_eq!(cli.snooze_min_seconds, 30);
        assert_eq!(cli.max_snoozes, 0);
        assert!(!cli.immediate);
        assert_eq!(cli.background, "#000000CC");
        assert_eq!(cli.foreground, "#FFFFFDDD");
        assert_eq!(cli.fade_fps, 60);
        assert!(!cli.reset_state);
    }

    #[test]
    fn parse_overrides() {
        let cli = Cli::try_parse_from([
            "interlude",
            "--interval-minutes",
            "25",
            "--initial-interval-minutes",
            "90",
            "--break-seconds",
            "120",
            "--initial-break-seconds",
            "240",
            "--snooze-base-seconds",
            "240",
            "--snooze-decay",
            "0.75",
            "--snooze-min-seconds",
            "45",
            "--max-snoozes",
            "3",
            "--immediate",
            "--background",
            "#11223344",
            "--foreground",
            "#abcdef",
            "--fade-fps",
            "24",
            "--reset-state",
        ])
        .expect("custom parse");

        assert_eq!(cli.interval_minutes, 25);
        assert_eq!(cli.initial_interval_minutes, 90);
        assert_eq!(cli.break_seconds, 120);
        assert_eq!(cli.initial_break_seconds, 240);
        assert_eq!(cli.snooze_base_seconds, 240);
        assert_eq!(cli.snooze_decay, 0.75);
        assert_eq!(cli.snooze_min_seconds, 45);
        assert_eq!(cli.max_snoozes, 3);
        assert!(cli.immediate);
        assert_eq!(cli.background, "#11223344");
        assert_eq!(cli.foreground, "#abcdef");
        assert_eq!(cli.fade_fps, 24);
        assert!(cli.reset_state);
    }
}
