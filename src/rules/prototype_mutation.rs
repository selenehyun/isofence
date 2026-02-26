use oxc_ast::ast::{AssignmentTarget, Expression, Statement};

use crate::engine::context::ModuleContext;
use crate::rule::{Diagnostic, HazardCategory, Rule, RuleMeta, Severity};

/// Detects prototype mutation at module scope.
pub struct PrototypeMutation;

impl Rule for PrototypeMutation {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            name: "prototype-mutation",
            description: "Prototype mutation at module scope affects all instances globally",
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

        if is_prototype_assignment(&assign.left) {
            let target = describe_assignment_target(&assign.left);
            return vec![Diagnostic {
                rule_name: "prototype-mutation".to_string(),
                severity: Severity::Error,
                message: format!(
                    "Assignment to `{target}` — prototype mutation at module scope"
                ),
                file_path: ctx.file_path.clone(),
                span: expr_stmt.span,
                help: Some(
                    "Prototype mutations persist globally and affect all tests. Mock this module."
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

fn is_prototype_assignment(target: &AssignmentTarget<'_>) -> bool {
    match target {
        AssignmentTarget::StaticMemberExpression(member) => {
            // Check if the chain contains ".prototype."
            has_prototype_in_chain(&member.object) || member.property.name.as_str() == "prototype"
        }
        AssignmentTarget::ComputedMemberExpression(member) => {
            has_prototype_in_chain(&member.object)
        }
        _ => false,
    }
}

fn has_prototype_in_chain(expr: &Expression<'_>) -> bool {
    match expr {
        Expression::StaticMemberExpression(member) => {
            member.property.name.as_str() == "prototype"
                || has_prototype_in_chain(&member.object)
        }
        _ => false,
    }
}

fn describe_assignment_target(target: &AssignmentTarget<'_>) -> String {
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
