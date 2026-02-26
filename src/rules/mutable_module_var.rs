use oxc_ast::ast::{Declaration, Statement, VariableDeclarationKind};

use crate::engine::context::ModuleContext;
use crate::rule::{Diagnostic, HazardCategory, Rule, RuleMeta, Severity};

/// Detects `let` and `var` declarations at module scope.
pub struct MutableModuleVar;

impl Rule for MutableModuleVar {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            name: "mutable-module-var",
            description: "Mutable module-scope variable (let/var) can leak state between tests",
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
            _ => return vec![],
        };

        match decl.kind {
            VariableDeclarationKind::Let | VariableDeclarationKind::Var => {}
            _ => return vec![],
        }

        let kind_str = match decl.kind {
            VariableDeclarationKind::Let => "let",
            VariableDeclarationKind::Var => "var",
            _ => unreachable!(),
        };

        decl.declarations
            .iter()
            .map(|declarator| {
                let name = declarator
                    .id
                    .get_identifier_name()
                    .map(|a| a.to_string())
                    .unwrap_or_else(|| "(destructured)".to_string());
                Diagnostic {
                    rule_name: "mutable-module-var".to_string(),
                    severity: Severity::Error,
                    message: format!(
                        "`{kind_str} {name}` at module scope — mutable binding can leak state between tests"
                    ),
                    file_path: ctx.file_path.clone(),
                    span: declarator.span,
                    help: Some("Consider using `const` with an immutable value, or mock this module in tests".to_string()),
                    fix: None,
                    import_chain: None,
                    hazard_sources: vec![],
                    hazardous_imports: vec![],
                }
            })
            .collect()
    }
}
