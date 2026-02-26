use oxc_ast::ast::{Expression, Statement};

use crate::engine::context::ModuleContext;
use crate::rule::{Diagnostic, HazardCategory, Rule, RuleMeta, Severity};

/// Detects top-level function calls at module scope (side effects on import).
pub struct TopLevelCall;

impl Rule for TopLevelCall {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            name: "top-level-call",
            description: "Top-level function call executes on import (side effect)",
            category: HazardCategory::SideEffect,
            default_severity: Severity::Error,
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

        let expr_stmt = match stmt {
            Statement::ExpressionStatement(expr_stmt) => expr_stmt,
            _ => return vec![],
        };

        match &expr_stmt.expression {
            Expression::CallExpression(call) => {
                // Get a description of what's being called
                let callee_name = describe_callee(&call.callee);
                vec![Diagnostic {
                    rule_name: "top-level-call".to_string(),
                    severity: Severity::Error,
                    message: format!(
                        "Top-level call `{callee_name}` executes on import — side effect"
                    ),
                    file_path: ctx.file_path.clone(),
                    span: expr_stmt.span,
                    help: Some(
                        "Move this call inside a function, or mock this module in tests."
                            .to_string(),
                    ),
                    fix: None,
                    import_chain: None,
                    hazard_sources: vec![],
                    hazardous_imports: vec![],
                }]
            }
            _ => vec![],
        }
    }
}

fn describe_callee(expr: &Expression<'_>) -> String {
    match expr {
        Expression::Identifier(id) => id.name.to_string(),
        Expression::StaticMemberExpression(member) => {
            let obj = describe_callee(&member.object);
            format!("{obj}.{}", member.property.name)
        }
        Expression::CallExpression(call) => {
            let inner = describe_callee(&call.callee);
            format!("{inner}(...)")
        }
        _ => "(expression)".to_string(),
    }
}
