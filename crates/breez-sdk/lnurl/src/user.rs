use crate::domains::DomainGroup;

/// Username validation: alphanumeric plus a limited set of special characters,
/// with dots allowed (but not leading, trailing, or consecutive). This is the
/// unquoted local-part from RFC 5322 without the quoted-string alternative that
/// would allow control characters.
pub const USERNAME_VALIDATION_REGEX: &str =
    "^[a-zA-Z0-9!#$%&'*+/=?^_`{|}~-]+(?:\\.[a-zA-Z0-9!#$%&'*+/=?^_`{|}~-]+)*$";

pub struct User {
    pub group: DomainGroup,
    pub pubkey: String,
    pub name: String,
    pub description: String,
}

/// The description the server writes for a user who stored none.
pub fn default_description(name: &str, domain: &str) -> String {
    format!("Pay to {name}@{domain}")
}

/// The description the server shows on `domain` for the user `name`, who
/// stored `description`.
///
/// A user who stored nothing gets the default. So does a user who stored the
/// default for one of `group_domains`: a wallet that writes the default itself
/// names the domain it registered on, which is not always `domain`.
pub fn shown_description<'a>(
    name: &str,
    description: &str,
    domain: &str,
    group_domains: impl IntoIterator<Item = &'a str>,
) -> String {
    let wrote_one = !description.is_empty()
        && !group_domains
            .into_iter()
            .any(|group_domain| description == default_description(name, group_domain));
    if wrote_one {
        description.to_string()
    } else {
        default_description(name, domain)
    }
}

#[cfg(test)]
mod tests {
    use super::shown_description;

    const GROUP_DOMAINS: [&str; 2] = ["a.com", "b.com"];

    #[test]
    fn a_user_who_stored_no_description_gets_the_default_for_the_domain() {
        assert_eq!(
            shown_description("alice", "", "b.com", GROUP_DOMAINS),
            "Pay to alice@b.com"
        );
    }

    #[test]
    fn a_default_a_wallet_wrote_names_the_domain_it_is_shown_on() {
        assert_eq!(
            shown_description("alice", "Pay to alice@a.com", "b.com", GROUP_DOMAINS),
            "Pay to alice@b.com"
        );
        assert_eq!(
            shown_description("alice", "Pay to alice@a.com", "a.com", GROUP_DOMAINS),
            "Pay to alice@a.com"
        );
    }

    #[test]
    fn a_description_the_user_wrote_is_shown_as_stored() {
        for own in [
            "Tips for alice",
            // The default for a domain outside the group, or for another user.
            "Pay to alice@c.com",
            "Pay to bob@a.com",
        ] {
            assert_eq!(shown_description("alice", own, "b.com", GROUP_DOMAINS), own);
        }
    }
}
