use oxc_span::Span;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::engine::graph::ModuleGraph;

/// Import specifier: which symbol is imported.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImportSpecifier {
    pub imported_name: String,
    pub local_name: String,
}

/// The kind of an import statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportKind {
    Named(Vec<ImportSpecifier>),
    Default(String),
    Namespace(String),
    Combined {
        default_local: String,
        named: Vec<ImportSpecifier>,
    },
    SideEffect,
}

/// A module's export entry extracted from the AST.
#[derive(Debug, Clone)]
pub struct ExportEntry {
    pub exported_name: String,
    pub local_name: String,
    pub span: Span,
    pub is_reexport: bool,
    pub source_specifier: Option<String>,
}

/// Mutation impact classification for an export.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MutationImpact {
    Mutating,
    Reading,
    Safe,
    Unknown,
}

impl std::fmt::Display for MutationImpact {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MutationImpact::Mutating => write!(f, "mutating"),
            MutationImpact::Reading => write!(f, "reading"),
            MutationImpact::Safe => write!(f, "safe"),
            MutationImpact::Unknown => write!(f, "unknown"),
        }
    }
}

/// Export analysis result: export entry + mutation classification.
#[derive(Debug, Clone)]
pub struct ExportAnalysis {
    pub entry: ExportEntry,
    pub impact: MutationImpact,
    pub referenced_bindings: Vec<String>,
}

/// Kind of mock applied to a module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MockKind {
    /// Full mock — module code never executes (`vi.mock('path')` or `vi.mock('path', () => ...)`)
    Full,
    /// Partial mock — original module loads, some exports overridden (`importOriginal`)
    Partial,
}

/// A mock declaration found in a test file.
#[derive(Debug, Clone)]
pub struct MockDeclaration {
    pub source: String,
    pub resolved_path: Option<PathBuf>,
    pub kind: MockKind,
    pub span: Span,
}

/// An import found in a source file.
#[derive(Debug, Clone)]
pub struct ImportInfo {
    pub source: String,
    pub resolved_path: Option<PathBuf>,
    pub is_type_only: bool,
    pub is_side_effect: bool,
    pub span: Span,
    pub kind: ImportKind,
}

/// Kind of edge in the module graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EdgeKind {
    StaticImport,
    DynamicImport,
    ReExport,
    SideEffectImport,
}

/// Test framework detected in the project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestFramework {
    Vitest,
    Jest,
    Unknown,
}

impl TestFramework {
    pub fn mock_fn_name(&self) -> &'static str {
        match self {
            TestFramework::Vitest => "vi.mock",
            TestFramework::Jest | TestFramework::Unknown => "jest.mock",
        }
    }

    /// Returns true if a concrete framework was detected (not Unknown).
    pub fn is_detected(&self) -> bool {
        !matches!(self, TestFramework::Unknown)
    }
}

/// Safe signal found in a test file (e.g., beforeEach with resetModules).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SafeSignal {
    ResetModules,
    RestoreAllMocks,
    ClearState,
}

/// Context for a single module file — built once, shared across all rules.
pub struct ModuleContext {
    /// Absolute path of the file being analyzed.
    pub file_path: PathBuf,
    /// Original source text.
    pub source_text: String,
    /// Whether this is a test file.
    pub is_test_file: bool,
    /// Test framework detected.
    pub framework: TestFramework,
    /// Imports found in this file.
    pub imports: Vec<ImportInfo>,
    /// Mock declarations found (only for test files).
    pub mocks: Vec<MockDeclaration>,
    /// Safe signals found (only for test files).
    pub safe_signals: Vec<SafeSignal>,
}

impl ModuleContext {
    pub fn new(file_path: PathBuf, source_text: String, framework: TestFramework) -> Self {
        let is_test_file = is_test_file_path(&file_path);
        Self {
            file_path,
            source_text,
            is_test_file,
            framework,
            imports: Vec::new(),
            mocks: Vec::new(),
            safe_signals: Vec::new(),
        }
    }

    /// Check if a given resolved path is fully mocked in this test file.
    pub fn is_fully_mocked(&self, resolved_path: &Path) -> bool {
        self.mocks.iter().any(|m| {
            m.kind == MockKind::Full
                && m.resolved_path.as_deref() == Some(resolved_path)
        })
    }

    /// Check if a given resolved path is partially mocked.
    pub fn is_partially_mocked(&self, resolved_path: &Path) -> bool {
        self.mocks.iter().any(|m| {
            m.kind == MockKind::Partial
                && m.resolved_path.as_deref() == Some(resolved_path)
        })
    }

    /// Check if a given resolved path is mocked (full or partial).
    pub fn is_mocked(&self, resolved_path: &Path) -> bool {
        self.mocks.iter().any(|m| m.resolved_path.as_deref() == Some(resolved_path))
    }
}

/// Context for graph-level analysis — built after all files are processed.
pub struct GraphContext {
    /// The module graph.
    pub graph: ModuleGraph,
    /// Global mock registry: maps resolved module path to list of (test file, mock kind).
    pub mock_registry: HashMap<PathBuf, Vec<(PathBuf, MockKind)>>,
    /// Per-module hazard list (from AST analysis).
    pub module_hazards: HashMap<PathBuf, Vec<crate::rule::Hazard>>,
    /// Per-test-file module context summary.
    pub test_contexts: HashMap<PathBuf, TestContextSummary>,
    /// Per-module export analysis results.
    pub export_analyses: HashMap<PathBuf, Vec<ExportAnalysis>>,
    /// Per-module import info (with resolved paths and ImportKind).
    pub all_imports: HashMap<PathBuf, Vec<ImportInfo>>,
}

/// Summary of a test file's context for graph analysis.
#[derive(Debug, Clone)]
pub struct TestContextSummary {
    pub file_path: PathBuf,
    pub mocks: Vec<MockDeclaration>,
    pub safe_signals: Vec<SafeSignal>,
    pub framework: TestFramework,
}

/// Check if a file path looks like a test file.
pub fn is_test_file_path(path: &Path) -> bool {
    let path_str = path.to_string_lossy();

    // Check for __tests__ directory
    if path_str.contains("__tests__") {
        return true;
    }

    // Check file name patterns
    if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
        let lower = file_name.to_lowercase();
        if lower.ends_with(".test.ts")
            || lower.ends_with(".test.tsx")
            || lower.ends_with(".spec.ts")
            || lower.ends_with(".spec.tsx")
            || lower.ends_with(".test.js")
            || lower.ends_with(".test.jsx")
            || lower.ends_with(".spec.js")
            || lower.ends_with(".spec.jsx")
        {
            return true;
        }
    }

    false
}
