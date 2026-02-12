pub const USERNAME_VALIDATION_REGEX: &str = "^(?:[a-zA-Z0-9!#$%&'*+\\/=?^_`{|}~-]+(?:\\.[a-z0-9!#$%&'*+\\/=?^_`{|}~-]+)*|\"(?:[\x01-\x08\x0b\x0c\x0e-\x1f\x21\x23-\x5b\x5d-\x7f]|\\[\x01-\x09\x0b\x0c\x0e-\x7f])*\")$";

pub struct User {
    pub domain: String,
    pub pubkey: String,
    pub name: String,
    pub description: String,
    pub nostr_pubkey: Option<String>,
    /// When true, the server won't track invoice payments for this user (LUD-21 and NIP-57 disabled)
    pub no_invoice_paid_support: bool,
}
