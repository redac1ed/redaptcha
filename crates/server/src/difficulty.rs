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
pub const GLOBAL_RATE_WINDOW: Duration = Duration::from_secs(60);
pub const MAX_GLOBAL_PER_WINDOW: u32 = 600;
pub const TOUCH_MIN_TRAIL_POINTS: usize = 3;
pub const TOUCH_MIN_TRAIL_PATH: f64 = 8.0;
pub const TOUCH_MIN_SPAN_MS: f64 = 30.0;
pub const TRUST_PASS_THRESHOLD: f64 = 0.60;
pub const TRUST_STEPUP_THRESHOLD: f64 = 0.35;
pub const PURSUIT_WINDOW_MS: f64 = 320.0;
pub const PURSUIT_MIN_POINTS: usize = 4;
pub const PURSUIT_LANDING_MAX_PX: f64 = 60.0;
pub const PURSUIT_MIN_PATH_PX: f64 = 12.0;
pub const MIN_HUMAN_MOVES: u32 = 8;

pub struct GlobalLimiter {
    pub count: u32,
    pub window_start: Instant,
}

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
    pub solved_rounds: std::collections::HashMap<String, (u32, Instant)>
}

pub struct TrailStats {
    pub points: usize,
    pub path_len: f64,
    pub reaction_ms: f64,
    pub max_step: f64,
    pub span: f64,
}

