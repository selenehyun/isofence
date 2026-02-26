use oxc_ast::ast::{Expression, Program, Statement};
use oxc_span::{GetSpan, Span};

use crate::engine::context::TestFramework;

/// A mock insertion to be applied to a test file.
#[derive(Debug, Clone)]
pub struct MockInsertion {
    /// The module source path (as it appears in the import).
    pub module_source: String,
    /// The span reference (for ordering).
    pub span: Span,
}

/// Find the best insertion point for new mock declarations.
///
/// Algorithm:
/// 1. Find the last vi.mock()/jest.mock() call
/// 2. Find the last ImportDeclaration
/// 3. Insert after whichever comes later
/// 4. If neither exists, insert at the top (after hashbang/directives)
pub fn find_insertion_point(program: &Program<'_>, framework: TestFramework) -> u32 {
    let mut last_mock_end: u32 = 0;
    let mut last_import_end: u32 = 0;

    for stmt in &program.body {
        // Track last import
        if matches!(stmt, Statement::ImportDeclaration(_)) {
            last_import_end = stmt.span().end;
        }

        // Track last mock call
        if let Statement::ExpressionStatement(expr_stmt) = stmt {
            if is_mock_call(&expr_stmt.expression, framework) {
                last_mock_end = expr_stmt.span.end;
            }
        }
    }

    if last_mock_end > 0 || last_import_end > 0 {
        last_mock_end.max(last_import_end)
    } else {
        // Insert at top, after hashbang and directives
        find_top_insertion_point(program)
    }
}

fn is_mock_call(expr: &Expression<'_>, framework: TestFramework) -> bool {
    if let Expression::CallExpression(call) = expr {
        if let Expression::StaticMemberExpression(member) = &call.callee {
            if let Expression::Identifier(obj) = &member.object {
                let method = member.property.name.as_str();
                if method != "mock" && method != "doMock" {
                    return false;
                }

                return match framework {
                    TestFramework::Vitest => obj.name.as_str() == "vi",
                    TestFramework::Jest => obj.name.as_str() == "jest",
                    TestFramework::Unknown => {
                        obj.name.as_str() == "vi" || obj.name.as_str() == "jest"
                    }
                };
            }
        }
    }
    false
}

fn find_top_insertion_point(program: &Program<'_>) -> u32 {
    // Skip hashbang
    if let Some(ref hashbang) = program.hashbang {
        return hashbang.span.end;
    }

    // Skip directives
    for directive in &program.directives {
        return directive.span.end;
    }

    0
}
