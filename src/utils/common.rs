use serde::Serialize;
use serde_json::Value;

#[derive(Clone, Copy)]
pub enum BackupMethod {
    Automatic,
    Manual,
}

impl ToString for BackupMethod {
    fn to_string(&self) -> String {
        match self {
            BackupMethod::Automatic => "automatic".into(),
            BackupMethod::Manual => "manual".into(),
        }
    }
}


pub fn vec_to_option_json<T: Serialize>(v: Vec<T>) -> Option<Value> {
    if v.is_empty() {
        None
    } else {
        Some(serde_json::to_value(v).expect("serialization failed"))
    }
}