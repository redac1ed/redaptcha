pub mod one;
pub mod two;
pub mod three;

use crate::captcha::Captcha;

pub const DEFAULT_KIND: &str = "one";

pub fn by_kind(kind: &str) -> Option<Box<dyn Captcha + Send + Sync>> {
    match kind {
        "one" => Some(Box::new(one::MovingBall)),
        "two" => Some(Box::new(two::Slider)),
        "three" => Some(Box::new(three::Passive)),
        _ => None,
    }
}