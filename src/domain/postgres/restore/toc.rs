pub(crate) fn toc_creates_public_schema(toc: &str) -> bool {
    toc.lines().any(|l| {
        l.split(" SCHEMA - ")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            == Some("public")
    })
}
