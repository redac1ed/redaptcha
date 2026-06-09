use core_types::{Click, PanelMotion};

pub struct Rendered {
    pub frames_b64: Vec<String>,
    pub motions: Vec<PanelMotion>,
}

pub trait Captcha {
    fn kind(&self) -> &'static str;
    fn expected_clicks(&self) -> usize;
    fn puzzle_w(&self) -> f64;
    fn puzzle_h(&self) -> f64;
    fn generate(&self, challenge_key: &[u8], challenge_id: &str) -> Rendered;
    fn validate(&self, clicks: &[Click]) -> Result<(), &'static str>;
    fn grade(&self, challenge_key: &[u8], challenge_id: &str, clicks: &[Click]) -> Result<(), &'static str>;
}