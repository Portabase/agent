use serde::{Deserialize, Deserializer};
use toml::Value;

pub fn deserialize_snake_case<'de, D>(deserializer: D) -> Result<Value, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    Ok(to_snake_case(value))
}
fn to_snake_case(value: Value) -> Value {
    match value {
        Value::Table(table) => Value::Table(
            table
                .into_iter()
                .map(|(k, v)| (camel_to_snake(&k), to_snake_case(v)))
                .collect(),
        ),
        Value::Array(arr) => {
            Value::Array(arr.into_iter().map(to_snake_case).collect())
        }
        other => other,
    }
}

fn camel_to_snake(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}
