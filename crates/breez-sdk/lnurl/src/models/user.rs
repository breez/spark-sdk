use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use super::schema::users;

pub const USERNAME_VALIDATION_REGEX: &str = "^(?:[a-zA-Z0-9!#$%&'*+\\/=?^_`{|}~-]+(?:\\.[a-z0-9!#$%&'*+\\/=?^_`{|}~-]+)*|\"(?:[\x01-\x08\x0b\x0c\x0e-\x1f\x21\x23-\x5b\x5d-\x7f]|\\[\x01-\x09\x0b\x0c\x0e-\x7f])*\")$";

#[derive(
    QueryableByName,
    Queryable,
    AsChangeset,
    Serialize,
    Deserialize,
    Debug,
    Clone,
    PartialEq,
    Insertable,
)]
#[diesel(table_name = users)]
pub struct User {
    pub pubkey: String,
    pub name: String,
    pub description: String,
}
