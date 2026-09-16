#[allow(dead_code)]
pub const SIGNAL_KEY: &str = "liquor_licence_ended";

pub use super::liquor::{
    detect_liquor_latest, detect_liquor_pair, print_liquor_diff, LiquorComparability, LiquorDiff,
};
