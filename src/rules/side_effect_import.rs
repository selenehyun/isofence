use crate::engine::context::ModuleContext;
use crate::rule::{Diagnostic, HazardCategory, Rule, RuleMeta, Severity};

/// Detects side-effect imports (`import './setup'` with no specifiers).
pub struct SideEffectImport;

impl Rule for SideEffectImport {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            name: "side-effect-import",
            description: "Side-effect import executes module code on import",
            category: HazardCategory::SideEffect,
            default_severity: Severity::Warning,
        }
    }

    fn check_module(
        &self,
        ctx: &ModuleContext,
    ) -> Vec<Diagnostic> {
        if ctx.is_test_file {
            return vec![];
        }

        ctx.imports
            .iter()
            .filter(|import| import.is_side_effect && !import.is_type_only)
            .map(|import| Diagnostic {
                rule_name: "side-effect-import".to_string(),
                severity: Severity::Warning,
                message: format!(
                    "`import '{}'` — side-effect import executes module on load",
                    import.source
                ),
                file_path: ctx.file_path.clone(),
                span: import.span,
                help: Some(
                    "Side-effect imports execute code when the importing module is loaded. Ensure the imported module is mocked or safe."
                        .to_string(),
                ),
                fix: None,
            })
            .collect()
    }
}
