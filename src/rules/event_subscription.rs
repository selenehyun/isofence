use oxc_ast::ast::{Expression, Statement};

use crate::engine::context::ModuleContext;
use crate::rule::{Diagnostic, HazardCategory, Rule, RuleMeta, Severity};

/// Detects event subscriptions at module scope (.on(), .addEventListener(), .subscribe()).
pub struct EventSubscription;

impl Rule for EventSubscription {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            name: "event-subscription",
            description: "Event subscription at module scope registers listener on import",
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

        if let Expression::CallExpression(call) = &expr_stmt.expression {
            if let Expression::StaticMemberExpression(member) = &call.callee {
                let method = member.property.name.as_str();
                if matches!(method, "on" | "addEventListener" | "subscribe" | "addListener") {
                    let obj_name = describe_object(&member.object);
                    return vec![Diagnostic {
                        rule_name: "event-subscription".to_string(),
                        severity: Severity::Error,
                        message: format!(
                            "`{obj_name}.{method}(...)` — event subscription at module scope"
                        ),
                        file_path: ctx.file_path.clone(),
                        span: expr_stmt.span,
                        help: Some(
                            "Event listeners registered on import persist across tests. Mock this module or move subscription inside a function."
                                .to_string(),
                        ),
                        fix: None,
                    }];
                }
            }
        }

        vec![]
    }
}

fn describe_object(expr: &Expression<'_>) -> String {
    match expr {
        Expression::Identifier(id) => id.name.to_string(),
        Expression::StaticMemberExpression(member) => {
            let obj = describe_object(&member.object);
            format!("{obj}.{}", member.property.name)
        }
        _ => "(object)".to_string(),
    }
}
