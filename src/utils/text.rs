pub fn normalize_cron(expr: &str) -> String {
    let fields: Vec<&str> = expr.split_whitespace().collect();

    let (sec, mut rest): (String, Vec<String>) = match fields.len() {
        5 => ("0".to_string(), fields.iter().map(|s| s.to_string()).collect()),
        6 => (
            fields[0].to_string(),
            fields[1..].iter().map(|s| s.to_string()).collect(),
        ),
        _ => return expr.to_string(),
    };

    if let Some(last) = rest.last_mut() {
        *last = convert_dow(last);
    }

    format!("{} {}", sec, rest.join(" "))
}

fn convert_dow(field: &str) -> String {
    field
        .split(',')
        .map(convert_dow_part)
        .collect::<Vec<_>>()
        .join(",")
}

fn convert_dow_part(part: &str) -> String {
    let (base, step) = match part.split_once('/') {
        Some((b, s)) => (b, Some(s)),
        None => (part, None),
    };

    let converted = if let Some((start, end)) = base.split_once('-') {
        match (start.parse::<u8>(), end.parse::<u8>()) {
            (Ok(a), Ok(b)) if a <= 7 && b <= 7 => {
                format!("{}-{}", (a % 7) + 1, (b % 7) + 1)
            }
            _ => base.to_string(),
        }
    } else if let Ok(n) = base.parse::<u8>() {
        if n <= 7 {
            ((n % 7) + 1).to_string()
        } else {
            base.to_string()
        }
    } else {
        base.to_string()
    };

    match step {
        Some(s) => format!("{}/{}", converted, s),
        None => converted,
    }
}
