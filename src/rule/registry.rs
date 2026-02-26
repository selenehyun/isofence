use std::collections::HashMap;

use crate::rule::{Rule, Severity};

/// Registry that manages all rules (built-in + custom).
pub struct RuleRegistry {
    rules: Vec<Box<dyn Rule>>,
    /// Severity overrides from config. None = use default.
    severity_overrides: HashMap<String, Severity>,
    /// Disabled rules.
    disabled: std::collections::HashSet<String>,
}

impl RuleRegistry {
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            severity_overrides: HashMap::new(),
            disabled: std::collections::HashSet::new(),
        }
    }

    /// Register a rule.
    pub fn register(&mut self, rule: Box<dyn Rule>) {
        self.rules.push(rule);
    }

    /// Register multiple rules.
    pub fn register_all(&mut self, rules: Vec<Box<dyn Rule>>) {
        self.rules.extend(rules);
    }

    /// Set severity override for a rule.
    pub fn set_severity(&mut self, rule_name: &str, severity: Severity) {
        if severity == Severity::Off {
            self.disabled.insert(rule_name.to_string());
        } else {
            self.disabled.remove(rule_name);
            self.severity_overrides
                .insert(rule_name.to_string(), severity);
        }
    }

    /// Disable a rule.
    pub fn disable(&mut self, rule_name: &str) {
        self.disabled.insert(rule_name.to_string());
    }

    /// Get the configured severity for a rule (if overridden).
    pub fn configured_severity(&self, rule_name: &str) -> Option<Severity> {
        self.severity_overrides.get(rule_name).copied()
    }

    /// Get all enabled rules.
    pub fn enabled_rules(&self) -> Vec<&dyn Rule> {
        self.rules
            .iter()
            .filter(|r| !self.disabled.contains(r.meta().name))
            .map(|r| r.as_ref())
            .collect()
    }

    /// Get all registered rule names.
    pub fn rule_names(&self) -> Vec<&str> {
        self.rules.iter().map(|r| r.meta().name).collect()
    }
}

impl Default for RuleRegistry {
    fn default() -> Self {
        Self::new()
    }
}
