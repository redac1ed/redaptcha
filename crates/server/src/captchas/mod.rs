pub mod one;

use crate::captcha::Captcha;

pub const DEFAULT_KIND: &str = "one";

pub fn by_kind(kind: &str) -> Option<Box<dyn Captcha + Send + Sync>> {
    match kind {
        "one" => Some(Box::new(one::MovingBall)),
        _ => None,
    }
}