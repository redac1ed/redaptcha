use std::time::{Duration, Instant};

pub const DIFFICULTY_MIN: u64 = 3_000;
pub const DIFFICULTY_MAX: u64 = 60_000;
pub const RATE_WINDOW: Duration = Duration::from_secs(60);
pub const MAX_CHALLENGES_PER_WINDOW: u32 = 20;
pub const VERIFY_RATE_WINDOW: Duration = Duration::from_secs(60);
pub const MAX_VERIFY_PER_WINDOW: u32 = 30;
pub const SUSPICION_DECAY: Duration = Duration::from_secs(300);
pub const FRAME_COUNT: u32 = 64;
pub const FRAME_DT_MS: f64 = 50.0;
pub const MIN_TRAIL_POINTS: usize = 6;
pub const MIN_TRAIL_PATH: f64 = 40.0;
pub const MIN_REACTION_MS: f64 = 120.0;
pub const MAX_TELEPORT_FRAC: f64 = 0.6;

#[derive(Clone)]
pub struct ClientProfile {
    pub count: u32,
    pub window_start: Instant,
    pub failures: u32,
    pub successes: u32,
    pub last_seen: Instant,
    pub suspicion: f64,
    pub verify_count: u32,
    pub verify_window_start: Instant,
}

pub struct TrailStats {
    pub points: usize,
    pub path_len: f64,
    pub reaction_ms: f64,
    pub max_step: f64,
    pub span: f64,
}

impl ClientProfile {
    pub fn new(now: Instant) -> Self {
        ClientProfile {
            count: 0,
            window_start: now,
            failures: 0,
            successes: 0,
            last_seen: now,
            suspicion: 0.0,
            verify_count: 0,
            verify_window_start: now,
        }
    }
    pub fn roll_window(&mut self, now: Instant) {
        if now.duration_since(self.window_start) > RATE_WINDOW {
            self.count = 0;
            self.window_start = now;
        }
        let elapsed = now.duration_since(self.last_seen);
        if elapsed > SUSPICION_DECAY {
            self.suspicion = 0.0;
        } else {
            let decay = elapsed.as_secs_f64() / SUSPICION_DECAY.as_secs_f64();
            self.suspicion = (self.suspicion - decay).max(0.0);
        }
        self.last_seen = now;
    }
    pub fn register_request(&mut self) -> bool {
        if self.count >= MAX_CHALLENGES_PER_WINDOW {
            return false;
        }
        self.count += 1;
        true
    }
    pub fn register_verify(&mut self, now: Instant) -> bool {
        if now.duration_since(self.verify_window_start) > VERIFY_RATE_WINDOW {
            self.verify_count = 0;
            self.verify_window_start = now;
        }
        if self.verify_count >= MAX_VERIFY_PER_WINDOW {
            return false;
        }
        self.verify_count += 1;
        true
    }
    pub fn record_failure(&mut self) {
        self.failures = self.failures.saturating_add(1);
        self.suspicion = (self.suspicion + 0.25).min(1.0);
    }
    pub fn record_success(&mut self) {
        self.successes = self.successes.saturating_add(1);
        self.suspicion = (self.suspicion - 0.1).max(0.0);
    }
    pub fn flag_anomaly(&mut self, weight: f64) {
        self.suspicion = (self.suspicion + weight).clamp(0.0, 1.0);
    }
}

pub fn difficulty_for(profile: &ClientProfile) -> u64 {
    let rate_frac = (profile.count as f64 / MAX_CHALLENGES_PER_WINDOW as f64).clamp(0.0, 1.0);
    let rate_curve = rate_frac * rate_frac;
    let fail_signal = if profile.successes + profile.failures == 0 {
        0.0
    } else {
        profile.failures as f64 / (profile.successes + profile.failures) as f64
    };
    let blended = (0.55 * rate_curve + 0.25 * profile.suspicion + 0.20 * fail_signal).clamp(0.0, 1.0);
    let span = (DIFFICULTY_MAX - DIFFICULTY_MIN) as f64;
    let raw = DIFFICULTY_MIN as f64 + span * blended;
    round_to_step(raw as u64, 500).clamp(DIFFICULTY_MIN, DIFFICULTY_MAX)
}

fn round_to_step(value: u64, step: u64) -> u64 {
    if step == 0 {
        return value;
    }
    let rem = value % step;
    if rem == 0 {
        value
    } else {
        value - rem
    }
}

