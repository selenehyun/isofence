use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Argument, ArrayExpressionElement, BindingPatternKind, CallExpression, Declaration, Expression,
    Program, Statement, VariableDeclarationKind,
};
use oxc_parser::{ParseOptions, Parser};
use oxc_span::{GetSpan, SourceType};
use std::collections::HashSet;
use std::path::Path;

use crate::engine::context::{
    ExportAnalysis, ExportEntry, ImportInfo, ImportKind, ImportSpecifier, MockDeclaration,
    MockKind, MutationImpact, SafeSignal, TestFramework,
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

                let kind = compute_import_kind(decl);

                imports.push(ImportInfo {
                    source,
                    resolved_path: None,
                    is_type_only,
                    is_side_effect,
                    span: decl.span,
                    kind,
                });
            }
            Statement::ExportNamedDeclaration(decl) => {
                if let Some(ref src) = decl.source {
                    // Re-export: `export { X } from './mod'` — treat as Named import
                    let named: Vec<ImportSpecifier> = decl
                        .specifiers
                        .iter()
                        .map(|s| ImportSpecifier {
                            imported_name: s.local.name().to_string(),
                            local_name: s.exported.name().to_string(),
                        })
                        .collect();
                    let kind = if named.is_empty() {
                        ImportKind::SideEffect
                    } else {
                        ImportKind::Named(named)
                    };
                    imports.push(ImportInfo {
                        source: src.value.to_string(),
                        resolved_path: None,
                        is_type_only: decl.export_kind.is_type(),
                        is_side_effect: false,
                        span: decl.span,
                        kind,
                    });
                }
            }
            Statement::ExportAllDeclaration(decl) => {
                let kind = if let Some(ref exported) = decl.exported {
                    ImportKind::Namespace(exported.name().to_string())
                } else {
                    ImportKind::Namespace("*".to_string())
                };
                imports.push(ImportInfo {
                    source: decl.source.value.to_string(),
                    resolved_path: None,
                    is_type_only: decl.export_kind.is_type(),
                    is_side_effect: false,
                    span: decl.span,
                    kind,
                });
            }
            _ => {}
        }
    }

    imports
}

