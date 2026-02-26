use oxc_ast::ast::{Declaration, Expression, Statement, VariableDeclarationKind};

use crate::engine::context::ModuleContext;
use crate::engine::parser::{
    is_collection_constructor, is_mutable_const_init,
    is_safe_const_init, is_stateful_regex,
};
use crate::rule::{Diagnostic, HazardCategory, Rule, RuleMeta, Severity};

/// Detects `const` declarations with mutable initializers at module scope.
pub struct MutableConstInit;

impl Rule for MutableConstInit {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            name: "mutable-const-init",
            description: "const with mutable initializer (object/array/new/call) can leak state",
            category: HazardCategory::MutableState,
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

        let decl = match stmt {
            Statement::VariableDeclaration(decl) => decl,
            Statement::ExportNamedDeclaration(export) => {
                match export.declaration.as_ref() {
                    Some(Declaration::VariableDeclaration(decl)) => decl,
                    _ => return vec![],
                }
            }
            Statement::ExportDefaultDeclaration(export) => {
                // export default <expression> — check separately
                return self.check_export_default_expr(&export.declaration, ctx);
            }
            _ => return vec![],
        };

        if decl.kind != VariableDeclarationKind::Const {
            return vec![];
        }

        let mut diagnostics = Vec::new();

        for declarator in &decl.declarations {
            let init = match &declarator.init {
                Some(init) => init,
                None => continue,
            };

            // Skip safe initializers
            if is_safe_const_init(init) {
                continue;
            }

            let name = declarator
                .id
                .get_identifier_name()
                .map(|a| a.to_string())
                .unwrap_or_else(|| "(destructured)".to_string());

            if is_stateful_regex(init) {
                diagnostics.push(Diagnostic {
                    rule_name: "mutable-const-init".to_string(),
                    severity: Severity::Error,
                    message: format!(
                        "`const {name}` — RegExp with /g or /y flag has mutable lastIndex"
                    ),
                    file_path: ctx.file_path.clone(),
                    span: declarator.span,
                    help: Some("RegExp with global/sticky flags mutates lastIndex on each exec(). Mock this module or remove the flag.".to_string()),
                    fix: None,
                });
                continue;
            }

            if is_collection_constructor(init) {
                diagnostics.push(Diagnostic {
                    rule_name: "mutable-const-init".to_string(),
                    severity: Severity::Error,
                    message: format!(
                        "`const {name}` — collection constructor (Map/Set/etc.) is mutable"
                    ),
                    file_path: ctx.file_path.clone(),
                    span: declarator.span,
                    help: Some("Collections are mutable via .set()/.add(). Mock this module in tests.".to_string()),
                    fix: None,
                });
                continue;
            }

            if is_mutable_const_init(init) {
                let kind = describe_mutable_init(init);
                // Call/New expressions that survived is_safe_const_init are likely
                // still safe (factory returns, wrappers) — downgrade to Warning.
                let severity = match init {
                    Expression::ObjectExpression(_) | Expression::ArrayExpression(_) => {
                        Severity::Error
                    }
                    Expression::CallExpression(_) | Expression::NewExpression(_) => {
                        Severity::Warning
                    }
                    _ => Severity::Error,
                };
                diagnostics.push(Diagnostic {
                    rule_name: "mutable-const-init".to_string(),
                    severity,
                    message: format!(
                        "`const {name}` — {kind} initializer is mutable despite const binding"
                    ),
                    file_path: ctx.file_path.clone(),
                    span: declarator.span,
                    help: Some("const only prevents reassignment, not mutation. Use Object.freeze(), `as const`, or mock this module.".to_string()),
                    fix: None,
                });
            }
        }

        diagnostics
    }
}

impl MutableConstInit {
    fn check_export_default_expr(
        &self,
        decl: &oxc_ast::ast::ExportDefaultDeclarationKind<'_>,
        ctx: &ModuleContext,
    ) -> Vec<Diagnostic> {
        // Only check expression exports, not function/class declarations
        let expr = match decl.as_expression() {
            Some(expr) => expr,
            None => return vec![],
        };

        if is_mutable_const_init(expr) {
            return vec![Diagnostic {
                rule_name: "mutable-const-init".to_string(),
                severity: Severity::Error,
                message: "export default — mutable value exported at module scope".to_string(),
                file_path: ctx.file_path.clone(),
                span: oxc_span::Span::default(),
                help: Some("Default export of mutable value can leak state. Use Object.freeze() or mock this module.".to_string()),
                fix: None,
            }];
        }

        vec![]
    }
}

fn describe_mutable_init(expr: &Expression<'_>) -> &'static str {
    match expr {
        Expression::ObjectExpression(_) => "object literal",
        Expression::ArrayExpression(_) => "array literal",
        Expression::NewExpression(_) => "constructor (new)",
        Expression::CallExpression(_) => "function call result",
        _ => "mutable value",
    }
}
