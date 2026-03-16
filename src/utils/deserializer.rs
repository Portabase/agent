use serde::{Deserialize, Deserializer};
use toml::Value;
use serde_json::{Value as ValueJson, };


pub fn deserialize_snake_case<'de, D>(deserializer: D) -> Result<Value, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    Ok(to_snake_case(value))
}

pub fn to_snake_case(value: Value) -> Value {
    match value {
        Value::Table(table) => Value::Table(
            table
                .into_iter()
                .map(|(k, v)| (camel_to_snake(&k), to_snake_case(v)))
                .collect(),
        ),
        Value::Array(arr) => Value::Array(arr.into_iter().map(to_snake_case).collect()),
        other => other,
    }
}

pub fn camel_to_snake(s: &str) -> String {
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


pub fn string_or_number_to_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<ValueJson>::deserialize(deserializer)?;

    match value {
        Some(ValueJson::String(s)) => Ok(Some(s)),
        Some(ValueJson::Number(n)) => Ok(Some(n.to_string())),
        Some(_) => Err(serde::de::Error::custom("port must be string or number")),
        None => Ok(None),
    }
}
