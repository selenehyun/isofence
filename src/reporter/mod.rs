pub mod console;
pub mod json;

use crate::engine::EngineResult;

/// Reporter trait for outputting results.
pub trait Reporter {
    fn report(&self, result: &EngineResult);
}
