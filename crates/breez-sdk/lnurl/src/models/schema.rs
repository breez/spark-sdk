diesel::table! {
    users (pubkey) {
        #[max_length = 66]
        pubkey -> VarChar,
        #[max_length = 64]
        name -> VarChar,
    }
}