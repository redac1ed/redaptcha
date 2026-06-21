use core_types::{Click, SliderHint, TrailPoint};

pub struct Rendered {
    pub frames_b64: Vec<String>,
    pub slider: Option<SliderHint>,
}

pub trait Captcha {
    #[allow(dead_code)]
    fn kind(&self) -> &'static str;
    #[allow(dead_code)]
    fn expected_clicks(&self) -> usize;
    fn rounds(&self) -> u32 { 1 }
    fn puzzle_w(&self) -> f64;
    fn puzzle_h(&self) -> f64;
    fn generate(&self, challenge_key: &[u8], challenge_id: &str) -> Rendered;
    fn grade(&self, challenge_key: &[u8], challenge_id: &str, clicks: &[Click], trail: &[TrailPoint]) -> Result<(), &'static str>;
    fn track(
        &self,
        _challenge_key: &[u8],
        _challenge_id: &str,
        _clicks: &[Click],
        _trail: &[TrailPoint],
    ) -> Result<(), &'static str> {
        Ok(())
    }
}