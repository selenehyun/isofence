use serde::Deserialize;

/// Schema for a declarative JSON rule file.
#[derive(Debug, Deserialize)]
pub struct DeclarativeRuleFile {
    pub rules: Vec<DeclarativeRuleSchema>,
}

/// Schema for a single declarative rule.
#[derive(Debug, Deserialize)]
pub struct DeclarativeRuleSchema {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_severity")]
    pub severity: String,
    pub r#match: MatchPattern,
    pub message: String,
}

fn default_severity() -> String {
    "error".to_string()
}

/// Match pattern for declarative rules.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MatchPattern {
    /// Match an expression statement.
    ExpressionStatement {
        expression: ExpressionPattern,
    },
    /// Match a variable declaration.
    VarDecl {
        #[serde(default)]
        kind: Option<String>,
        #[serde(default)]
        init: Option<ExpressionPattern>,
    },
    /// Match an import declaration.
    Import {
        #[serde(default)]
        source: Option<StringPattern>,
    },
}

/// Pattern for matching expressions.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExpressionPattern {
    /// Match a function call.
    Call {
        callee: CalleePattern,
    },
    /// Match a `new` expression.
    New {
        callee: CalleePattern,
    },
    /// Match an identifier.
    Identifier {
        name: String,
    },
}

/// Pattern for matching call targets.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum CalleePattern {
    /// Match by identifier name.
    Identifier { name: String },
    /// Match by member expression.
    Member { object: String, property: String },
}

/// Pattern for matching strings.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum StringPattern {
    /// Exact match.
    Exact(String),
    /// Glob/regex pattern.
    Pattern { pattern: String },
}
