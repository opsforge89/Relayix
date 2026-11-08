//! JSON-safe webhook body substitution, without evaluating expressions or code.
use crate::*;

fn variable(name: &str, title: &str, message: &str, priority: i32) -> Result<Value> {
    Ok(match name.trim() {
        "title" => json!(title),
        "message" => json!(message),
        "priority" => json!(priority),
        "source" => json!("relayix"),
        _ => return Err(anyhow!("Unknown webhook body variable; use title, message, priority or source")),
    })
}
fn append(output: &mut String, text: &str) -> Result<()> {
    if output.len().saturating_add(text.len()) > 65536 {
        return Err(anyhow!("Rendered webhook body exceeds 64 KiB"));
    }
    output.push_str(text);
    Ok(())
}
fn string(text: &str, title: &str, message: &str, priority: i32) -> Result<Value> {
    if let Some(name) = text.strip_prefix("{{").and_then(|v| v.strip_suffix("}}")) {
        if !name.contains("{{") && !name.contains("}}") {
            return variable(name, title, message, priority);
        }
    }
    let mut output = String::new();
    let mut rest = text;
    while let Some((before, after)) = rest.split_once("{{") {
        append(&mut output, before)?;
        let (name, tail) = after.split_once("}}").ok_or_else(|| anyhow!("Unclosed webhook body variable"))?;
        let value = variable(name, title, message, priority)?;
        match value {
            Value::String(value) => append(&mut output, &value)?,
            value => append(&mut output, &value.to_string())?,
        }
        rest = tail;
    }
    append(&mut output, rest)?;
    Ok(json!(output))
}
fn visit(value: &Value, title: &str, message: &str, priority: i32, depth: usize, budget: &mut usize) -> Result<Value> {
    if depth > 16 {
        return Err(anyhow!("Webhook body template is nested too deeply"));
    }
    let result = match value {
        Value::String(text) => string(text, title, message, priority)?,
        Value::Array(values) => Value::Array(values.iter().map(|v| visit(v, title, message, priority, depth + 1, budget)).collect::<Result<_>>()?),
        Value::Object(values) => Value::Object(values.iter().map(|(key, value)| Ok((key.clone(), visit(value, title, message, priority, depth + 1, budget)?))).collect::<Result<_>>()?),
        value => value.clone(),
    };
    let cost = match &result {
        Value::String(text) => text.len(),
        Value::Object(values) => values.keys().map(String::len).sum(),
        _ => 1,
    };
    *budget = budget.checked_sub(cost).ok_or_else(|| anyhow!("Rendered webhook body exceeds 64 KiB"))?;
    Ok(result)
}
pub fn validate(template: &Value) -> Result<()> {
    if !template.is_object() || serde_json::to_vec(template)?.len() > 16384 {
        return Err(anyhow!("Webhook body template must be a JSON object of at most 16 KiB"));
    }
    visit(template, "", "", 3, 0, &mut 65536)?;
    Ok(())
}
pub fn render(template: Option<&Value>, title: &str, message: &str, priority: i32) -> Result<Value> {
    let result = match template.filter(|v| !v.is_null()) {
        Some(value) => {
            validate(value)?;
            visit(value, title, message, priority, 0, &mut 65536)?
        }
        None => json!({"title":title,"message":message,"priority":priority,"source":"relayix"}),
    };
    if serde_json::to_vec(&result)?.len() > 65536 {
        return Err(anyhow!("Rendered webhook body exceeds 64 KiB"));
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn templates_preserve_json_types_and_escape_payloads() {
        let message = "quote: \" newline:\n {{source}}";
        let template = json!({"text":"{{message}}","caption":"Title: {{title}}","priority":"{{priority}}","items":["{{source}}",true]});
        let body = render(Some(&template), "Build", message, 5).unwrap();
        assert_eq!(body["text"], message);
        assert_eq!(body["priority"], 5);
        assert_eq!(body["caption"], "Title: Build");
        assert_eq!(body["items"], json!(["relayix",true]));
        assert!(validate(&json!({"text":"{{unknown}}"})).is_err());
        assert!(validate(&json!({"text":"{{message"})).is_err());
        assert!(validate(&json!("raw body")).is_err());
        assert_eq!(render(None, "t", "m", 3).unwrap()["message"], "m");
    }
}
