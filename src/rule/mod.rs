use oxc_ast::ast::Statement;
use oxc_span::Span;
use std::fmt;
use std::path::PathBuf;

use crate::engine::context::{GraphContext, ModuleContext};

/// Severity level for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Off,
    Warning,
    Error,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Severity::Off => write!(f, "off"),
            Severity::Warning => write!(f, "warning"),
            Severity::Error => write!(f, "error"),
        }
    }
}

/// Confidence level for hazard detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Confidence {
    /// Definitely hazardous (e.g., `let` at module scope).
    Definite,
    /// Potentially hazardous (e.g., TypeScript enum — conventionally immutable).
    Potential,
}

/// Category of hazard.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HazardCategory {
    MutableState,
    SideEffect,
}

/// A detected hazard in a module.
#[derive(Debug, Clone)]
pub struct Hazard {
    pub rule_name: String,
    pub category: HazardCategory,
    pub confidence: Confidence,
    pub span: Span,
    pub message: String,
}

/// A diagnostic produced by a rule.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub rule_name: String,
    pub severity: Severity,
    pub message: String,
    pub file_path: PathBuf,
    pub span: Span,
    pub help: Option<String>,
    pub fix: Option<Fix>,
    /// Import chain from test file to hazardous module (hazard-reachability only).
    pub import_chain: Option<Vec<PathBuf>>,
    /// Source locations of hazards in the referenced module (hazard-reachability only).
    pub hazard_sources: Vec<HazardSource>,
}

/// Source location of a hazard in a referenced module (for hazard-reachability).
#[derive(Debug, Clone)]
pub struct HazardSource {
    pub file_path: PathBuf,
    pub span: Span,
    pub message: String,
}

/// An auto-fix action.
#[derive(Debug, Clone)]
pub struct Fix {
    /// Text to insert.
    pub text: String,
    /// Span where the fix should be applied. For insertions, start == end.
    pub span: Span,
}

/// Metadata about a rule.
#[derive(Debug, Clone)]
pub struct RuleMeta {
    pub name: &'static str,
    pub description: &'static str,
    pub category: HazardCategory,
    pub default_severity: Severity,
}

/// The core Rule trait with 3 execution hooks.
pub trait Rule: Send + Sync {
    /// Return metadata about this rule.
    fn meta(&self) -> RuleMeta;

    /// Phase 1: Called for each top-level statement in the module.
    fn check_module_item(
        &self,
        _stmt: &Statement<'_>,
        _ctx: &ModuleContext,
    ) -> Vec<Diagnostic> {
        vec![]
    }

    /// Phase 2: Called once after all statements are processed.
    fn check_module(&self, _ctx: &ModuleContext) -> Vec<Diagnostic> {
        vec![]
    }

    /// Phase 3: Called once after the full project graph is built.
    fn check_graph(&self, _ctx: &GraphContext) -> Vec<Diagnostic> {
        vec![]
    }
}

pub mod registry;
pub mod declarative;
