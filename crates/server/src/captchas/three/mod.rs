use core_types::{Click, TrailPoint};

use crate::captcha::{Captcha, Rendered};

pub struct Passive;

impl Captcha for Passive {
    fn kind(&self) -> &'static str { "three" }
    fn expected_clicks(&self) -> usize { 0 }
    fn rounds(&self) -> u32 { 1 }
    fn puzzle_w(&self) -> f64 { 0.0 }
    fn puzzle_h(&self) -> f64 { 0.0 }
    fn generate(&self, _challenge_key: &[u8], _challenge_id: &str) -> Rendered {
        Rendered { frames_b64: Vec::new(), slider: None }
    }
    fn grade(&self, _challenge_key: &[u8], _challenge_id: &str, clicks: &[Click], _trail: &[TrailPoint]) -> Result<(), &'static str> {
        if clicks.is_empty() {
            Ok(())
        } else {
            Err("unexpected interaction")
        }
    }
    fn track(&self, _challenge_key: &[u8], _challenge_id: &str, clicks: &[Click], _trail: &[TrailPoint]) -> Result<(), &'static str> {
        if clicks.is_empty() { Ok(()) } else { Err("unexpected interaction") }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn passive_grades_empty_clicks() {
        let p = Passive;
        assert!(p.grade(b"k", "id", &[], &[]).is_ok());
    }
    #[test]
    fn passive_rejects_clicks() {
        let p = Passive;
        let clicks = vec![Click { x: 1.0, y: 2.0, t: 3.0 }];
        assert!(p.grade(b"k", "id", &clicks, &[]).is_err());
    }
    #[test]
    fn passive_single_round() {
        assert_eq!(Passive.rounds(), 1);
    }
    #[test]
    fn passive_renders_nothing() {
        let r = Passive.generate(b"k", "id");
        assert!(r.frames_b64.is_empty());
        assert!(r.slider.is_none());
    }
}