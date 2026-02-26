pub mod matchers;
pub mod schema;

use std::path::Path;

use oxc_ast::ast::Statement;
use oxc_span::GetSpan;

use crate::engine::context::ModuleContext;
use crate::rule::{Diagnostic, HazardCategory, Rule, RuleMeta, Severity};
use matchers::matches_statement;
use schema::{DeclarativeRuleFile, DeclarativeRuleSchema};

/// A rule compiled from a JSON declarative rule file.
pub struct DeclarativeRule {
    pub name: String,
    pub description: String,
    pub severity: Severity,
    pub pattern: schema::MatchPattern,
    pub message: String,
}

impl DeclarativeRule {
    /// Load rules from a JSON file.
    pub fn load_from_file(path: &Path) -> Result<Vec<DeclarativeRule>, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read rule file {}: {}", path.display(), e))?;
        Self::load_from_str(&content)
    }

    /// Load rules from a JSON string.
    pub fn load_from_str(content: &str) -> Result<Vec<DeclarativeRule>, String> {
        let file: DeclarativeRuleFile =
            serde_json::from_str(content).map_err(|e| format!("Invalid rule JSON: {e}"))?;

        file.rules.into_iter().map(Self::from_schema).collect()
    }

    fn from_schema(schema: DeclarativeRuleSchema) -> Result<DeclarativeRule, String> {
        let severity = match schema.severity.as_str() {
            "error" => Severity::Error,
            "warning" => Severity::Warning,
            "off" => Severity::Off,
            other => return Err(format!("Invalid severity '{}' in rule '{}'", other, schema.name)),
        };

        Ok(DeclarativeRule {
            name: schema.name,
            description: schema.description,
            severity,
            pattern: schema.r#match,
            message: schema.message,
        })
    }
}

impl Rule for DeclarativeRule {
    fn meta(&self) -> RuleMeta {
        // We leak the strings to get 'static lifetimes — these live for the program duration
        RuleMeta {
            name: Box::leak(self.name.clone().into_boxed_str()),
            description: Box::leak(self.description.clone().into_boxed_str()),
            category: HazardCategory::SideEffect,
            default_severity: self.severity,
        }
    }

    fn check_module_item(
        &self,
        stmt: &Statement<'_>,
        ctx: &ModuleContext,
    ) -> Vec<Diagnostic> {
        if ctx.is_test_file {
            return vec![];
        }

        if matches_statement(stmt, &self.pattern) {
            vec![Diagnostic {
                rule_name: self.name.clone(),
                severity: self.severity,
                message: self.message.clone(),
                file_path: ctx.file_path.clone(),
                span: stmt.span(),
                help: None,
                fix: None,
                import_chain: None,
                hazard_sources: vec![],
            }]
        } else {
            vec![]
        }
    }
}
