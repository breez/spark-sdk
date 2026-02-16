use std::collections::HashMap;

use bitcoin::hashes::{Hash, sha256};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Simple semantic version for schema versioning
/// Supports only major.minor.patch format without pre-release or build metadata
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SchemaVersion {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl SchemaVersion {
    /// Create a new schema version
    pub const fn new(major: u64, minor: u64, patch: u64) -> Self {
        SchemaVersion {
            major,
            minor,
            patch,
        }
    }

    /// Returns true when this schema version can be handled by a client that supports up to
    /// `supported_max`. Compatibility is gated by major version only.
    #[must_use]
    pub const fn is_supported_by(&self, supported_max: &SchemaVersion) -> bool {
        self.major <= supported_max.major
    }
}

impl std::fmt::Display for SchemaVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl std::str::FromStr for SchemaVersion {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() != 3 {
            return Err(format!(
                "Invalid version format '{s}': expected major.minor.patch"
            ));
        }

        let major = parts[0]
            .parse()
            .map_err(|_| format!("Invalid major version: {}", parts[0]))?;
        let minor = parts[1]
            .parse()
            .map_err(|_| format!("Invalid minor version: {}", parts[1]))?;
        let patch = parts[2]
            .parse()
            .map_err(|_| format!("Invalid patch version: {}", parts[2]))?;

        Ok(SchemaVersion {
            major,
            minor,
            patch,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct RecordId {
    pub r#type: String,
    pub data_id: String,
}

impl RecordId {
    pub fn new(r#type: impl Into<String>, data_id: impl Into<String>) -> Self {
        RecordId {
            r#type: r#type.into(),
            data_id: data_id.into(),
        }
    }

    pub fn to_id_string(&self) -> String {
        let combined = format!("{}:{}", self.r#type, self.data_id);
        let hash = sha256::Hash::hash(combined.as_bytes());
        format!("{hash:x}")
    }
}

#[derive(Debug)]
pub struct RecordChangeRequest {
    pub id: RecordId,
    pub schema_version: SchemaVersion,
    pub updated_fields: HashMap<String, Value>,
}

impl From<&RecordChangeRequest> for UnversionedRecordChange {
    fn from(value: &RecordChangeRequest) -> Self {
        UnversionedRecordChange {
            id: value.id.clone(),
            schema_version: value.schema_version.clone(),
            updated_fields: value.updated_fields.clone(),
        }
    }
}

pub struct UnversionedRecordChange {
    pub id: RecordId,
    pub schema_version: SchemaVersion,
    pub updated_fields: HashMap<String, Value>,
}

pub struct OutgoingChange {
    pub change: RecordChange,
    pub parent: Option<Record>,
}

impl OutgoingChange {
    pub fn merge(self) -> Record {
        let parent_revision = self.parent.as_ref().map_or(0, |p| p.revision);
        let mut record = Record {
            id: self.change.id.clone(),
            revision: parent_revision,
            schema_version: self.change.schema_version.clone(),
            data: HashMap::new(),
        };

        if let Some(parent) = self.parent {
            for (k, v) in parent.data {
                record.data.insert(k, v);
            }
        }

        for (k, v) in &self.change.updated_fields {
            record.data.insert(k.clone(), v.clone());
        }

        record
    }
}

pub struct IncomingChange {
    /// The incoming record from remote.
    pub new_state: Record,

    /// The current already existing sync state for this record.
    pub old_state: Option<Record>,
    // Pending outgoing changes are changes that have been applied to the relational data store already, but not to the parent.
    // The incoming change will come _before_ these outgoing changes, so to do things perfectly these changes have to be rolled back
    // in reverse order, then the incoming change applied, then the outgoing changes reapplied in forward order.
    // commented out because unneeded at the moment.
    // pub pending_outgoing_changes: Vec<RecordChange>,
}

pub struct RecordChange {
    pub id: RecordId,
    pub schema_version: SchemaVersion,
    pub updated_fields: HashMap<String, Value>,
    /// Local queue id from pending outgoing storage.
    pub local_revision: u64,
}

#[derive(Clone, Debug)]
pub struct Record {
    pub id: RecordId,
    pub revision: u64,
    pub schema_version: SchemaVersion,
    pub data: HashMap<String, Value>,
}

impl Record {
    #[must_use]
    pub fn with_parent(&self, parent: Option<Record>) -> Record {
        let mut record = Record {
            id: self.id.clone(),
            revision: self.revision,
            schema_version: self.schema_version.clone(),
            data: HashMap::new(),
        };

        if let Some(parent) = parent {
            for (k, v) in parent.data {
                record.data.insert(k, v);
            }
        }

        for (k, v) in &self.data {
            record.data.insert(k.clone(), v.clone());
        }

        record
    }

    pub fn change_set(&self, parent: Option<Record>) -> OutgoingChange {
        let mut updated_fields = HashMap::new();

        if let Some(parent) = &parent {
            for (k, v) in &self.data {
                if let Some(parent_value) = parent.data.get(k) {
                    if parent_value != v {
                        updated_fields.insert(k.clone(), v.clone());
                    }
                } else {
                    updated_fields.insert(k.clone(), v.clone());
                }
            }
        } else {
            updated_fields.clone_from(&self.data);
        }

        OutgoingChange {
            change: RecordChange {
                id: self.id.clone(),
                schema_version: self.schema_version.clone(),
                updated_fields,
                local_revision: self.revision,
            },
            parent,
        }
    }
}

#[cfg(test)]
mod test {
    use std::collections::HashMap;

    use bitcoin::hashes::{Hash, sha256};
    use serde_json::json;

    use crate::sync::{
        OutgoingChange, Record, RecordChange, RecordChangeRequest, RecordId, SchemaVersion,
        UnversionedRecordChange,
    };

    #[test]
    fn test_record_id_creation() {
        // Test with string literals
        let record_id1 = RecordId::new("test", "123");
        assert_eq!(record_id1.r#type, "test");
        assert_eq!(record_id1.data_id, "123");

        // Test with String types
        let record_id2 = RecordId::new("test".to_string(), "123".to_string());
        assert_eq!(record_id2.r#type, "test");
        assert_eq!(record_id2.data_id, "123");

        // Test with mixed types
        let type_str = "another_type".to_string();
        let record_id3 = RecordId::new(type_str, "456");
        assert_eq!(record_id3.r#type, "another_type");
        assert_eq!(record_id3.data_id, "456");
    }

    #[test]
    fn test_record_id_to_id_string() {
        // This test already exists but we'll include it for completeness
        let record_id = RecordId::new("test", "123");
        let expected_hash = sha256::Hash::hash(format!("{}:{}", "test", "123").as_bytes());
        assert_eq!(record_id.to_id_string(), format!("{expected_hash:x}"));

        // Test with different values
        let record_id2 = RecordId::new("payment", "invoice123");
        let expected_hash2 =
            sha256::Hash::hash(format!("{}:{}", "payment", "invoice123").as_bytes());
        assert_eq!(record_id2.to_id_string(), format!("{expected_hash2:x}"));
    }

    #[test]
    fn test_record_change_request_to_unversioned_record_change() {
        // Create a simple record change request
        let id = RecordId::new("payment", "invoice123");
        let mut updated_fields = HashMap::new();
        updated_fields.insert("amount".to_string(), json!(1000));
        updated_fields.insert("status".to_string(), json!("pending"));

        let version = SchemaVersion::new(1, 0, 0);
        let request = RecordChangeRequest {
            id: id.clone(),
            schema_version: version.clone(),
            updated_fields: updated_fields.clone(),
        };

        // Convert to unversioned record change
        let unversioned: UnversionedRecordChange = (&request).into();

        // Verify conversion
        assert_eq!(unversioned.id.r#type, id.r#type);
        assert_eq!(unversioned.id.data_id, id.data_id);
        assert_eq!(unversioned.schema_version, version);
        assert_eq!(unversioned.updated_fields.get("amount"), Some(&json!(1000)));
        assert_eq!(
            unversioned.updated_fields.get("status"),
            Some(&json!("pending"))
        );
    }

    #[test]
    fn test_outgoing_change_merge_with_parent() {
        // Create a parent record with some data
        let id = RecordId::new("payment", "invoice123");
        let mut parent_data = HashMap::new();
        parent_data.insert("amount".to_string(), json!(1000));
        parent_data.insert("currency".to_string(), json!("BTC"));
        parent_data.insert("status".to_string(), json!("pending"));

        let parent = Record {
            id: id.clone(),
            revision: 1,
            schema_version: SchemaVersion::new(0, 2, 5),
            data: parent_data,
        };

        // Create a change that updates some fields and adds new ones
        let mut updated_fields = HashMap::new();
        updated_fields.insert("status".to_string(), json!("confirmed"));
        updated_fields.insert(
            "confirmation_time".to_string(),
            json!("2023-10-23T12:00:00Z"),
        );

        let change = RecordChange {
            id: id.clone(),
            schema_version: SchemaVersion::new(0, 2, 6),
            updated_fields,
            local_revision: 2,
        };

        let outgoing_change = OutgoingChange {
            change,
            parent: Some(parent),
        };

        // Merge the change with parent
        let merged = outgoing_change.merge();

        // Verify the merged record
        assert_eq!(merged.id.r#type, "payment");
        assert_eq!(merged.id.data_id, "invoice123");
        assert_eq!(merged.revision, 1);
        assert_eq!(merged.schema_version, SchemaVersion::new(0, 2, 6));

        // Check that parent data was preserved
        assert_eq!(merged.data.get("amount"), Some(&json!(1000)));
        assert_eq!(merged.data.get("currency"), Some(&json!("BTC")));

        // Check that updated fields were applied
        assert_eq!(merged.data.get("status"), Some(&json!("confirmed")));
        assert_eq!(
            merged.data.get("confirmation_time"),
            Some(&json!("2023-10-23T12:00:00Z"))
        );
    }

    #[test]
    fn test_outgoing_change_merge_without_parent() {
        // Create a change with some fields
        let id = RecordId::new("payment", "invoice123");
        let mut updated_fields = HashMap::new();
        updated_fields.insert("amount".to_string(), json!(500));
        updated_fields.insert("currency".to_string(), json!("BTC"));
        updated_fields.insert("status".to_string(), json!("pending"));

        let change = RecordChange {
            id: id.clone(),
            schema_version: SchemaVersion::new(0, 2, 6),
            updated_fields,
            local_revision: 1,
        };

        let outgoing_change = OutgoingChange {
            change,
            parent: None,
        };

        // Merge the change without a parent
        let merged = outgoing_change.merge();

        // Verify the merged record
        assert_eq!(merged.id.r#type, "payment");
        assert_eq!(merged.id.data_id, "invoice123");
        assert_eq!(merged.revision, 0);
        assert_eq!(merged.schema_version, SchemaVersion::new(0, 2, 6));

        // Check that fields were applied
        assert_eq!(merged.data.get("amount"), Some(&json!(500)));
        assert_eq!(merged.data.get("currency"), Some(&json!("BTC")));
        assert_eq!(merged.data.get("status"), Some(&json!("pending")));
    }

    #[test]
    fn test_record_with_parent() {
        // Create a parent record
        let id = RecordId::new("payment", "invoice123");
        let mut parent_data = HashMap::new();
        parent_data.insert("amount".to_string(), json!(1000));
        parent_data.insert("currency".to_string(), json!("BTC"));

        let parent = Record {
            id: id.clone(),
            revision: 1,
            schema_version: SchemaVersion::new(0, 2, 5),
            data: parent_data,
        };

        // Create a child record
        let mut child_data = HashMap::new();
        child_data.insert("status".to_string(), json!("confirmed"));
        child_data.insert("amount".to_string(), json!(1200)); // Override parent value

        let child = Record {
            id: id.clone(),
            revision: 2,
            schema_version: SchemaVersion::new(0, 2, 6),
            data: child_data,
        };

        // Combine child with parent
        let combined = child.with_parent(Some(parent));

        // Verify combined record
        assert_eq!(combined.id.r#type, "payment");
        assert_eq!(combined.id.data_id, "invoice123");
        assert_eq!(combined.revision, 2);
        assert_eq!(combined.schema_version, SchemaVersion::new(0, 2, 6));

        // Check that parent data was included
        assert_eq!(combined.data.get("currency"), Some(&json!("BTC")));

        // Check that child data overrides parent data
        assert_eq!(combined.data.get("amount"), Some(&json!(1200)));
        assert_eq!(combined.data.get("status"), Some(&json!("confirmed")));
    }

    #[test]
    fn test_record_with_parent_none() {
        // Create a record
        let id = RecordId::new("payment", "invoice123");
        let mut data = HashMap::new();
        data.insert("amount".to_string(), json!(1000));
        data.insert("currency".to_string(), json!("BTC"));

        let record = Record {
            id: id.clone(),
            revision: 1,
            schema_version: SchemaVersion::new(0, 2, 5),
            data: data.clone(),
        };

        // Call with_parent with None
        let result = record.with_parent(None);

        // Verify result is the same as original
        assert_eq!(result.id.r#type, "payment");
        assert_eq!(result.id.data_id, "invoice123");
        assert_eq!(result.revision, 1);
        assert_eq!(result.schema_version, SchemaVersion::new(0, 2, 5));

        // Check data is preserved
        assert_eq!(result.data.get("amount"), Some(&json!(1000)));
        assert_eq!(result.data.get("currency"), Some(&json!("BTC")));
    }

    #[test]
    fn test_record_change_set_with_parent() {
        // Create a parent record
        let id = RecordId::new("payment", "invoice123");
        let mut parent_data = HashMap::new();
        parent_data.insert("amount".to_string(), json!(1000));
        parent_data.insert("currency".to_string(), json!("BTC"));
        parent_data.insert("status".to_string(), json!("pending"));

        let parent = Record {
            id: id.clone(),
            revision: 1,
            schema_version: SchemaVersion::new(0, 2, 5),
            data: parent_data,
        };

        // Create a child record with some changes
        let mut child_data = HashMap::new();
        child_data.insert("amount".to_string(), json!(1000)); // Same as parent, no change
        child_data.insert("currency".to_string(), json!("BTC")); // Same as parent, no change
        child_data.insert("status".to_string(), json!("confirmed")); // Different from parent
        child_data.insert(
            "confirmation_time".to_string(),
            json!("2023-10-23T12:00:00Z"),
        ); // New field

        let child = Record {
            id: id.clone(),
            revision: 2,
            schema_version: SchemaVersion::new(0, 2, 6),
            data: child_data,
        };

        // Get the change set
        let change_set = child.change_set(Some(parent.clone()));

        // Verify change set contains only the changes
        assert_eq!(change_set.change.id.r#type, "payment");
        assert_eq!(change_set.change.id.data_id, "invoice123");
        assert_eq!(change_set.change.local_revision, 2);
        assert_eq!(
            change_set.change.schema_version,
            SchemaVersion::new(0, 2, 6)
        );

        // Only changed or new fields should be in updated_fields
        assert!(!change_set.change.updated_fields.contains_key("amount")); // Unchanged
        assert!(!change_set.change.updated_fields.contains_key("currency")); // Unchanged
        assert_eq!(
            change_set.change.updated_fields.get("status"),
            Some(&json!("confirmed"))
        ); // Changed
        assert_eq!(
            change_set.change.updated_fields.get("confirmation_time"),
            Some(&json!("2023-10-23T12:00:00Z"))
        ); // Added

        // Parent should be included in change set
        assert!(change_set.parent.is_some());
        let parent_in_change_set = change_set.parent.unwrap();
        assert_eq!(parent_in_change_set.revision, parent.revision);
    }

    #[test]
    fn test_record_change_set_without_parent() {
        // Create a record
        let id = RecordId::new("payment", "invoice123");
        let mut data = HashMap::new();
        data.insert("amount".to_string(), json!(1000));
        data.insert("currency".to_string(), json!("BTC"));
        data.insert("status".to_string(), json!("pending"));

        let record = Record {
            id: id.clone(),
            revision: 1,
            schema_version: SchemaVersion::new(0, 2, 5),
            data: data.clone(),
        };

        // Get change set without parent
        let change_set = record.change_set(None);

        // Verify change set contains all fields
        assert_eq!(change_set.change.id.r#type, "payment");
        assert_eq!(change_set.change.id.data_id, "invoice123");
        assert_eq!(change_set.change.local_revision, 1);
        assert_eq!(
            change_set.change.schema_version,
            SchemaVersion::new(0, 2, 5)
        );

        // All fields should be included
        assert_eq!(
            change_set.change.updated_fields.get("amount"),
            Some(&json!(1000))
        );
        assert_eq!(
            change_set.change.updated_fields.get("currency"),
            Some(&json!("BTC"))
        );
        assert_eq!(
            change_set.change.updated_fields.get("status"),
            Some(&json!("pending"))
        );

        // Parent should be None
        assert!(change_set.parent.is_none());
    }

    #[test]
    fn test_schema_version_new() {
        let v = SchemaVersion::new(1, 2, 3);
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 2);
        assert_eq!(v.patch, 3);
    }

    #[test]
    fn test_schema_version_to_string() {
        let v = SchemaVersion::new(1, 0, 0);
        assert_eq!(v.to_string(), "1.0.0");

        let v = SchemaVersion::new(0, 2, 5);
        assert_eq!(v.to_string(), "0.2.5");
    }

    #[test]
    fn test_schema_version_from_str() {
        let v: SchemaVersion = "1.0.0".parse().unwrap();
        assert_eq!(v, SchemaVersion::new(1, 0, 0));

        let v: SchemaVersion = "0.2.5".parse().unwrap();
        assert_eq!(v, SchemaVersion::new(0, 2, 5));
    }

    #[test]
    fn test_schema_version_from_str_errors() {
        assert!("1.0".parse::<SchemaVersion>().is_err());
        assert!("1.0.0.0".parse::<SchemaVersion>().is_err());
        assert!("a.b.c".parse::<SchemaVersion>().is_err());
        assert!("".parse::<SchemaVersion>().is_err());
    }

    #[test]
    fn test_schema_version_roundtrip() {
        let v = SchemaVersion::new(1, 2, 3);
        let s = v.to_string();
        let parsed: SchemaVersion = s.parse().unwrap();
        assert_eq!(parsed, v);
    }

    #[test]
    fn test_schema_version_is_supported_by_same_major() {
        let schema = SchemaVersion::new(1, 4, 2);
        let supported_max = SchemaVersion::new(1, 0, 0);
        assert!(schema.is_supported_by(&supported_max));
    }

    #[test]
    fn test_schema_version_is_supported_by_newer_major_is_unsupported() {
        let schema = SchemaVersion::new(2, 0, 0);
        let supported_max = SchemaVersion::new(1, 9, 9);
        assert!(!schema.is_supported_by(&supported_max));
    }
}
