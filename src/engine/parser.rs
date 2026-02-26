use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Argument, BindingPatternKind, CallExpression, Expression, Program, Statement,
};
use oxc_parser::{ParseOptions, Parser};
use oxc_span::SourceType;
use std::path::Path;

use crate::engine::context::{
    ImportInfo, MockDeclaration, MockKind, SafeSignal, TestFramework,
};

/// Parse a TypeScript/JavaScript source file and return the AST.
pub fn parse_source<'a>(
    allocator: &'a Allocator,
    source_text: &'a str,
    file_path: &Path,
) -> oxc_parser::ParserReturn<'a> {
    let source_type = SourceType::from_path(file_path).unwrap_or_default();
    Parser::new(allocator, source_text, source_type)
        .with_options(ParseOptions {
            allow_return_outside_function: true,
            ..Default::default()
        })
        .parse()
}

/// Extract import information from the AST.
pub fn extract_imports(program: &Program<'_>) -> Vec<ImportInfo> {
    let mut imports = Vec::new();

    for stmt in &program.body {
        match stmt {
            Statement::ImportDeclaration(decl) => {
                let source = decl.source.value.to_string();
                let is_type_only = decl.import_kind.is_type();
                let is_side_effect = decl.specifiers.as_ref().map_or(true, |s| s.is_empty());

                imports.push(ImportInfo {
                    source,
                    resolved_path: None,
                    is_type_only,
                    is_side_effect,
                    span: decl.span,
                });
            }
            Statement::ExportNamedDeclaration(decl) => {
                if let Some(ref src) = decl.source {
                    imports.push(ImportInfo {
                        source: src.value.to_string(),
                        resolved_path: None,
                        is_type_only: decl.export_kind.is_type(),
                        is_side_effect: false,
                        span: decl.span,
                    });
                }
            }
            Statement::ExportAllDeclaration(decl) => {
                imports.push(ImportInfo {
                    source: decl.source.value.to_string(),
                    resolved_path: None,
                    is_type_only: decl.export_kind.is_type(),
                    is_side_effect: false,
                    span: decl.span,
                });
            }
            _ => {}
        }
    }

    imports
}

/// Extract mock declarations from a test file's AST.
pub fn extract_mocks(program: &Program<'_>, framework: TestFramework) -> Vec<MockDeclaration> {
    let mut mocks = Vec::new();

    for stmt in &program.body {
        if let Some(expr) = get_expression_statement(stmt) {
            extract_mock_from_expr(expr, framework, &mut mocks);
        }
    }

    mocks
}

fn extract_mock_from_expr(
    expr: &Expression<'_>,
    framework: TestFramework,
    mocks: &mut Vec<MockDeclaration>,
) {
    let call = match expr {
        Expression::CallExpression(call) => call.as_ref(),
        _ => return,
    };

    let (obj_name, method_name) = match &call.callee {
        Expression::StaticMemberExpression(member) => {
            let obj = match &member.object {
                Expression::Identifier(id) => id.name.as_str(),
                _ => return,
            };
            let method = member.property.name.as_str();
            (obj, method)
        }
        _ => return,
    };

    // Check for vi.mock / jest.mock / vi.doMock / jest.doMock
    let is_vi = obj_name == "vi" && matches!(framework, TestFramework::Vitest | TestFramework::Unknown);
    let is_jest = obj_name == "jest" && matches!(framework, TestFramework::Jest | TestFramework::Unknown);

    if !(is_vi || is_jest) {
        return;
    }

    if method_name != "mock" && method_name != "doMock" {
        return;
    }

    // First argument should be the module path string
    let source = match call.arguments.first() {
        Some(Argument::StringLiteral(lit)) => lit.value.to_string(),
        Some(Argument::TemplateLiteral(tpl)) if tpl.expressions.is_empty() => {
            tpl.quasis.first().map(|q| q.value.raw.to_string()).unwrap_or_default()
        }
        _ => return,
    };

    // Determine mock kind: check if factory uses importOriginal
    let kind = if call.arguments.len() >= 2 {
        if is_partial_mock(call) {
            MockKind::Partial
        } else {
            MockKind::Full
        }
    } else {
        MockKind::Full
    };

    mocks.push(MockDeclaration {
        source,
        resolved_path: None,
        kind,
        span: call.span,
    });
}

/// Check if a mock call uses importOriginal (partial mock pattern).
fn is_partial_mock(call: &CallExpression<'_>) -> bool {
    // Look at the second argument — if it's an async arrow/function that calls importOriginal
    if let Some(arg) = call.arguments.get(1) {
        match arg {
            Argument::ArrowFunctionExpression(arrow) => {
                return arrow.params.items.iter().any(|p| {
                    matches!(&p.pattern.kind, BindingPatternKind::BindingIdentifier(id) if id.name.as_str() == "importOriginal")
                });
            }
            Argument::FunctionExpression(func) => {
                return func.params.items.iter().any(|p| {
                    matches!(&p.pattern.kind, BindingPatternKind::BindingIdentifier(id) if id.name.as_str() == "importOriginal")
                });
            }
            _ => return false,
        }
    }
    false
}

