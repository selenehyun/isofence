use oxc_ast::ast::{Expression, Statement};

use crate::engine::context::ModuleContext;
use crate::rule::{Diagnostic, HazardCategory, Rule, RuleMeta, Severity};

/// Detects IIFEs (Immediately Invoked Function Expressions) at module scope.
pub struct Iife;

impl Rule for Iife {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            name: "iife",
            description: "IIFE at module scope executes on import",
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

        if is_iife(&expr_stmt.expression) {
            return vec![Diagnostic {
                rule_name: "iife".to_string(),
                severity: Severity::Error,
                message: "IIFE at module scope — executes on import".to_string(),
                file_path: ctx.file_path.clone(),
                span: expr_stmt.span,
                help: Some(
                    "IIFEs execute immediately when the module is imported. Mock this module in tests."
                        .to_string(),
                ),
                fix: None,
                import_chain: None,
                hazard_sources: vec![],
                hazardous_imports: vec![],
            }];
        }

        vec![]
    }
}

fn is_iife(expr: &Expression<'_>) -> bool {
    if let Expression::CallExpression(call) = expr {
        return match &call.callee {
            Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_) => true,
            Expression::ParenthesizedExpression(paren) => matches!(
                &paren.expression,
                Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_)
            ),
            _ => false,
        };
    }
    false
}
