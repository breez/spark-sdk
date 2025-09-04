use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use super::schema::users;

#[derive(
    QueryableByName, Queryable, AsChangeset, Serialize, Deserialize, Debug, Clone, PartialEq, Insertable
)]
#[diesel(table_name = users)]
pub struct User {
    pub pubkey: String,
    pub name: String,
}
