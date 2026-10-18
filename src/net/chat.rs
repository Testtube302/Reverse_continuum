//! Continuum chat prefixes (nullspace ChatController / SS.NET Chat).
#[derive(Debug, PartialEq)]
pub struct Route<'a> {
    pub kind: u8,
    pub target: u16,
    pub text: &'a str,
}

pub fn route<'a>(text: &'a str, selected: Option<u16>, find: impl Fn(&str) -> Option<u16>) -> Result<Route<'a>, &'static str> {
    let text = text.split('\0').next().unwrap_or("");
    let (kind, target, body) = if let Some(body) = text.strip_prefix("//").or_else(|| text.strip_prefix('\'')) {
        (3, 0, body)
    } else if let Some(body) = text.strip_prefix('/') {
        (5, selected.ok_or("Select a player for private chat first.")?, body)
    } else if let Some(body) = text.strip_prefix('"') {
        (4, selected.ok_or("Select a player for team chat first.")?, body)
    } else if let Some(rest) = text.strip_prefix(':') {
        let (name, body) = rest.split_once(':').ok_or("Use :name:message for private chat.")?;
        if name.is_empty() || name.len() > 20 || body.is_empty() { return Err("Private chat needs a name and message."); }
        match find(name) {
            Some(pid) => (5, pid, body),
            None => (7, 0, text),
        }
    } else if let Some(body) = text.strip_prefix(';') {
        (9, 0, body)
    } else {
        (2, 0, text)
    };
    if body.is_empty() { return Err("Message is empty."); }
    Ok(Route { kind, target, text: body })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prefixes_and_missing_targets() {
        for (input, kind, target, body) in [
            ("hello", 2, 0, "hello"), ("?help", 2, 0, "?help"),
            ("//team", 3, 0, "team"), ("'team", 3, 0, "team"),
            ("/secret", 5, 42, "secret"), ("\"team", 4, 42, "team"),
            (":Local:secret", 5, 7, "secret"), (":Away:secret", 7, 0, ":Away:secret"),
            (";hello", 9, 0, "hello"), (";2;hello", 9, 0, "2;hello"),
        ] {
            assert_eq!(route(input, Some(42), |n| n.eq_ignore_ascii_case("local").then_some(7)), Ok(Route {kind, target, text: body}));
        }
        for input in ["", ":", ":name:", "::secret", "'", "//", ";", "/secret", "\"secret"] {
            assert!(route(input, None, |_| None).is_err(), "{input}");
        }
        assert_eq!(route("hello\0hidden", None, |_| None).unwrap().text, "hello");
    }
}
