/// Username validation: alphanumeric plus a limited set of special characters,
/// with dots allowed (but not leading, trailing, or consecutive). This is the
/// unquoted local-part from RFC 5322 without the quoted-string alternative that
/// would allow control characters.
pub const USERNAME_VALIDATION_REGEX: &str =
    "^[a-zA-Z0-9!#$%&'*+/=?^_`{|}~-]+(?:\\.[a-zA-Z0-9!#$%&'*+/=?^_`{|}~-]+)*$";

pub struct User {
    pub domain: String,
    pub pubkey: String,
    pub name: String,
    /// `None` when the user gave no description.
    pub description: Option<String>,
}

/// The description payers read for `name@domain`: the one its user gave, or
/// else `Pay to name@domain`.
pub fn shown_description(name: &str, domain: &str, description: Option<&str>) -> String {
    description.map_or_else(|| format!("Pay to {name}@{domain}"), str::to_string)
}

#[cfg(test)]
mod tests {
    use super::shown_description;

    #[test]
    fn a_user_without_a_description_is_shown_the_default() {
        assert_eq!(
            shown_description("alice", "a.com", None),
            "Pay to alice@a.com"
        );
    }

    #[test]
    fn a_description_the_user_gave_is_shown_as_it_is() {
        assert_eq!(shown_description("alice", "a.com", Some("Tips")), "Tips");
    }
}
