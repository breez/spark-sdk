//! What happens to the usernames under a domain when the domain joins a group.
//!
//! A group has one username per pubkey and one pubkey per username. A username
//! under the joining domain therefore moves to the group unless the group
//! already has that pubkey or that username. What is registered in the group
//! always stays.

/// A username registered under the joining domain, with what the group has for
/// the same pubkey and for the same username.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoiningRegistration {
    pub name: String,
    pub pubkey: String,
    pub description: String,
    /// The username the pubkey has registered in the group.
    pub group_name: Option<String>,
    /// The pubkey the username is registered to in the group.
    pub group_owner: Option<String>,
    /// The pubkey the group holds the username for.
    pub group_holder: Option<String>,
}

/// A username held under the joining domain for the pubkey that released it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoiningHold {
    pub name: String,
    pub pubkey: String,
    /// Whether the username is registered or held in the group.
    pub in_group: bool,
}

/// What the server does with a username under the joining domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// It moves to the group as it is.
    Moved,
    /// Its registration ends and the group holds it for its pubkey.
    Released,
    /// It is removed.
    Dropped,
}

impl Outcome {
    /// The outcome as `merged_names` stores it.
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Moved => "moved",
            Outcome::Released => "released",
            Outcome::Dropped => "dropped",
        }
    }
}

/// How many usernames a merge moved to the group, released for their pubkey
/// and dropped.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MergedNames {
    pub moved: usize,
    pub released: usize,
    pub dropped: usize,
}

impl MergedNames {
    pub fn count(outcomes: &[Outcome]) -> Self {
        let of = |outcome| outcomes.iter().filter(|o| **o == outcome).count();
        Self {
            moved: of(Outcome::Moved),
            released: of(Outcome::Released),
            dropped: of(Outcome::Dropped),
        }
    }
}

impl JoiningRegistration {
    pub fn outcome(&self) -> Outcome {
        let Some(group_name) = &self.group_name else {
            // The group's registration stays. Payers of this address pay the
            // group's pubkey from now on, and this pubkey is left without one.
            if self.group_owner.is_some() {
                return Outcome::Dropped;
            }
            return Outcome::Moved;
        };
        // The pubkey keeps the username it has in the group, so it gives this
        // one up. The group holds it for the pubkey unless the group has the
        // username already.
        let in_group = self.group_owner.is_some() || self.group_holder.is_some();
        if *group_name == self.name || in_group {
            return Outcome::Dropped;
        }
        Outcome::Released
    }

    /// The pubkey whose hold in the group ends because this registration moves
    /// to the group: a registration outranks a hold.
    pub fn dropped_hold(&self) -> Option<&str> {
        match self.outcome() {
            Outcome::Moved => self.group_holder.as_deref(),
            Outcome::Released | Outcome::Dropped => None,
        }
    }
}

impl JoiningHold {
    pub fn outcome(&self) -> Outcome {
        if self.in_group {
            return Outcome::Dropped;
        }
        Outcome::Moved
    }
}

#[cfg(test)]
mod tests {
    use super::{JoiningHold, JoiningRegistration, Outcome};

    /// `pubkey` registered `name` under the joining domain, and the group has
    /// neither.
    fn registration(pubkey: &str, name: &str) -> JoiningRegistration {
        JoiningRegistration {
            name: name.to_string(),
            pubkey: pubkey.to_string(),
            description: String::new(),
            group_name: None,
            group_owner: None,
            group_holder: None,
        }
    }

    #[test]
    fn a_registration_the_group_lacks_moves() {
        let alice = registration("aaaa", "alice");
        assert_eq!(alice.outcome(), Outcome::Moved);
        assert_eq!(alice.dropped_hold(), None);
    }

    #[test]
    fn a_pubkey_with_the_same_username_in_the_group_keeps_the_groups_one() {
        let alice = JoiningRegistration {
            group_name: Some("alice".to_string()),
            group_owner: Some("aaaa".to_string()),
            ..registration("aaaa", "alice")
        };
        assert_eq!(alice.outcome(), Outcome::Dropped);
    }

    #[test]
    fn a_pubkey_with_another_username_in_the_group_releases_this_one() {
        let alice = JoiningRegistration {
            group_name: Some("bob".to_string()),
            ..registration("aaaa", "alice")
        };
        assert_eq!(alice.outcome(), Outcome::Released);
        assert_eq!(alice.dropped_hold(), None);
    }

    #[test]
    fn a_released_username_the_group_already_has_is_dropped() {
        let registered_to_another = JoiningRegistration {
            group_name: Some("bob".to_string()),
            group_owner: Some("bbbb".to_string()),
            ..registration("aaaa", "alice")
        };
        assert_eq!(registered_to_another.outcome(), Outcome::Dropped);

        let held_for_another = JoiningRegistration {
            group_name: Some("bob".to_string()),
            group_holder: Some("bbbb".to_string()),
            ..registration("aaaa", "alice")
        };
        assert_eq!(held_for_another.outcome(), Outcome::Dropped);
        assert_eq!(
            held_for_another.dropped_hold(),
            None,
            "the group's hold stays"
        );
    }

    #[test]
    fn a_username_registered_to_another_pubkey_in_the_group_is_dropped() {
        let alice = JoiningRegistration {
            group_owner: Some("bbbb".to_string()),
            ..registration("aaaa", "alice")
        };
        assert_eq!(alice.outcome(), Outcome::Dropped);
    }

    #[test]
    fn a_registration_moves_over_a_hold_in_the_group() {
        let held_for_another = JoiningRegistration {
            group_holder: Some("bbbb".to_string()),
            ..registration("aaaa", "alice")
        };
        assert_eq!(held_for_another.outcome(), Outcome::Moved);
        assert_eq!(held_for_another.dropped_hold(), Some("bbbb"));

        let held_for_itself = JoiningRegistration {
            group_holder: Some("aaaa".to_string()),
            ..registration("aaaa", "alice")
        };
        assert_eq!(held_for_itself.outcome(), Outcome::Moved);
        assert_eq!(held_for_itself.dropped_hold(), Some("aaaa"));
    }

    #[test]
    fn a_hold_moves_unless_the_group_has_the_username() {
        let hold = |in_group| JoiningHold {
            name: "alice".to_string(),
            pubkey: "aaaa".to_string(),
            in_group,
        };
        assert_eq!(hold(false).outcome(), Outcome::Moved);
        assert_eq!(hold(true).outcome(), Outcome::Dropped);
    }
}
