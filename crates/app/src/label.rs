//! The templates that say what workspace items and tabs show (`[workspace_items]`,
//! `[tab_items]`): `{placeholder}`s filled in from the item. The view supplies the values
//! (`view::sidebar`, `view::tabs`), so the set of placeholders is defined there.

/// Fills each `{key}` in `template` that `value` knows; unknown ones stay as written, so a typo
/// shows. The result is trimmed of spaces and of separators left dangling by empty values.
pub fn fill(template: &str, mut value: impl FnMut(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let filled = after
            .find('}')
            .and_then(|end| Some((value(&after[..end])?, end)));
        match filled {
            Some((v, end)) => {
                out.push_str(&v);
                rest = &after[end + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out.trim_matches(|c: char| c.is_whitespace() || matches!(c, ',' | '·' | '•' | '|' | '-'))
        .to_owned()
}

/// A length of time as "45s", "2m 05s" or "1h 03m": a command's run time, how long an agent
/// has been in its state.
pub fn duration(elapsed: std::time::Duration) -> String {
    let s = elapsed.as_secs();
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m {:02}s", s / 60, s % 60),
        _ => format!("{}h {:02}m", s / 3600, s % 3600 / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::fill;

    fn values(key: &str) -> Option<String> {
        match key {
            "name" => Some("api".into()),
            "tabs" => Some("3".into()),
            "notice" => Some(String::new()),
            _ => None,
        }
    }

    #[test]
    fn fills_known_placeholders() {
        assert_eq!(fill("{name}: {tabs} tabs", values), "api: 3 tabs");
        assert_eq!(fill("plain", values), "plain");
        assert_eq!(fill("", values), "");
    }

    #[test]
    fn keeps_unknown_and_unclosed_ones() {
        assert_eq!(fill("{nmae} {name}", values), "{nmae} api");
        assert_eq!(fill("{name", values), "{name");
    }

    #[test]
    fn empty_values_leave_no_dangling_separators() {
        assert_eq!(fill("{tabs} tabs · {notice}", values), "3 tabs");
        assert_eq!(fill("{notice}", values), "");
    }
}
