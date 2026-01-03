use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Working,
    LockedAwaitingAction,
    OnBreak,
    BreakFinished,
    Snoozing,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub interval: Duration,
    pub break_len: Duration,
    pub snooze_base: Duration,
    pub snooze_decay: f64,
    pub snooze_min: Duration,
    pub max_snoozes: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct Scheduler {
    pub phase: Phase,
    pub deadline: Option<Instant>,
    pub snooze_count: u32,
    pub cfg: Config,
}

impl Scheduler {
    pub fn new(cfg: Config) -> Self {
        Self {
            phase: Phase::Working,
            deadline: Some(Instant::now() + cfg.interval),
            snooze_count: 0,
            cfg,
        }
    }

    pub fn tick(&mut self) {
        if let Some(dl) = self.deadline {
            if Instant::now() >= dl {
                match self.phase {
                    Phase::Working => {
                        self.phase = Phase::LockedAwaitingAction;
                        self.deadline = None;
                    }
                    Phase::OnBreak => {
                        self.phase = Phase::BreakFinished;
                        self.deadline = None;
                    }
                    Phase::Snoozing => {
                        self.phase = Phase::LockedAwaitingAction;
                        self.deadline = None;
                    }
                    _ => {}
                }
            }
        }
    }

    pub fn time_left(&self) -> Option<Duration> {
        self.deadline
            .map(|d| d.saturating_duration_since(Instant::now()))
    }

    pub fn snooze_duration(&self) -> Duration {
        let base = self.cfg.snooze_base.as_secs_f64();
        let dec = self.cfg.snooze_decay.clamp(0.01, 0.999);
        let dur = base * dec.powi(self.snooze_count as i32);
        let dur = dur.round().max(self.cfg.snooze_min.as_secs_f64());
        Duration::from_secs(dur as u64)
    }

    pub fn can_snooze(&self) -> bool {
        match self.cfg.max_snoozes {
            None => true,
            Some(n) => self.snooze_count < n,
        }
    }

    pub fn start_break(&mut self) {
        self.phase = Phase::OnBreak;
        self.deadline = Some(Instant::now() + self.cfg.break_len);
    }

    pub fn finish_and_restart(&mut self) {
        self.phase = Phase::Working;
        self.deadline = Some(Instant::now() + self.cfg.interval);
        self.snooze_count = 0;
    }

    pub fn snooze(&mut self) -> Duration {
        let d = self.snooze_duration();
        self.snooze_count = self.snooze_count.saturating_add(1);
        self.phase = Phase::Snoozing;
        self.deadline = Some(Instant::now() + d);
        d
    }
}

