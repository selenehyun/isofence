use oxc_ast::ast::{AssignmentTarget, Expression, Statement};

use crate::engine::context::ModuleContext;
use crate::rule::{Diagnostic, HazardCategory, Rule, RuleMeta, Severity};

/// Detects assignments to global objects (globalThis, process, window) at module scope.
pub struct GlobalMutation;

impl Rule for GlobalMutation {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            name: "global-mutation",
            description: "Global state mutation at module scope (globalThis/process/window)",
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

        let assign = match &expr_stmt.expression {
            Expression::AssignmentExpression(assign) => assign,
            _ => return vec![],
        };

        let target_name = match &assign.left {
            AssignmentTarget::StaticMemberExpression(member) => {
                if get_global_obj_name(&member.object).is_some() {
                    describe_member_target(&assign.left)
                } else {
                    return vec![];
                }
            }
            AssignmentTarget::ComputedMemberExpression(member) => {
                if get_global_obj_name(&member.object).is_some() {
                    describe_member_target(&assign.left)
                } else {
                    return vec![];
                }
            }
            _ => return vec![],
        };

        vec![Diagnostic {
            rule_name: "global-mutation".to_string(),
            severity: Severity::Error,
            message: format!(
                "Assignment to `{target_name}` — global state mutation on import"
            ),
            file_path: ctx.file_path.clone(),
            span: expr_stmt.span,
            help: Some(
                "Global mutations at module scope affect all tests. Mock this module or move mutation inside a function."
                    .to_string(),
            ),
            fix: None,
            import_chain: None,
            hazard_sources: vec![],
            hazardous_imports: vec![],
        }]
    }
}

fn describe_member_target(target: &AssignmentTarget<'_>) -> String {
    match target {
        AssignmentTarget::StaticMemberExpression(member) => {
            let obj = describe_expr(&member.object);
            format!("{obj}.{}", member.property.name)
        }
        AssignmentTarget::ComputedMemberExpression(member) => {
            let obj = describe_expr(&member.object);
            format!("{obj}[...]")
        }
        _ => "(target)".to_string(),
    }
}

fn describe_expr(expr: &Expression<'_>) -> String {
    match expr {
        Expression::Identifier(id) => id.name.to_string(),
        Expression::StaticMemberExpression(member) => {
            let obj = describe_expr(&member.object);
            format!("{obj}.{}", member.property.name)
        }
        _ => "(expr)".to_string(),
    }
}

fn get_global_obj_name<'a>(expr: &'a Expression<'a>) -> Option<&'a str> {
    match expr {
        Expression::Identifier(id) => {
            let name = id.name.as_str();
            if matches!(name, "globalThis" | "global" | "process" | "window" | "self" | "document") {
                Some(name)
            } else {
                None
            }
        }
        // Recurse through member expressions: process.env.X → find "process" at the root
        Expression::StaticMemberExpression(member) => get_global_obj_name(&member.object),
        Expression::ComputedMemberExpression(member) => get_global_obj_name(&member.object),
        _ => None,
    }
}
