use oxc_ast::ast::{
    Declaration, Expression, Statement, VariableDeclarationKind,
};

use super::schema::{CalleePattern, ExpressionPattern, MatchPattern, StringPattern};

/// Check if a statement matches a declarative pattern.
pub fn matches_statement(stmt: &Statement<'_>, pattern: &MatchPattern) -> bool {
    match pattern {
        MatchPattern::ExpressionStatement { expression } => {
            if let Statement::ExpressionStatement(expr_stmt) = stmt {
                matches_expression(&expr_stmt.expression, expression)
            } else {
                false
            }
        }
        MatchPattern::VarDecl { kind, init } => {
            let decl = match stmt {
                Statement::VariableDeclaration(d) => d,
                Statement::ExportNamedDeclaration(export) => {
                    match export.declaration.as_ref() {
                        Some(Declaration::VariableDeclaration(d)) => d,
                        _ => return false,
                    }
                }
                _ => return false,
            };

            // Check kind
            if let Some(expected_kind) = kind {
                let actual_kind = match decl.kind {
                    VariableDeclarationKind::Const => "const",
                    VariableDeclarationKind::Let => "let",
                    VariableDeclarationKind::Var => "var",
                    _ => "using",
                };
                if actual_kind != expected_kind {
                    return false;
                }
            }

            // Check init pattern
            if let Some(init_pattern) = init {
                decl.declarations.iter().any(|d| {
                    d.init
                        .as_ref()
                        .is_some_and(|expr| matches_expression(expr, init_pattern))
                })
            } else {
                true
            }
        }
        MatchPattern::Import { source } => {
            if let Statement::ImportDeclaration(import) = stmt {
                if let Some(source_pattern) = source {
                    matches_string(&import.source.value, source_pattern)
                } else {
                    true
                }
            } else {
                false
            }
        }
    }
}

fn matches_expression(expr: &Expression<'_>, pattern: &ExpressionPattern) -> bool {
    match pattern {
        ExpressionPattern::Call { callee } => {
            if let Expression::CallExpression(call) = expr {
                matches_callee(&call.callee, callee)
            } else {
                false
            }
        }
        ExpressionPattern::New { callee } => {
            if let Expression::NewExpression(new_expr) = expr {
                matches_callee(&new_expr.callee, callee)
            } else {
                false
            }
        }
        ExpressionPattern::Identifier { name } => {
            if let Expression::Identifier(id) = expr {
                id.name.as_str() == name
            } else {
                false
            }
        }
    }
}

fn matches_callee(expr: &Expression<'_>, pattern: &CalleePattern) -> bool {
    match pattern {
        CalleePattern::Identifier { name } => {
            if let Expression::Identifier(id) = expr {
                id.name.as_str() == name
            } else {
                false
            }
        }
        CalleePattern::Member { object, property } => {
            if let Expression::StaticMemberExpression(member) = expr {
                if member.property.name.as_str() != property {
                    return false;
                }
                if let Expression::Identifier(obj_id) = &member.object {
                    obj_id.name.as_str() == object
                } else {
                    false
                }
            } else {
                false
            }
        }
    }
}

fn matches_string(value: &str, pattern: &StringPattern) -> bool {
    match pattern {
        StringPattern::Exact(expected) => value == expected,
        StringPattern::Pattern { pattern } => {
            // Simple glob matching: * matches anything
            let regex = pattern.replace('.', "\\.").replace('*', ".*");
            regex::Regex::new(&format!("^{regex}$"))
                .map(|r| r.is_match(value))
                .unwrap_or(false)
        }
    }
}
