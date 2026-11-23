pub mod crypto;
pub mod send;

/// The badge count from `TWYDO_TODAY_OPEN`: a plain decimal, or none.
pub fn badge_from_env(v: Option<&str>) -> Option<u64> {
    v.filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badge_env_parses() {
        assert_eq!(badge_from_env(Some("7")), Some(7));
        assert_eq!(badge_from_env(Some("0")), Some(0));
    }

    #[test]
    fn badge_env_garbage_is_omitted() {
        for v in [None, Some(""), Some("x"), Some("-1"), Some(" 3")] {
            assert_eq!(badge_from_env(v), None, "{v:?}");
        }
    }
}