pub struct TrustInputs {
    pub trail_weight: f64,
    pub timing_weight: f64,
    pub grade_score: f64,
    pub suspicion: f64,
    pub fail_ratio: f64,
    pub page_load_to_first_move_ms: Option<f64>,
    pub focus_events: u32,
    pub blur_events: u32,
    pub scroll_events: u32,
    pub key_events: u32,
    pub move_events: u32,
    pub has_touch: bool,
    pub max_pressure: f64,
    pub webdriver: bool,
    pub input_type: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum TrustDecision {
    Pass,
    StepUp,
    Fail,
}

impl GlobalLimiter {
    pub fn new(now: Instant) -> Self { GlobalLimiter { count: 0, window_start: now } }
    pub fn allow(&mut self, now: Instant) -> bool {
        if now.duration_since(self.window_start) > GLOBAL_RATE_WINDOW {
            self.count = 0;
            self.window_start = now;
        }
        if self.count >= MAX_GLOBAL_PER_WINDOW { return false; }
        self.count += 1;
        true
    }
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
            solved_rounds: std::collections::HashMap::new(),
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
    pub fn record_round(&mut self, kind: &str, now: Instant, window: Duration) -> u32 {
        let entry = self.solved_rounds.entry(kind.to_string()).or_insert((0, now));
        if now.duration_since(entry.1) > window {
            *entry = (0, now);
        }
        entry.0 += 1;
        entry.1 = now;
        entry.0
    }
    pub fn reset_rounds(&mut self, kind: &str) {
        self.solved_rounds.remove(kind);
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
    pub fn fail_ratio(&self) -> f64 {
        if self.successes + self.failures == 0 {
            0.0
        } else {
            self.failures as f64 / (self.successes + self.failures) as f64
        }
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

pub fn grade_trail(trail: &[(f64, f64, f64)], is_touch: bool) -> Result<f64, &'static str> {
    let st = trail_stats(trail);
    let min_points = if is_touch { TOUCH_MIN_TRAIL_POINTS } else { MIN_TRAIL_POINTS };
    let min_path = if is_touch { TOUCH_MIN_TRAIL_PATH } else { MIN_TRAIL_PATH };
    let min_span = if is_touch { TOUCH_MIN_SPAN_MS } else { MIN_REACTION_MS };
    if st.points < min_points {
        return Err("no human movement");
    }
    if st.path_len < min_path {
        return Err("movement too small");
    }
    if st.span < min_span {
        return Err("movement too brief");
    }
    if st.path_len > 0.0 && (st.max_step / st.path_len) > MAX_TELEPORT_FRAC {
        return Err("teleport detected");
    }
    let mut weight = 0.0;
    if !is_touch && st.reaction_ms < 50.0 {
        weight += 0.1;
    }
    if !is_touch && st.points < MIN_TRAIL_POINTS * 2 {
        weight += 0.1;
    }
    Ok(weight)
}

pub fn pursuit_coherent(trail: &[(f64, f64, f64)], clicks: &[(f64, f64, f64)]) -> Result<(), &'static str> {
    if clicks.is_empty() || trail.is_empty() {
        return Ok(());
    }
    for c in clicks {
        let lead: Vec<&(f64, f64, f64)> = trail
            .iter()
            .filter(|p| p.2 <= c.2 && p.2 >= c.2 - PURSUIT_WINDOW_MS)
            .collect();
        if lead.len() < PURSUIT_MIN_POINTS {
            continue;
        }
        let last = lead.last().unwrap();
        let landing = ((last.0 - c.0).powi(2) + (last.1 - c.1).powi(2)).sqrt();
        if landing > PURSUIT_LANDING_MAX_PX {
            return Err("click far from cursor");
        }
        let mut seg = 0.0;
        let mut max_seg = 0.0;
        for w in lead.windows(2) {
            let d = ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt();
            seg += d;
            if d > max_seg {
                max_seg = d;
            }
        }
        if seg > PURSUIT_MIN_PATH_PX && (max_seg / seg) > MAX_TELEPORT_FRAC {
            return Err("teleport before click");
        }
    }
    Ok(())
}

pub fn trust_score(t: &TrustInputs) -> f64 {
    let mut score = 1.0;
    score -= t.suspicion * 0.30;
    score -= t.fail_ratio * 0.20;
    score -= t.trail_weight * 0.15;
    score -= t.timing_weight * 0.15;
    score += (t.grade_score - 0.5) * 0.20;
    match t.page_load_to_first_move_ms {
        Some(ms) if ms < 80.0 => score -= 0.20,
        Some(ms) if ms < 200.0 => score -= 0.10,
        Some(_) => score += 0.03,
        None => score -= 0.12,
    }
    if t.focus_events == 0 && t.blur_events == 0 {
        score -= 0.05;
    }
    if t.scroll_events == 0 {
        score -= 0.02;
    }
    if t.key_events == 0 {
        score -= 0.02;
    }
    if t.move_events < 8 {
        score -= 0.05;
    }
    if t.webdriver {
        score -= 0.25;
    }
    let is_touch = t.input_type.eq_ignore_ascii_case("touch")
        || t.input_type.eq_ignore_ascii_case("pen")
        || t.has_touch;
    if is_touch {
        score += 0.03;
    } else if t.max_pressure <= 0.0 {
        score -= 0.03;
    }
    score.clamp(0.0, 1.0)
}

pub fn trust_decision(score: f64) -> TrustDecision {
    if score >= TRUST_PASS_THRESHOLD {
        TrustDecision::Pass
    } else if score >= TRUST_STEPUP_THRESHOLD {
        TrustDecision::StepUp
    } else {
        TrustDecision::Fail
    }
}

pub fn is_headless(t: &TrustInputs) -> bool {
    if t.webdriver {
        return true;
    }
    let human_activity = t.focus_events > 0
        || t.blur_events > 0
        || t.scroll_events > 0
        || t.key_events > 0
        || t.move_events >= MIN_HUMAN_MOVES
        || t.has_touch;
    !human_activity
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fresh() -> ClientProfile {
        ClientProfile::new(Instant::now())
    }
    fn clean_inputs() -> TrustInputs {
        TrustInputs {
            trail_weight: 0.0,
            timing_weight: 0.0,
            grade_score: 1.0,
            suspicion: 0.0,
            fail_ratio: 0.0,
            page_load_to_first_move_ms: Some(600.0),
            focus_events: 2,
            blur_events: 1,
            scroll_events: 2,
            key_events: 1,
            move_events: 40,
            has_touch: false,
            max_pressure: 0.4,
            webdriver: false,
            input_type: "mouse".into(),
        }
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
        assert!(grade_trail(&trail, false).is_err());
    }
    #[test]
    fn trail_accepts_human_drag() {
        let mut trail = vec![];
        for i in 0..20 {
            let f = i as f64;
            trail.push((f * 5.0, f * 3.0, 150.0 + f * 40.0));
        }
        assert!(grade_trail(&trail, false).is_ok());
    }
    #[test]
    fn trail_rejects_empty() {
        assert!(grade_trail(&[], false).is_err());
    }
    #[test]
    fn trail_accepts_short_touch_drag() {
        let trail = vec![
            (10.0, 10.0, 0.0),
            (14.0, 13.0, 20.0),
            (19.0, 17.0, 45.0),
        ];
        assert!(grade_trail(&trail, false).is_err());
        assert!(grade_trail(&trail, true).is_ok());
    }
    #[test]
    fn clean_human_passes_trust() {
        let s = trust_score(&clean_inputs());
        assert!(s >= TRUST_PASS_THRESHOLD, "score was {s}");
        assert_eq!(trust_decision(s), TrustDecision::Pass);
    }
    #[test]
    fn webdriver_lowers_trust() {
        let mut t = clean_inputs();
        t.webdriver = true;
        let s = trust_score(&t);
        assert!(s < trust_score(&clean_inputs()));
        assert!(is_headless(&t));
    }
    #[test]
    fn headless_no_events_detected() {
        let mut t = clean_inputs();
        t.focus_events = 0;
        t.blur_events = 0;
        t.scroll_events = 0;
        t.key_events = 0;
        t.move_events = 0;
        t.has_touch = false;
        assert!(is_headless(&t));
    }
    #[test]
    fn pointer_movement_is_human() {
        let mut t = clean_inputs();
        t.focus_events = 0;
        t.blur_events = 0;
        t.scroll_events = 0;
        t.key_events = 0;
        t.move_events = MIN_HUMAN_MOVES;
        assert!(!is_headless(&t));
    }
    #[test]
    fn trust_score_in_range() {
        let mut t = clean_inputs();
        t.suspicion = 1.0;
        t.fail_ratio = 1.0;
        t.trail_weight = 1.0;
        t.timing_weight = 1.0;
        t.grade_score = 0.0;
        t.webdriver = true;
        let s = trust_score(&t);
        assert!((0.0..=1.0).contains(&s));
    }
    #[test]
    fn pursuit_accepts_tracking_trail() {
        let clicks = vec![(100.0, 100.0, 500.0)];
        let mut trail = vec![];
        for i in 0..10 {
            let f = i as f64 / 9.0;
            trail.push((95.0 + 10.0 * f, 95.0 + 10.0 * f, 200.0 + f * 300.0));
        }
        assert!(pursuit_coherent(&trail, &clicks).is_ok());
    }
    #[test]
    fn pursuit_rejects_click_far_from_cursor() {
        let clicks = vec![(300.0, 200.0, 500.0)];
        let trail = vec![
            (10.0, 10.0, 480.0),
            (10.0, 11.0, 488.0),
            (9.0, 10.0, 494.0),
            (10.0, 10.0, 499.0),
        ];
        assert!(pursuit_coherent(&trail, &clicks).is_err());
    }
    #[test]
    fn pursuit_rejects_teleport_before_click() {
        let clicks = vec![(200.0, 150.0, 500.0)];
        let trail = vec![
            (10.0, 10.0, 460.0),
            (11.0, 10.0, 470.0),
            (12.0, 11.0, 480.0),
            (200.0, 150.0, 499.0),
        ];
        assert!(pursuit_coherent(&trail, &clicks).is_err());
    }
}