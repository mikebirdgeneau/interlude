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
        let now = Instant::now();
        if let Some(dl) = self.deadline
            && now >= dl {
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

    pub fn handle_session_locked(&mut self) {
        self.phase = Phase::Working;
        self.deadline = None;
        self.snooze_count = 0;
    }

    pub fn handle_session_unlocked(&mut self) {
        self.phase = Phase::Working;
        self.deadline = Some(Instant::now() + self.cfg.interval);
        self.snooze_count = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_cfg() -> Config {
        Config {
            interval: Duration::from_secs(10),
            break_len: Duration::from_secs(5),
            snooze_base: Duration::from_secs(100),
            snooze_decay: 0.5,
            snooze_min: Duration::from_secs(30),
            max_snoozes: Some(2),
        }
    }

    #[test]
    fn tick_transitions_working_to_locked() {
        let mut sched = Scheduler::new(test_cfg());
        sched.deadline = Some(Instant::now() - Duration::from_secs(1));
        sched.tick();
        assert_eq!(sched.phase, Phase::LockedAwaitingAction);
        assert!(sched.deadline.is_none());
    }

    #[test]
    fn tick_transitions_on_break_to_finished() {
        let mut sched = Scheduler::new(test_cfg());
        sched.start_break();
        sched.deadline = Some(Instant::now() - Duration::from_secs(1));
        sched.tick();
        assert_eq!(sched.phase, Phase::BreakFinished);
        assert!(sched.deadline.is_none());
    }

    #[test]
    fn snooze_duration_decays_with_floor() {
        let mut sched = Scheduler::new(test_cfg());
        sched.snooze_count = 0;
        assert_eq!(sched.snooze_duration().as_secs(), 100);
        sched.snooze_count = 1;
        assert_eq!(sched.snooze_duration().as_secs(), 50);
        sched.snooze_count = 2;
        assert_eq!(sched.snooze_duration().as_secs(), 30);
    }

    #[test]
    fn snooze_resets_after_finish() {
        let mut sched = Scheduler::new(test_cfg());
        let _ = sched.snooze();
        assert_eq!(sched.snooze_count, 1);
        sched.finish_and_restart();
        assert_eq!(sched.snooze_count, 0);
        assert_eq!(sched.phase, Phase::Working);
    }

    #[test]
    fn can_snooze_respects_max() {
        let mut sched = Scheduler::new(test_cfg());
        assert!(sched.can_snooze());
        let _ = sched.snooze();
        let _ = sched.snooze();
        assert!(!sched.can_snooze());
    }

    #[test]
    fn session_lock_clears_deadline() {
        let mut sched = Scheduler::new(test_cfg());
        sched.phase = Phase::LockedAwaitingAction;
        sched.deadline = Some(Instant::now() + Duration::from_secs(1));
        sched.snooze_count = 2;
        sched.handle_session_locked();
        assert_eq!(sched.phase, Phase::Working);
        assert!(sched.deadline.is_none());
        assert_eq!(sched.snooze_count, 0);
    }

    #[test]
    fn session_unlock_resets_interval() {
        let mut sched = Scheduler::new(test_cfg());
        sched.phase = Phase::BreakFinished;
        sched.deadline = None;
        sched.snooze_count = 2;
        let before = Instant::now();
        sched.handle_session_unlocked();
        assert_eq!(sched.phase, Phase::Working);
        let deadline = sched.deadline.expect("deadline should be set");
        assert!(deadline >= before + sched.cfg.interval);
        assert_eq!(sched.snooze_count, 0);
    }
}
