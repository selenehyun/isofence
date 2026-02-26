use oxc_ast::ast::{
    Class, ClassElement, Declaration, Statement,
};

use crate::engine::context::ModuleContext;
use crate::engine::parser::{is_primitive_literal, is_safe_const_init, is_undefined};
use crate::rule::{Diagnostic, HazardCategory, Rule, RuleMeta, Severity};

/// Detects classes with static fields that have mutable initializers at module scope.
pub struct StaticClassField;

impl Rule for StaticClassField {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            name: "static-class-field",
            description: "Static class field with mutable initializer shares state globally",
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

        let class = match stmt {
            Statement::ClassDeclaration(class) => class,
            Statement::ExportNamedDeclaration(export) => {
                match export.declaration.as_ref() {
                    Some(Declaration::ClassDeclaration(class)) => class,
                    _ => return vec![],
                }
            }
            Statement::ExportDefaultDeclaration(export) => {
                match &export.declaration {
                    oxc_ast::ast::ExportDefaultDeclarationKind::ClassDeclaration(class) => class,
                    _ => return vec![],
                }
            }
            _ => return vec![],
        };

        check_class_static_fields(class, ctx)
    }
}

fn check_class_static_fields(class: &Class<'_>, ctx: &ModuleContext) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let class_name = class
        .id
        .as_ref()
        .map(|id| id.name.as_str())
        .unwrap_or("(anonymous)");

    for element in &class.body.body {
        if let ClassElement::PropertyDefinition(prop) = element {
            if !prop.r#static {
                continue;
            }

            // Check if the initializer is mutable
            if let Some(init) = &prop.value {
                if is_safe_const_init(init) || is_primitive_literal(init) || is_undefined(init) {
                    continue;
                }

                let field_name = prop
                    .key
                    .static_name()
                    .unwrap_or("(computed)".into());

                diagnostics.push(Diagnostic {
                    rule_name: "static-class-field".to_string(),
                    severity: Severity::Error,
                    message: format!(
                        "`{class_name}.{field_name}` — static field with mutable initializer"
                    ),
                    file_path: ctx.file_path.clone(),
                    span: prop.span,
                    help: Some(
                        "Static class fields with mutable values are shared across all instances and tests. Mock this module."
                            .to_string(),
                    ),
                    fix: None,
                });
            }
        }
    }

    diagnostics
}