pub fn classify_timing(total_ms: f64, click_count: usize) -> Option<f64> {
    if click_count == 0 {
        return None;
    }
    let per_click = total_ms / click_count as f64;
    if per_click < 60.0 {
        Some(0.4)
    } else if per_click < 120.0 {
        Some(0.15)
    } else {
        None
    }
}

pub fn trail_stats(trail: &[(f64, f64, f64)]) -> TrailStats {
    if trail.len() < 2 {
        return TrailStats {
            points: trail.len(),
            path_len: 0.0,
            reaction_ms: 0.0,
            max_step: 0.0,
            span: 0.0,
        };
    }
    let mut path_len = 0.0;
    let mut max_step = 0.0;
    for w in trail.windows(2) {
        let d = ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt();
        path_len += d;
        if d > max_step {
            max_step = d;
        }
    }
    let reaction_ms = trail.first().unwrap().2;
    let span = trail.last().unwrap().2 - trail.first().unwrap().2;
    TrailStats {
        points: trail.len(),
        path_len,
        reaction_ms,
        max_step,
        span,
    }
}

pub fn grade_trail(trail: &[(f64, f64, f64)]) -> Result<f64, &'static str> {
    let st = trail_stats(trail);
    if st.points < MIN_TRAIL_POINTS {
        return Err("no human movement");
    }
    if st.path_len < MIN_TRAIL_PATH {
        return Err("movement too small");
    }
    if st.span < MIN_REACTION_MS {
        return Err("movement too brief");
    }
    if st.path_len > 0.0 && (st.max_step / st.path_len) > MAX_TELEPORT_FRAC {
        return Err("teleport detected");
    }
    let mut weight = 0.0;
    if st.reaction_ms < 50.0 {
        weight += 0.1;
    }
    if st.points < MIN_TRAIL_POINTS * 2 {
        weight += 0.1;
    }
    Ok(weight) 
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fresh() -> ClientProfile {
        ClientProfile::new(Instant::now())
    }
    #[test]
    fn fresh_client_gets_minimum() {
        let p = fresh();
        assert_eq!(difficulty_for(&p), DIFFICULTY_MIN);
    }
    #[test]
    fn heavy_rate_increases_difficulty() {
        let mut p = fresh();
        p.count = MAX_CHALLENGES_PER_WINDOW;
        let d = difficulty_for(&p);
        assert!(d > DIFFICULTY_MIN);
        assert!(d <= DIFFICULTY_MAX);
    }
    #[test]
    fn failures_raise_difficulty() {
        let mut low = fresh();
        low.count = 4;
        let baseline = difficulty_for(&low);
        let mut suspicious = fresh();
        suspicious.count = 4;
        for _ in 0..6 {
            suspicious.record_failure();
        }
        suspicious.failures = 8;
        suspicious.successes = 1;
        assert!(difficulty_for(&suspicious) > baseline);
    }
    #[test]
    fn never_exceeds_max() {
        let mut p = fresh();
        p.count = MAX_CHALLENGES_PER_WINDOW;
        p.suspicion = 1.0;
        p.failures = 100;
        p.successes = 0;
        assert!(difficulty_for(&p) <= DIFFICULTY_MAX);
    }
    #[test]
    fn rate_limit_blocks_after_cap() {
        let mut p = fresh();
        for _ in 0..MAX_CHALLENGES_PER_WINDOW {
            assert!(p.register_request());
        }
        assert!(!p.register_request());
    }
    #[test]
    fn timing_classifier_flags_fast_bots() {
        assert!(classify_timing(90.0, 3).is_some());
        assert!(classify_timing(2000.0, 3).is_none());
        assert!(classify_timing(0.0, 0).is_none());
    }
    #[test]
    fn trail_rejects_teleport() {
        let trail = vec![
            (0.0, 0.0, 0.0),
            (1.0, 1.0, 50.0),
            (200.0, 200.0, 100.0),
            (201.0, 201.0, 150.0),
            (202.0, 202.0, 200.0),
            (203.0, 203.0, 250.0),
        ];
        assert!(grade_trail(&trail).is_err());
    }
    #[test]
    fn trail_accepts_human_drag() {
        let mut trail = vec![];
        for i in 0..20 {
            let f = i as f64;
            trail.push((f * 5.0, f * 3.0, 150.0 + f * 40.0));
        }
        assert!(grade_trail(&trail).is_ok());
    }
    #[test]
    fn trail_rejects_empty() {
        assert!(grade_trail(&[]).is_err());
    }
}