/// Compute the ImportKind from an import declaration's specifiers.
fn compute_import_kind(decl: &oxc_ast::ast::ImportDeclaration<'_>) -> ImportKind {
    use oxc_ast::ast::ImportDeclarationSpecifier;

    let specifiers = match &decl.specifiers {
        Some(specs) if !specs.is_empty() => specs,
        _ => return ImportKind::SideEffect,
    };

    let mut default_local: Option<String> = None;
    let mut named = Vec::new();
    let mut namespace: Option<String> = None;

    for spec in specifiers {
        match spec {
            ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => {
                default_local = Some(s.local.name.to_string());
            }
            ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => {
                namespace = Some(s.local.name.to_string());
            }
            ImportDeclarationSpecifier::ImportSpecifier(s) => {
                named.push(ImportSpecifier {
                    imported_name: s.imported.name().to_string(),
                    local_name: s.local.name.to_string(),
                });
            }
        }
    }

    // Namespace takes precedence
    if let Some(ns) = namespace {
        return ImportKind::Namespace(ns);
    }

    match (default_local, named.is_empty()) {
        (Some(dl), false) => ImportKind::Combined {
            default_local: dl,
            named,
        },
        (Some(dl), true) => ImportKind::Default(dl),
        (None, false) => ImportKind::Named(named),
        (None, true) => ImportKind::SideEffect,
    }
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
                // Check for beforeEach(...) or afterEach(...)
                let is_lifecycle = match &call.callee {
                    Expression::Identifier(id) => matches!(id.name.as_str(), "beforeEach" | "afterEach"),
                    _ => false,
                };

                if is_lifecycle {
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
                    let is_vi_or_jest = match &member.object {
                        Expression::Identifier(id) => matches!(id.name.as_str(), "vi" | "jest"),
                        _ => false,
                    };
                    if !is_vi_or_jest {
                        continue;
                    }
                    let method = member.property.name.as_str();
                    match method {
                        "resetModules" => signals.push(SafeSignal::ResetModules),
                        "restoreAllMocks" => signals.push(SafeSignal::RestoreAllMocks),
                        "clearAllMocks" => signals.push(SafeSignal::ClearAllMocks),
                        "resetAllMocks" => signals.push(SafeSignal::ResetAllMocks),
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

/// Arrow function or function expression — always immutable.
pub fn is_function_value(expr: &Expression<'_>) -> bool {
    matches!(
        expr,
        Expression::ArrowFunctionExpression(_) | Expression::FunctionExpression(_)
    )
}

/// Check if an array element is a safe leaf value (primitive, identifier, or member access).
fn is_safe_array_element(elem: &ArrayExpressionElement<'_>) -> bool {
    matches!(
        elem,
        ArrayExpressionElement::Elision(_)
            | ArrayExpressionElement::StringLiteral(_)
            | ArrayExpressionElement::NumericLiteral(_)
            | ArrayExpressionElement::BooleanLiteral(_)
            | ArrayExpressionElement::NullLiteral(_)
            | ArrayExpressionElement::TemplateLiteral(_)
            | ArrayExpressionElement::Identifier(_)
            | ArrayExpressionElement::StaticMemberExpression(_)
            | ArrayExpressionElement::ComputedMemberExpression(_)
            | ArrayExpressionElement::UnaryExpression(_)
    )
}

/// Non-empty array containing only primitive literals, identifiers, and member expressions.
/// e.g. `['PAID', 'UNPAID']`, `[Status.PAID, Status.UNPAID]`, `[1, 2, 3]`
/// Empty arrays `[]` are still mutable containers and NOT considered safe.
pub fn is_literal_only_array(expr: &Expression<'_>) -> bool {
    if let Expression::ArrayExpression(arr) = expr {
        return !arr.elements.is_empty()
            && arr.elements.iter().all(|elem| is_safe_array_element(elem));
    }
    false
}

/// Schema builder call — z.object(), z.string(), Joi.object(), yup.object(), etc.
/// These return immutable schema descriptors.
pub fn is_schema_builder_call(expr: &Expression<'_>) -> bool {
    // Walk through method chains: z.object({...}).optional().default(...)
    let root_call = unwrap_chain(expr);
    if let Expression::CallExpression(call) = root_call {
        if let Expression::StaticMemberExpression(member) = &call.callee {
            if let Expression::Identifier(obj) = &member.object {
                return matches!(
                    obj.name.as_str(),
                    "z" | "zod" | "Joi" | "joi" | "yup" | "Yup"
                );
            }
        }
    }
    false
}

/// Unwrap method chains to find the root call expression.
/// e.g. `z.object({}).optional()` → `z.object({})`
fn unwrap_chain<'a>(expr: &'a Expression<'a>) -> &'a Expression<'a> {
    if let Expression::CallExpression(call) = expr {
        if let Expression::StaticMemberExpression(member) = &call.callee {
            if let Expression::CallExpression(_) = &member.object {
                return unwrap_chain(&member.object);
            }
        }
    }
    expr
}

/// Symbol() or Symbol.for() — always unique and immutable.
pub fn is_symbol_call(expr: &Expression<'_>) -> bool {
    if let Expression::CallExpression(call) = expr {
        match &call.callee {
            Expression::Identifier(id) if id.name.as_str() == "Symbol" => return true,
            Expression::StaticMemberExpression(member) => {
                if let Expression::Identifier(obj) = &member.object {
                    return obj.name.as_str() == "Symbol";
                }
            }
            _ => {}
        }
    }
    false
}

/// DI token constructors — `new Token(...)`, `new InjectionToken(...)`.
/// These are marker objects used for dependency injection, effectively immutable.
pub fn is_safe_constructor(expr: &Expression<'_>) -> bool {
    if let Expression::NewExpression(new_expr) = expr {
        if let Expression::Identifier(id) = &new_expr.callee {
            return matches!(
                id.name.as_str(),
                "Token" | "InjectionToken" | "OpaqueToken"
            );
        }
    }
    false
}

/// Identifier or member expression reference — doesn't create new mutable state,
/// just aliases an existing binding. The referenced value's mutability is checked
/// separately by rules on its declaration.
pub fn is_reference_value(expr: &Expression<'_>) -> bool {
    matches!(
        expr,
        Expression::Identifier(_)
            | Expression::StaticMemberExpression(_)
            | Expression::ComputedMemberExpression(_)
    )
}

/// Array whose elements are all `...Object.values(X)` or `...Object.keys(X)` spreads.
/// e.g. `[...Object.values(FooEnum), ...Object.values(BarEnum)]`
/// These produce a new array of primitive enum values — effectively immutable lookup data.
pub fn is_enum_spread_array(expr: &Expression<'_>) -> bool {
    if let Expression::ArrayExpression(arr) = expr {
        if arr.elements.is_empty() {
            return false;
        }
        return arr.elements.iter().all(|elem| {
            if let ArrayExpressionElement::SpreadElement(spread) = elem {
                is_object_values_or_keys_call(&spread.argument)
            } else {
                false
            }
        });
    }
    false
}

/// Check if expression is `Object.values(X)` or `Object.keys(X)`.
fn is_object_values_or_keys_call(expr: &Expression<'_>) -> bool {
    if let Expression::CallExpression(call) = expr {
        if let Expression::StaticMemberExpression(member) = &call.callee {
            if let Expression::Identifier(obj) = &member.object {
                return obj.name.as_str() == "Object"
                    && matches!(member.property.name.as_str(), "values" | "keys");
            }
        }
    }
    false
}

/// Check if a value is a "primitive leaf" — a value that cannot be mutated.
/// Includes: primitive literals, undefined, identifiers, member expressions,
/// template literals, unary expressions, `process.env.*`, and `as const`.
fn is_primitive_leaf(expr: &Expression<'_>) -> bool {
    is_primitive_literal(expr)
        || is_undefined(expr)
        || is_as_const(expr)
        || is_function_value(expr)
        || is_reference_value(expr)
        || matches!(expr, Expression::UnaryExpression(_))
        || is_primitive_compound(expr)
}

/// Check if a logical/binary expression has all-primitive-leaf operands.
/// e.g. `isStage && !isTenant`, `a || b`, `x + 1` → true
/// e.g. `getConfig() && flag` → false (call expression operand)
fn is_primitive_compound(expr: &Expression<'_>) -> bool {
    match expr {
        Expression::LogicalExpression(e) => {
            is_primitive_leaf(&e.left) && is_primitive_leaf(&e.right)
        }
        Expression::BinaryExpression(e) => {
            is_primitive_leaf(&e.left) && is_primitive_leaf(&e.right)
        }
        Expression::ConditionalExpression(e) => {
            is_primitive_leaf(&e.test)
                && is_primitive_leaf(&e.consequent)
                && is_primitive_leaf(&e.alternate)
        }
        _ => false,
    }
}

/// Recursively check if an expression is a "primitive-only" structure:
/// objects/arrays whose leaf values are all primitives, identifiers, member expressions,
/// or nested primitive-only objects/arrays. Uses a depth limit for performance.
///
/// Examples that return true:
/// - `{ type: 'mysql', port: 3306, flag: true }`
/// - `{ Alabama: { code: 'AL', tz: 'America/Chicago' } }`
/// - `[1, 2, [3, 4]]`
///
/// Examples that return false:
/// - `{ handler: someCall() }` (call expression)
/// - `{ client: new HttpClient() }` (constructor)
pub fn is_primitive_only_value(expr: &Expression<'_>, depth: u8) -> bool {
    if depth == 0 {
        return false;
    }

    if is_primitive_leaf(expr) {
        return true;
    }

    match expr {
        Expression::ObjectExpression(obj) => {
            if obj.properties.is_empty() {
                return false; // empty object `{}` is still a mutable container
            }
            // Pure spread `{ ...x }` without own properties is just aliasing — not safe
            let has_own_property = obj.properties.iter().any(|prop| {
                matches!(prop, oxc_ast::ast::ObjectPropertyKind::ObjectProperty(_))
            });
            if !has_own_property {
                return false;
            }
            obj.properties.iter().all(|prop| {
                match prop {
                    oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) => {
                        is_primitive_only_value(&p.value, depth - 1)
                    }
                    oxc_ast::ast::ObjectPropertyKind::SpreadProperty(spread) => {
                        // Only allow spread from plain identifiers (not calls or member chains)
                        matches!(&spread.argument, Expression::Identifier(_))
                    }
                }
            })
        }
        Expression::ArrayExpression(arr) => {
            if arr.elements.is_empty() {
                return false; // empty array `[]` is still a mutable container
            }
            arr.elements.iter().all(|elem| {
                match elem {
                    ArrayExpressionElement::SpreadElement(_) => false,
                    ArrayExpressionElement::Elision(_) => true,
                    _ => {
                        if let Some(inner) = elem.as_expression() {
                            is_primitive_only_value(inner, depth - 1)
                        } else {
                            false
                        }
                    }
                }
            })
        }
        _ => false,
    }
}

/// Check if an expression is a "safe" const initializer.
pub fn is_safe_const_init(expr: &Expression<'_>) -> bool {
    is_primitive_literal(expr)
        || is_undefined(expr)
        || is_object_freeze(expr)
        || is_as_const(expr)
        || is_function_value(expr)
        || is_literal_only_array(expr)
        || is_schema_builder_call(expr)
        || is_symbol_call(expr)
        || is_safe_constructor(expr)
        || is_enum_spread_array(expr)
        || is_primitive_only_value(expr, 4)
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

// ===== Export-level mutation analysis =====

/// Extract export declarations from the AST.
pub fn extract_exports(program: &Program<'_>) -> Vec<ExportEntry> {
    let mut exports = Vec::new();

    for stmt in &program.body {
        match stmt {
            // export { X, Y }
            Statement::ExportNamedDeclaration(decl) => {
                if let Some(ref src) = decl.source {
                    // Re-export: export { X } from './other'
                    for spec in &decl.specifiers {
                        exports.push(ExportEntry {
                            exported_name: spec.exported.name().to_string(),
                            local_name: spec.local.name().to_string(),
                            span: spec.span(),
                            is_reexport: true,
                            source_specifier: Some(src.value.to_string()),
                        });
                    }
                } else if let Some(ref declaration) = decl.declaration {
                    // export const/let/var/function/class
                    extract_declaration_exports(declaration, &mut exports);
                } else {
                    // export { X, Y } (local re-export)
                    for spec in &decl.specifiers {
                        exports.push(ExportEntry {
                            exported_name: spec.exported.name().to_string(),
                            local_name: spec.local.name().to_string(),
                            span: spec.span(),
                            is_reexport: false,
                            source_specifier: None,
                        });
                    }
                }
            }
            // export * from './other'
            Statement::ExportAllDeclaration(decl) => {
                let exported_name = decl
                    .exported
                    .as_ref()
                    .map(|e| e.name().to_string())
                    .unwrap_or_else(|| "*".to_string());
                exports.push(ExportEntry {
                    exported_name,
                    local_name: "*".to_string(),
                    span: decl.span,
                    is_reexport: true,
                    source_specifier: Some(decl.source.value.to_string()),
                });
            }
            // export default ...
            Statement::ExportDefaultDeclaration(decl) => {
                let local_name = match &decl.declaration {
                    oxc_ast::ast::ExportDefaultDeclarationKind::FunctionDeclaration(f) => f
                        .id
                        .as_ref()
                        .map(|id| id.name.to_string())
                        .unwrap_or_else(|| "default".to_string()),
                    oxc_ast::ast::ExportDefaultDeclarationKind::ClassDeclaration(c) => c
                        .id
                        .as_ref()
                        .map(|id| id.name.to_string())
                        .unwrap_or_else(|| "default".to_string()),
                    _ => "default".to_string(),
                };
                exports.push(ExportEntry {
                    exported_name: "default".to_string(),
                    local_name,
                    span: decl.span,
                    is_reexport: false,
                    source_specifier: None,
                });
            }
            _ => {}
        }
    }

    exports
}

/// Extract exports from a declaration (const/let/var/function/class).
fn extract_declaration_exports(decl: &Declaration<'_>, exports: &mut Vec<ExportEntry>) {
    match decl {
        Declaration::VariableDeclaration(var_decl) => {
            for declarator in &var_decl.declarations {
                if let Some(name) = declarator.id.get_identifier_name() {
                    exports.push(ExportEntry {
                        exported_name: name.to_string(),
                        local_name: name.to_string(),
                        span: declarator.span,
                        is_reexport: false,
                        source_specifier: None,
                    });
                }
            }
        }
        Declaration::FunctionDeclaration(func) => {
            if let Some(ref id) = func.id {
                exports.push(ExportEntry {
                    exported_name: id.name.to_string(),
                    local_name: id.name.to_string(),
                    span: func.span(),
                    is_reexport: false,
                    source_specifier: None,
                });
            }
        }
        Declaration::ClassDeclaration(class) => {
            if let Some(ref id) = class.id {
                exports.push(ExportEntry {
                    exported_name: id.name.to_string(),
                    local_name: id.name.to_string(),
                    span: class.span(),
                    is_reexport: false,
                    source_specifier: None,
                });
            }
        }
        _ => {}
    }
}

/// Collect module-level mutable binding names (Error-severity patterns only).
pub fn collect_module_mutable_bindings(program: &Program<'_>) -> HashSet<String> {
    let mut bindings = HashSet::new();

    for stmt in &program.body {
        collect_mutable_bindings_from_stmt(stmt, &mut bindings);
    }

    bindings
}

fn collect_mutable_bindings_from_stmt(stmt: &Statement<'_>, bindings: &mut HashSet<String>) {
    let var_decl = match stmt {
        Statement::VariableDeclaration(decl) => decl,
        Statement::ExportNamedDeclaration(export) => match export.declaration.as_ref() {
            Some(Declaration::VariableDeclaration(decl)) => decl,
            _ => return,
        },
        _ => return,
    };

    for declarator in &var_decl.declarations {
        let name = match declarator.id.get_identifier_name() {
            Some(n) => n.to_string(),
            None => continue,
        };

        match var_decl.kind {
            // let/var are always mutable
            VariableDeclarationKind::Let | VariableDeclarationKind::Var => {
                bindings.insert(name);
            }
            VariableDeclarationKind::Const => {
                // Only Error-level mutable const patterns, skip safe inits
                if let Some(init) = &declarator.init {
                    if !is_safe_const_init(init)
                        && (is_mutable_object_or_array(init)
                            || is_collection_constructor(init)
                            || is_stateful_regex(init))
                    {
                        bindings.insert(name);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Check if an expression is an object literal {} or empty array [] (Error-level mutable).
fn is_mutable_object_or_array(expr: &Expression<'_>) -> bool {
    match expr {
        Expression::ObjectExpression(_) => !is_object_freeze(expr),
        Expression::ArrayExpression(arr) => {
            // Empty array or non-literal-only array is mutable
            arr.elements.is_empty() || !is_literal_only_array(expr)
        }
        _ => false,
    }
}

/// Analyze mutation impact of each export relative to module-level mutable bindings.
pub fn analyze_export_mutation(
    program: &Program<'_>,
    exports: &[ExportEntry],
    mutable_bindings: &HashSet<String>,
) -> Vec<ExportAnalysis> {
    if mutable_bindings.is_empty() {
        // No mutable state → re-exports are Unknown, others are Safe
        return exports
            .iter()
            .map(|entry| ExportAnalysis {
                entry: entry.clone(),
                impact: if entry.is_reexport {
                    MutationImpact::Unknown
                } else {
                    MutationImpact::Safe
                },
                referenced_bindings: vec![],
            })
            .collect();
    }

    // Build a map: local function/variable name → its AST statement (for body analysis)
    let func_bodies = collect_function_bodies(program);

    exports
        .iter()
        .map(|entry| {
            if entry.is_reexport {
                return ExportAnalysis {
                    entry: entry.clone(),
                    impact: MutationImpact::Unknown,
                    referenced_bindings: vec![],
                };
            }

            // Check if the export's local name is itself a mutable binding
            if mutable_bindings.contains(&entry.local_name) {
                return ExportAnalysis {
                    entry: entry.clone(),
                    impact: MutationImpact::Mutating,
                    referenced_bindings: vec![entry.local_name.clone()],
                };
            }

            // For function exports, analyze the body
            if let Some(body_stmts) = func_bodies.get(entry.local_name.as_str()) {
                let (read_refs, write_refs) =
                    scan_statements_for_refs(body_stmts, mutable_bindings);

                if !write_refs.is_empty() {
                    return ExportAnalysis {
                        entry: entry.clone(),
                        impact: MutationImpact::Mutating,
                        referenced_bindings: write_refs.into_iter().collect(),
                    };
                }
                if !read_refs.is_empty() {
                    return ExportAnalysis {
                        entry: entry.clone(),
                        impact: MutationImpact::Reading,
                        referenced_bindings: read_refs.into_iter().collect(),
                    };
                }
            }

            ExportAnalysis {
                entry: entry.clone(),
                impact: MutationImpact::Safe,
                referenced_bindings: vec![],
            }
        })
        .collect()
}

/// Collect function body statements indexed by function name.
/// Returns references into the AST statements vector.
fn collect_function_bodies<'a>(
    program: &'a Program<'a>,
) -> std::collections::HashMap<&'a str, Vec<&'a Statement<'a>>> {
    let mut map = std::collections::HashMap::new();

    for stmt in &program.body {
        match stmt {
            Statement::FunctionDeclaration(func) => {
                if let Some(ref id) = func.id {
                    if let Some(ref body) = func.body {
                        let stmts: Vec<&Statement<'a>> = body.statements.iter().collect();
                        map.insert(id.name.as_str(), stmts);
                    }
                }
            }
            Statement::ExportNamedDeclaration(export) => {
                if let Some(ref declaration) = export.declaration {
                    match declaration {
                        Declaration::FunctionDeclaration(func) => {
                            if let Some(ref id) = func.id {
                                if let Some(ref body) = func.body {
                                    let stmts: Vec<&Statement<'a>> =
                                        body.statements.iter().collect();
                                    map.insert(id.name.as_str(), stmts);
                                }
                            }
                        }
                        Declaration::VariableDeclaration(var_decl) => {
                            // export const fn = () => { ... }
                            for declarator in &var_decl.declarations {
                                if let Some(name) = declarator.id.get_identifier_name() {
                                    if let Some(init) = &declarator.init {
                                        if let Some(stmts) = extract_fn_body_stmts(init) {
                                            map.insert(name.as_str(), stmts);
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            Statement::ExportDefaultDeclaration(export) => {
                match &export.declaration {
                    oxc_ast::ast::ExportDefaultDeclarationKind::FunctionDeclaration(func) => {
                        let name = func
                            .id
                            .as_ref()
                            .map(|id| id.name.as_str())
                            .unwrap_or("default");
                        if let Some(ref body) = func.body {
                            let stmts: Vec<&Statement<'a>> = body.statements.iter().collect();
                            map.insert(name, stmts);
                        }
                    }
                    _ => {}
                }
            }
            Statement::VariableDeclaration(var_decl) => {
                // const fn = () => { ... }  (non-exported, but may be referenced by export { fn })
                for declarator in &var_decl.declarations {
                    if let Some(name) = declarator.id.get_identifier_name() {
                        if let Some(init) = &declarator.init {
                            if let Some(stmts) = extract_fn_body_stmts(init) {
                                map.insert(name.as_str(), stmts);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    map
}

/// Extract function body statements from arrow/function expressions.
fn extract_fn_body_stmts<'a>(expr: &'a Expression<'a>) -> Option<Vec<&'a Statement<'a>>> {
    match expr {
        Expression::ArrowFunctionExpression(arrow) => {
            Some(arrow.body.statements.iter().collect())
        }
        Expression::FunctionExpression(func) => func
            .body
            .as_ref()
            .map(|b| b.statements.iter().collect()),
        _ => None,
    }
}

// ===== Function body scanning for mutable binding references =====

/// Scope stack for tracking variable shadowing.
struct ScopeStack {
    scopes: Vec<HashSet<String>>,
}

impl ScopeStack {
    fn new() -> Self {
        Self {
            scopes: vec![HashSet::new()],
        }
    }

    fn push(&mut self) {
        self.scopes.push(HashSet::new());
    }

    fn pop(&mut self) {
        if self.scopes.len() > 1 {
            self.scopes.pop();
        }
    }

    fn declare(&mut self, name: String) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name);
        }
    }

    /// Check if a name is shadowed by an inner scope declaration.
    fn is_shadowed(&self, name: &str) -> bool {
        // Check all scopes except the first (module scope)
        self.scopes.iter().skip(1).any(|s| s.contains(name))
    }
}

/// Scan statements for references to mutable bindings. Returns (read_refs, write_refs).
/// Pushes a scope for the function body so local declarations shadow module-level bindings.
fn scan_statements_for_refs(
    stmts: &[&Statement<'_>],
    mutable_bindings: &HashSet<String>,
) -> (HashSet<String>, HashSet<String>) {
    let mut read_refs = HashSet::new();
    let mut write_refs = HashSet::new();
    let mut scope = ScopeStack::new();

    // Push a scope for the function body so local declarations shadow module-level bindings
    scope.push();

    for stmt in stmts {
        scan_statement(stmt, mutable_bindings, &mut read_refs, &mut write_refs, &mut scope);
    }

    scope.pop();

    (read_refs, write_refs)
}

fn scan_statement(
    stmt: &Statement<'_>,
    mutable_bindings: &HashSet<String>,
    read_refs: &mut HashSet<String>,
    write_refs: &mut HashSet<String>,
    scope: &mut ScopeStack,
) {
    match stmt {
        Statement::ExpressionStatement(expr_stmt) => {
            scan_expression(&expr_stmt.expression, mutable_bindings, read_refs, write_refs, scope);
        }
        Statement::ReturnStatement(ret) => {
            if let Some(ref arg) = ret.argument {
                scan_expression(arg, mutable_bindings, read_refs, write_refs, scope);
            }
        }
        Statement::VariableDeclaration(decl) => {
            for declarator in &decl.declarations {
                if let Some(name) = declarator.id.get_identifier_name() {
                    scope.declare(name.to_string());
                }
                if let Some(ref init) = declarator.init {
                    scan_expression(init, mutable_bindings, read_refs, write_refs, scope);
                }
            }
        }
        Statement::IfStatement(if_stmt) => {
            scan_expression(&if_stmt.test, mutable_bindings, read_refs, write_refs, scope);
            scan_statement(&if_stmt.consequent, mutable_bindings, read_refs, write_refs, scope);
            if let Some(ref alt) = if_stmt.alternate {
                scan_statement(alt, mutable_bindings, read_refs, write_refs, scope);
            }
        }
        Statement::BlockStatement(block) => {
            scope.push();
            for s in &block.body {
                scan_statement(s, mutable_bindings, read_refs, write_refs, scope);
            }
            scope.pop();
        }
        Statement::ForStatement(for_stmt) => {
            scope.push();
            if let Some(ref init) = for_stmt.init {
                if let oxc_ast::ast::ForStatementInit::VariableDeclaration(decl) = init {
                    for declarator in &decl.declarations {
                        if let Some(name) = declarator.id.get_identifier_name() {
                            scope.declare(name.to_string());
                        }
                        if let Some(ref init_expr) = declarator.init {
                            scan_expression(init_expr, mutable_bindings, read_refs, write_refs, scope);
                        }
                    }
                }
            }
            if let Some(ref test) = for_stmt.test {
                scan_expression(test, mutable_bindings, read_refs, write_refs, scope);
            }
            if let Some(ref update) = for_stmt.update {
                scan_expression(update, mutable_bindings, read_refs, write_refs, scope);
            }
            scan_statement(&for_stmt.body, mutable_bindings, read_refs, write_refs, scope);
            scope.pop();
        }
        Statement::ForInStatement(for_in) => {
            scope.push();
            scan_expression(&for_in.right, mutable_bindings, read_refs, write_refs, scope);
            scan_statement(&for_in.body, mutable_bindings, read_refs, write_refs, scope);
            scope.pop();
        }
        Statement::ForOfStatement(for_of) => {
            scope.push();
            scan_expression(&for_of.right, mutable_bindings, read_refs, write_refs, scope);
            scan_statement(&for_of.body, mutable_bindings, read_refs, write_refs, scope);
            scope.pop();
        }
        Statement::WhileStatement(while_stmt) => {
            scan_expression(&while_stmt.test, mutable_bindings, read_refs, write_refs, scope);
            scan_statement(&while_stmt.body, mutable_bindings, read_refs, write_refs, scope);
        }
        Statement::DoWhileStatement(do_while) => {
            scan_statement(&do_while.body, mutable_bindings, read_refs, write_refs, scope);
            scan_expression(&do_while.test, mutable_bindings, read_refs, write_refs, scope);
        }
        Statement::SwitchStatement(switch_stmt) => {
            scan_expression(&switch_stmt.discriminant, mutable_bindings, read_refs, write_refs, scope);
            for case in &switch_stmt.cases {
                if let Some(ref test) = case.test {
                    scan_expression(test, mutable_bindings, read_refs, write_refs, scope);
                }
                for s in &case.consequent {
                    scan_statement(s, mutable_bindings, read_refs, write_refs, scope);
                }
            }
        }
        Statement::TryStatement(try_stmt) => {
            scope.push();
            for s in &try_stmt.block.body {
                scan_statement(s, mutable_bindings, read_refs, write_refs, scope);
            }
            scope.pop();
            if let Some(ref handler) = try_stmt.handler {
                scope.push();
                if let Some(ref param) = handler.param {
                    if let Some(name) = param.pattern.get_identifier_name() {
                        scope.declare(name.to_string());
                    }
                }
                for s in &handler.body.body {
                    scan_statement(s, mutable_bindings, read_refs, write_refs, scope);
                }
                scope.pop();
            }
            if let Some(ref finalizer) = try_stmt.finalizer {
                scope.push();
                for s in &finalizer.body {
                    scan_statement(s, mutable_bindings, read_refs, write_refs, scope);
                }
                scope.pop();
            }
        }
        Statement::ThrowStatement(throw_stmt) => {
            scan_expression(&throw_stmt.argument, mutable_bindings, read_refs, write_refs, scope);
        }
        _ => {}
    }
}

fn scan_expression(
    expr: &Expression<'_>,
    mutable_bindings: &HashSet<String>,
    read_refs: &mut HashSet<String>,
    write_refs: &mut HashSet<String>,
    scope: &mut ScopeStack,
) {
    match expr {
        Expression::Identifier(id) => {
            let name = id.name.as_str();
            if mutable_bindings.contains(name) && !scope.is_shadowed(name) {
                read_refs.insert(name.to_string());
            }
        }
        Expression::AssignmentExpression(assign) => {
            // Check if the assignment target is a mutable binding
            if let oxc_ast::ast::AssignmentTarget::AssignmentTargetIdentifier(id) = &assign.left {
                let name = id.name.as_str();
                if mutable_bindings.contains(name) && !scope.is_shadowed(name) {
                    write_refs.insert(name.to_string());
                }
            } else {
                // Member expression assignment: check if object is mutable binding
                check_member_write(&assign.left, mutable_bindings, write_refs, scope);
            }
            scan_expression(&assign.right, mutable_bindings, read_refs, write_refs, scope);
        }
        Expression::UpdateExpression(update) => {
            if let oxc_ast::ast::SimpleAssignmentTarget::AssignmentTargetIdentifier(id) =
                &update.argument
            {
                let name = id.name.as_str();
                if mutable_bindings.contains(name) && !scope.is_shadowed(name) {
                    write_refs.insert(name.to_string());
                }
            }
        }
        Expression::CallExpression(call) => {
            // Check for method calls on mutable bindings (e.g., cache.set(...))
            if let Expression::StaticMemberExpression(member) = &call.callee {
                if let Expression::Identifier(id) = &member.object {
                    let name = id.name.as_str();
                    let method = member.property.name.as_str();
                    if mutable_bindings.contains(name) && !scope.is_shadowed(name) {
                        if is_mutating_method(method) {
                            write_refs.insert(name.to_string());
                        } else {
                            read_refs.insert(name.to_string());
                        }
                    }
                }
            }
            // Scan callee and arguments
            scan_expression(&call.callee, mutable_bindings, read_refs, write_refs, scope);
            for arg in &call.arguments {
                match arg {
                    Argument::SpreadElement(spread) => {
                        scan_expression(&spread.argument, mutable_bindings, read_refs, write_refs, scope);
                    }
                    _ => {
                        if let Some(expr) = arg.as_expression() {
                            scan_expression(expr, mutable_bindings, read_refs, write_refs, scope);
                        }
                    }
                }
            }
        }
        Expression::StaticMemberExpression(member) => {
            scan_expression(&member.object, mutable_bindings, read_refs, write_refs, scope);
        }
        Expression::ComputedMemberExpression(member) => {
            scan_expression(&member.object, mutable_bindings, read_refs, write_refs, scope);
            scan_expression(&member.expression, mutable_bindings, read_refs, write_refs, scope);
        }
        Expression::BinaryExpression(bin) => {
            scan_expression(&bin.left, mutable_bindings, read_refs, write_refs, scope);
            scan_expression(&bin.right, mutable_bindings, read_refs, write_refs, scope);
        }
        Expression::LogicalExpression(log) => {
            scan_expression(&log.left, mutable_bindings, read_refs, write_refs, scope);
            scan_expression(&log.right, mutable_bindings, read_refs, write_refs, scope);
        }
        Expression::UnaryExpression(unary) => {
            scan_expression(&unary.argument, mutable_bindings, read_refs, write_refs, scope);
        }
        Expression::ConditionalExpression(cond) => {
            scan_expression(&cond.test, mutable_bindings, read_refs, write_refs, scope);
            scan_expression(&cond.consequent, mutable_bindings, read_refs, write_refs, scope);
            scan_expression(&cond.alternate, mutable_bindings, read_refs, write_refs, scope);
        }
        Expression::TemplateLiteral(tpl) => {
            for expr in &tpl.expressions {
                scan_expression(expr, mutable_bindings, read_refs, write_refs, scope);
            }
        }
        Expression::ArrayExpression(arr) => {
            for elem in &arr.elements {
                if let Some(expr) = elem.as_expression() {
                    scan_expression(expr, mutable_bindings, read_refs, write_refs, scope);
                }
            }
        }
        Expression::ObjectExpression(obj) => {
            for prop in &obj.properties {
                if let oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) = prop {
                    scan_expression(&p.value, mutable_bindings, read_refs, write_refs, scope);
                }
            }
        }
        Expression::ArrowFunctionExpression(arrow) => {
            // Recurse into nested arrow functions with scope
            scope.push();
            for param in &arrow.params.items {
                if let Some(name) = param.pattern.get_identifier_name() {
                    scope.declare(name.to_string());
                }
            }
            for s in &arrow.body.statements {
                scan_statement(s, mutable_bindings, read_refs, write_refs, scope);
            }
            scope.pop();
        }
        Expression::FunctionExpression(func) => {
            scope.push();
            for param in &func.params.items {
                if let Some(name) = param.pattern.get_identifier_name() {
                    scope.declare(name.to_string());
                }
            }
            if let Some(ref body) = func.body {
                for s in &body.statements {
                    scan_statement(s, mutable_bindings, read_refs, write_refs, scope);
                }
            }
            scope.pop();
        }
        Expression::AwaitExpression(await_expr) => {
            scan_expression(&await_expr.argument, mutable_bindings, read_refs, write_refs, scope);
        }
        Expression::ParenthesizedExpression(paren) => {
            scan_expression(&paren.expression, mutable_bindings, read_refs, write_refs, scope);
        }
        Expression::SequenceExpression(seq) => {
            for expr in &seq.expressions {
                scan_expression(expr, mutable_bindings, read_refs, write_refs, scope);
            }
        }
        Expression::NewExpression(new_expr) => {
            scan_expression(&new_expr.callee, mutable_bindings, read_refs, write_refs, scope);
            for arg in &new_expr.arguments {
                if let Some(expr) = arg.as_expression() {
                    scan_expression(expr, mutable_bindings, read_refs, write_refs, scope);
                }
            }
        }
        Expression::TaggedTemplateExpression(tagged) => {
            scan_expression(&tagged.tag, mutable_bindings, read_refs, write_refs, scope);
            for expr in &tagged.quasi.expressions {
                scan_expression(expr, mutable_bindings, read_refs, write_refs, scope);
            }
        }
        Expression::TSAsExpression(ts_as) => {
            scan_expression(&ts_as.expression, mutable_bindings, read_refs, write_refs, scope);
        }
        Expression::TSNonNullExpression(nn) => {
            scan_expression(&nn.expression, mutable_bindings, read_refs, write_refs, scope);
        }
        _ => {}
    }
}

/// Check if a member assignment target writes to a mutable binding.
fn check_member_write(
    target: &oxc_ast::ast::AssignmentTarget<'_>,
    mutable_bindings: &HashSet<String>,
    write_refs: &mut HashSet<String>,
    scope: &ScopeStack,
) {
    let root = get_assignment_target_root(target);
    if let Some(name) = root {
        if mutable_bindings.contains(name) && !scope.is_shadowed(name) {
            write_refs.insert(name.to_string());
        }
    }
}

/// Get the root identifier name from an assignment target.
fn get_assignment_target_root<'a>(target: &'a oxc_ast::ast::AssignmentTarget<'a>) -> Option<&'a str> {
    match target {
        oxc_ast::ast::AssignmentTarget::AssignmentTargetIdentifier(id) => Some(id.name.as_str()),
        oxc_ast::ast::AssignmentTarget::StaticMemberExpression(member) => {
            get_expr_root_name(&member.object)
        }
        oxc_ast::ast::AssignmentTarget::ComputedMemberExpression(member) => {
            get_expr_root_name(&member.object)
        }
        _ => None,
    }
}

/// Get the root identifier name from an expression chain.
fn get_expr_root_name<'a>(expr: &'a Expression<'a>) -> Option<&'a str> {
    match expr {
        Expression::Identifier(id) => Some(id.name.as_str()),
        Expression::StaticMemberExpression(member) => get_expr_root_name(&member.object),
        Expression::ComputedMemberExpression(member) => get_expr_root_name(&member.object),
        _ => None,
    }
}

/// Check if a method name is a mutating operation on a collection or object.
fn is_mutating_method(method: &str) -> bool {
    matches!(
        method,
        "set"
            | "delete"
            | "clear"
            | "add"
            | "push"
            | "pop"
            | "shift"
            | "unshift"
            | "splice"
            | "sort"
            | "reverse"
            | "fill"
            | "copyWithin"
    )
}
