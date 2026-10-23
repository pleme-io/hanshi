use std::sync::{Arc, Mutex};

use blue_lang_host::{host, Entry, Program, Value};

use hanshi_domain::{Advisory, Severity};

use crate::error::CoreError;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("unknown severity `{0}`: expected info, warn, error or critical")]
pub struct UnknownSeverity(pub Arc<str>);

fn severity(label: Arc<str>) -> Result<Severity, UnknownSeverity> {
    match &*label {
        "info" => Ok(Severity::Info),
        "warn" => Ok(Severity::Warn),
        "error" => Ok(Severity::Error),
        "critical" => Ok(Severity::Critical),
        _ => Err(UnknownSeverity(label)),
    }
}

pub struct RuleHost {
    pub collected: Vec<Advisory>,
    pub inputs_json: serde_json::Value,
}

impl RuleHost {
    #[must_use]
    pub fn new(inputs_json: serde_json::Value) -> Self {
        Self {
            collected: Vec::new(),
            inputs_json,
        }
    }
}

#[host]
impl RuleHost {
    /// Record one advisory: severity (info, warn, error, critical), title, diagnosis, suggested action, rule name.
    fn emit_advisory(
        &mut self,
        severity: Arc<str>,
        title: String,
        diagnosis: String,
        action: String,
        rule: String,
    ) -> Result<(), UnknownSeverity> {
        let severity = self::severity(severity)?;
        self.collected.push(Advisory::new(
            severity,
            title,
            diagnosis,
            action,
            rule,
            hanshi_domain::now(),
        ));
        Ok(())
    }

    /// The rule inputs of this evaluation, as JSON text.
    fn inputs_json(&self) -> String {
        self.inputs_json.to_string()
    }
}

pub const RULE_LIST: &str = "rules";

#[derive(Default)]
pub struct BlueRules {
    source: String,
    program: Mutex<Option<Program<RuleHost>>>,
}

impl BlueRules {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load_str(&mut self, src: &str) -> Result<(), CoreError> {
        let mut source = self.source.clone();
        if !source.is_empty() && !src.is_empty() {
            source.push('\n');
        }
        source.push_str(src);
        let mut host = RuleHost::new(serde_json::Value::Null);
        let program = Program::load(Entry::anonymous(&source), &mut host)?;
        *self
            .program
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(program);
        self.source = source;
        Ok(())
    }

    pub fn evaluate(&self, inputs_json: serde_json::Value) -> Result<Vec<Advisory>, CoreError> {
        let mut guard = self
            .program
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut host = RuleHost::new(inputs_json);
        let Some(program) = guard.as_mut() else {
            return Ok(host.collected);
        };
        if !program.defines(RULE_LIST) {
            return Ok(host.collected);
        }
        let names: Vec<String> = program.call(RULE_LIST, (), &mut host)?;
        for name in names {
            program.call::<Value>(&name, (), &mut host)?;
        }
        Ok(host.collected)
    }
}

#[cfg(test)]
mod tests {
    use blue_lang_host::Host;

    use super::*;

    const BUNDLE: &str = include_str!("../rules/heartbeat.b");

    #[test]
    fn empty_bundle_loads() {
        let mut rules = BlueRules::new();
        rules.load_str("").unwrap();
    }

    #[test]
    fn rules_primitives_registered() {
        let names: Vec<_> = RuleHost::SURFACE.iter().map(|f| f.name).collect();
        assert_eq!(names, ["emit_advisory", "inputs_json"]);
        assert_eq!(
            RuleHost::SURFACE[0].params,
            ["severity", "title", "diagnosis", "action", "rule"]
        );
    }

    #[test]
    fn top_level_forms_can_call_emit_advisory() {
        let mut rules = BlueRules::new();
        rules
            .load_str(r#"emit_advisory("warn", "at-load", "ran at load", "inspect", "load-rule")"#)
            .unwrap();
        let out = rules
            .evaluate(serde_json::Value::Null)
            .expect("evaluate ok");
        assert!(out.is_empty());
    }

    #[test]
    fn bad_severity_errors() {
        let mut rules = BlueRules::new();
        let err = rules
            .load_str(r#"emit_advisory("panic", "x", "y", "z", "r")"#)
            .unwrap_err();
        assert!(matches!(err, CoreError::Rule(_)));
        assert!(
            err.to_string().contains("unknown severity `panic`"),
            "{err}"
        );
    }

    #[test]
    fn a_bundle_runs_each_listed_rule_against_the_inputs() {
        let mut rules = BlueRules::new();
        rules.load_str(BUNDLE).unwrap();
        let quiet = rules
            .evaluate(serde_json::json!({"ink_low": false}))
            .unwrap();
        let names: Vec<_> = quiet.iter().map(|a| a.rule.as_str()).collect();
        assert_eq!(names, ["bundle-loaded"]);
        let low = rules
            .evaluate(serde_json::json!({"ink_low": true}))
            .unwrap();
        let fired: Vec<_> = low.iter().map(|a| (a.rule.as_str(), a.severity)).collect();
        assert_eq!(
            fired,
            [
                ("bundle-loaded", Severity::Info),
                ("ink-low", Severity::Warn)
            ]
        );
    }

    #[test]
    fn a_rule_naming_an_unknown_host_function_is_refused_at_load() {
        let mut rules = BlueRules::new();
        let err = rules
            .load_str("def r()\n  emit_alert(1)\nend\n")
            .unwrap_err();
        assert!(
            matches!(err, CoreError::Rule(blue_lang_host::Error::Load(_))),
            "{err}"
        );
    }
}