/// Extract safe signals from test file (beforeEach with resetModules, etc.).
pub fn extract_safe_signals(program: &Program<'_>) -> Vec<SafeSignal> {
    let mut signals = Vec::new();

    for stmt in &program.body {
        if let Some(expr) = get_expression_statement(stmt) {
            if let Expression::CallExpression(call) = expr {
                // Check for beforeEach(...)
                let is_before_each = match &call.callee {
                    Expression::Identifier(id) => id.name.as_str() == "beforeEach",
                    _ => false,
                };

                if is_before_each {
                    // Check callback body for safe signals
                    if let Some(arg) = call.arguments.first() {
                        check_callback_for_signals(arg, &mut signals);
                    }
                }
            }
        }
    }

    signals
}

fn check_callback_for_signals(arg: &Argument<'_>, signals: &mut Vec<SafeSignal>) {
    // Look for vi.resetModules(), vi.restoreAllMocks() in the callback body
    // This is simplified — we check the source text for known patterns
    let body_stmts: Vec<&Statement<'_>> = match arg {
        Argument::ArrowFunctionExpression(arrow) => {
            arrow.body.statements.iter().collect()
        }
        Argument::FunctionExpression(func) => {
            func.body.as_ref().map(|b| b.statements.iter().collect()).unwrap_or_default()
        }
        _ => return,
    };

    for stmt in body_stmts {
        if let Some(expr) = get_expression_statement(stmt) {
            if let Expression::CallExpression(call) = expr {
                if let Expression::StaticMemberExpression(member) = &call.callee {
                    let method = member.property.name.as_str();
                    match method {
                        "resetModules" => signals.push(SafeSignal::ResetModules),
                        "restoreAllMocks" => signals.push(SafeSignal::RestoreAllMocks),
                        "clear" | "reset" => signals.push(SafeSignal::ClearState),
                        _ => {}
                    }
                }
            }
        }
    }
}

/// Helper to get the expression from an ExpressionStatement.
fn get_expression_statement<'a>(stmt: &'a Statement<'a>) -> Option<&'a Expression<'a>> {
    match stmt {
        Statement::ExpressionStatement(expr_stmt) => Some(&expr_stmt.expression),
        _ => None,
    }
}

/// Check if an expression is a primitive literal.
pub fn is_primitive_literal(expr: &Expression<'_>) -> bool {
    matches!(
        expr,
        Expression::NumericLiteral(_)
            | Expression::StringLiteral(_)
            | Expression::BooleanLiteral(_)
            | Expression::NullLiteral(_)
            | Expression::TemplateLiteral(_)
    )
}

/// Check if an expression is `undefined` identifier.
pub fn is_undefined(expr: &Expression<'_>) -> bool {
    matches!(expr, Expression::Identifier(id) if id.name.as_str() == "undefined")
}

/// Check if an expression is `Object.freeze(...)`.
pub fn is_object_freeze(expr: &Expression<'_>) -> bool {
    if let Expression::CallExpression(call) = expr {
        if let Expression::StaticMemberExpression(member) = &call.callee {
            if let Expression::Identifier(obj) = &member.object {
                return obj.name.as_str() == "Object"
                    && member.property.name.as_str() == "freeze";
            }
        }
    }
    false
}

/// Check if an expression has `as const` assertion.
pub fn is_as_const(expr: &Expression<'_>) -> bool {
    if let Expression::TSAsExpression(ts_as) = expr {
        // Check if it's `as const` — the type annotation would be TSTypeReference with "const"
        // In OXC, `as const` is represented as TSAsExpression with a specific type
        return is_const_type_ref(&ts_as.type_annotation);
    }
    // Also check TSSatisfiesExpression and TSTypeAssertion
    false
}

fn is_const_type_ref(ty: &oxc_ast::ast::TSType<'_>) -> bool {
    matches!(ty, oxc_ast::ast::TSType::TSTypeReference(r) if {
        matches!(&r.type_name, oxc_ast::ast::TSTypeName::IdentifierReference(id) if id.name.as_str() == "const")
    })
}

/// Check if an expression is a "safe" const initializer.
pub fn is_safe_const_init(expr: &Expression<'_>) -> bool {
    is_primitive_literal(expr)
        || is_undefined(expr)
        || is_object_freeze(expr)
        || is_as_const(expr)
}

/// Check if an expression is a mutable const initializer.
pub fn is_mutable_const_init(expr: &Expression<'_>) -> bool {
    matches!(
        expr,
        Expression::ObjectExpression(_)
            | Expression::ArrayExpression(_)
            | Expression::NewExpression(_)
            | Expression::CallExpression(_)
    ) && !is_object_freeze(expr)
}

/// Check if expression is a regex with g or y flags.
pub fn is_stateful_regex(expr: &Expression<'_>) -> bool {
    if let Expression::RegExpLiteral(regex) = expr {
        let flags = regex.regex.flags;
        return flags.contains(oxc_ast::ast::RegExpFlags::G)
            || flags.contains(oxc_ast::ast::RegExpFlags::Y);
    }
    false
}

/// Check if expression is `new Map()`, `new Set()`, `new WeakMap()`, `new WeakSet()`.
pub fn is_collection_constructor(expr: &Expression<'_>) -> bool {
    if let Expression::NewExpression(new_expr) = expr {
        if let Expression::Identifier(id) = &new_expr.callee {
            return matches!(
                id.name.as_str(),
                "Map" | "Set" | "WeakMap" | "WeakSet"
            );
        }
    }
    false
}
