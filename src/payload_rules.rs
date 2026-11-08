//! Bounded, opt-in validation of original webhook JSON, before template rendering.
use crate::*;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub field: String,
    pub kind: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
}
pub fn path_valid(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 128
        && path.split('.').count() <= 8
        && path.split('.').all(|part| {
            !part.is_empty() && part.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
        })
}
pub fn lookup<'a>(payload: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.')
        .try_fold(payload, |value, key| value.get(key))
}
pub fn validate_rules(rules: &[Rule]) -> Result<()> {
    if rules.len() > 40 {
        return Err(anyhow!("Use at most 40 payload rules"));
    }
    let mut seen = std::collections::HashSet::new();
    for rule in rules {
        if !path_valid(&rule.field) || !seen.insert(&rule.field) {
            return Err(anyhow!("Use unique field paths such as suite or build.branch (maximum 128 characters and 8 levels)"));
        }
        if !matches!(
            rule.kind.as_str(),
            "string" | "integer" | "number" | "boolean" | "array" | "object" | "url"
        ) {
            return Err(anyhow!("Unknown payload field type"));
        }
        if (rule.min.is_some() || rule.max.is_some())
            && !matches!(rule.kind.as_str(), "integer" | "number")
            || rule.min.is_some_and(|n| !n.is_finite())
            || rule.max.is_some_and(|n| !n.is_finite())
            || rule.min.zip(rule.max).is_some_and(|(a, b)| a > b)
        {
            return Err(anyhow!(
                "Numeric limits must be finite and minimum cannot exceed maximum"
            ));
        }
    }
    Ok(())
}
pub fn check(rules: &[Rule], payload: &Value) -> std::collections::BTreeMap<String, String> {
    let mut errors = std::collections::BTreeMap::new();
    for rule in rules {
        let Some(value) = lookup(payload, &rule.field).filter(|v| !v.is_null()) else {
            if rule.required {
                errors.insert(rule.field.clone(), "Required field".into());
            }
            continue;
        };
        let valid = match rule.kind.as_str() {
            "string" => value.is_string(),
            "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
            "number" => value.is_number(),
            "boolean" => value.is_boolean(),
            "array" => value.is_array(),
            "object" => value.is_object(),
            "url" => value.as_str().is_some_and(|s| {
                !s.chars().any(|c| c.is_control() || c.is_whitespace())
                    && url::Url::parse(s).is_ok_and(|u| {
                        matches!(u.scheme(), "http" | "https")
                            && u.host_str().is_some()
                            && u.username().is_empty()
                            && u.password().is_none()
                    })
            }),
            _ => false,
        };
        let message = if !valid {
            Some(format!("Expected {}", rule.kind))
        } else if rule.min.zip(value.as_f64()).is_some_and(|(min, n)| n < min) {
            Some(format!("Minimum value is {}", rule.min.unwrap()))
        } else if rule.max.zip(value.as_f64()).is_some_and(|(max, n)| n > max) {
            Some(format!("Maximum value is {}", rule.max.unwrap()))
        } else {
            None
        };
        if let Some(message) = message {
            errors.insert(rule.field.clone(), message);
        }
    }
    errors
}
#[cfg(test)]
mod tests {
    use super::*;
    fn rule(field: &str, kind: &str, required: bool) -> Rule {
        Rule {
            field: field.into(),
            kind: kind.into(),
            required,
            min: None,
            max: None,
        }
    }
    #[test]
    fn original_json_types_and_nested_paths() {
        let rules = vec![
            rule("suite", "string", true),
            Rule {
                min: Some(0.),
                ..rule("failed", "integer", true)
            },
            rule("build.url", "url", false),
        ];
        assert!(check(&rules, &json!({"suite":"Tests","failed":0})).is_empty());
        assert_eq!(
            check(
                &rules,
                &json!({"failed":"2","build":{"url":"javascript:alert(1)"}})
            )
            .len(),
            3
        );
        assert!(check(&rules, &json!({"suite":"Tests","failed":-1})).contains_key("failed"));
        assert!(check(&rules, &json!({"suite":null,"failed":0})).contains_key("suite"));
    }
    #[test]
    fn bounded_schema() {
        assert!(validate_rules(&vec![rule("x", "string", true); 41]).is_err());
        assert!(validate_rules(&[rule("a..b", "string", true)]).is_err());
        assert!(validate_rules(&[rule("x", "script", true)]).is_err());
        assert!(validate_rules(&[Rule {
            min: Some(2.),
            max: Some(1.),
            ..rule("x", "integer", true)
        }])
        .is_err());
    }
}
