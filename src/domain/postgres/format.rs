#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostgresDumpFormat {
    Fc,
    Fd,
}

impl PostgresDumpFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            PostgresDumpFormat::Fc => "fc",
            PostgresDumpFormat::Fd => "fd",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "fc" => Some(PostgresDumpFormat::Fc),
            "fd" => Some(PostgresDumpFormat::Fd),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PostgresDumpFormat;

    #[test]
    fn round_trips_through_as_str_and_from_str() {
        assert_eq!(PostgresDumpFormat::from_str(PostgresDumpFormat::Fc.as_str()), Some(PostgresDumpFormat::Fc));
        assert_eq!(PostgresDumpFormat::from_str(PostgresDumpFormat::Fd.as_str()), Some(PostgresDumpFormat::Fd));
    }

    #[test]
    fn from_str_rejects_unknown_value() {
        assert_eq!(PostgresDumpFormat::from_str("plain"), None);
    }
}
