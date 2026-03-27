pub fn normalize_cron(expr: &str) -> String {
    if expr.split_whitespace().count() == 5 {
        format!("0 {}", expr)
    } else {
        expr.to_string()
    }
}
