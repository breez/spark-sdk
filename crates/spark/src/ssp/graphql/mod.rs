mod auth_provider;
mod client;
mod error;
mod fragments;
mod mutations;
mod queries;
pub(crate) mod types;

pub(crate) use client::GraphQLClient;
pub(crate) use error::GraphQLError;
pub use types::*;

/// Macro to automatically add Unknown variant to an enum and implement Default to return Unknown
#[macro_export]
macro_rules! default_unknown_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $($variant:ident),* $(,)?
        }
    ) => {
        $(#[$meta])*
        $vis enum $name {
            $($variant,)*
            /// Indicates a value that is not recognized or not yet defined
            Unknown,
        }

        impl Default for $name {
            fn default() -> Self {
                $name::Unknown
            }
        }
    };
}
