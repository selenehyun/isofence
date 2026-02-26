use oxc_ast::ast::{Declaration, Expression, Statement, VariableDeclarationKind};

use crate::engine::context::ModuleContext;
use crate::rule::{Diagnostic, HazardCategory, Rule, RuleMeta, Severity};

/// Detects top-level await at module scope.
pub struct TopLevelAwait;

impl Rule for TopLevelAwait {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            name: "top-level-await",
            description: "Top-level await at module scope causes side effects on import",
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

        // Check for: await expr; (expression statement)
        if let Statement::ExpressionStatement(expr_stmt) = stmt {
            if contains_await(&expr_stmt.expression) {
                return vec![Diagnostic {
                    rule_name: "top-level-await".to_string(),
                    severity: Severity::Error,
                    message: "Top-level `await` — async side effect on import".to_string(),
                    file_path: ctx.file_path.clone(),
                    span: expr_stmt.span,
                    help: Some(
                        "Top-level await blocks module loading and causes side effects. Mock this module in tests."
                            .to_string(),
                    ),
                    fix: None,
                    import_chain: None,
                    hazard_sources: vec![],
                    hazardous_imports: vec![],
                }];
            }
        }

        // Check for: const x = await expr; or let x = await expr;
        let decl = match stmt {
            Statement::VariableDeclaration(decl) => decl,
            Statement::ExportNamedDeclaration(export) => {
                match export.declaration.as_ref() {
                    Some(Declaration::VariableDeclaration(decl)) => decl,
                    _ => return vec![],
                }
            }
            _ => return vec![],
        };

        let mut diagnostics = Vec::new();
        for declarator in &decl.declarations {
            if let Some(init) = &declarator.init {
                if contains_await(init) {
                    let name = declarator.id.get_identifier_name()
                        .map(|a| a.to_string())
                        .unwrap_or_else(|| "(destructured)".to_string());
                    diagnostics.push(Diagnostic {
                        rule_name: "top-level-await".to_string(),
                        severity: Severity::Error,
                        message: format!(
                            "`{} {name} = await ...` — async side effect on import",
                            match decl.kind {
                                VariableDeclarationKind::Const => "const",
                                VariableDeclarationKind::Let => "let",
                                VariableDeclarationKind::Var => "var",
                                _ => "using",
                            }
                        ),
                        file_path: ctx.file_path.clone(),
                        span: declarator.span,
                        help: Some(
                            "Top-level await causes async side effects on import. Mock this module."
                                .to_string(),
                        ),
                        fix: None,
                        import_chain: None,
                        hazard_sources: vec![],
                    hazardous_imports: vec![],
                    });
                }
            }
        }

        diagnostics
    }
}

fn contains_await(expr: &Expression<'_>) -> bool {
    matches!(expr, Expression::AwaitExpression(_))
}
