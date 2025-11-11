// Tests for derive_from and derive_into macros

// Test structs with named fields
mod source {
    pub struct SimpleStruct {
        pub field1: String,
        pub field2: i32,
    }

    pub struct StructWithOption {
        pub field1: Option<String>,
        pub field2: i32,
    }

    pub struct StructWithVec {
        pub field1: Vec<String>,
        pub field2: i32,
    }

    pub struct StructWithOptionVec {
        pub field1: Option<Vec<String>>,
        pub field2: i32,
    }

    pub struct TupleStruct(pub String, pub i32);

    pub struct UnitStruct;

    pub enum SimpleEnum {
        Variant1,
        Variant2,
    }

    pub enum EnumWithData {
        Tuple(String, i32),
        Struct { field1: String, field2: i32 },
        Unit,
    }

    pub enum EnumWithContainers {
        Option(Option<String>),
        Vec(Vec<String>),
        OptionVec(Option<Vec<String>>),
        Named { opt: Option<String>, vec: Vec<i32> },
    }

    // Nested struct types for testing manual From implementations
    #[derive(Debug, Clone, PartialEq)]
    pub struct InnerData {
        pub value: String,
        pub count: i32,
    }

    pub struct StructWithNested {
        pub name: String,
        pub inner: InnerData,
    }

    pub struct StructWithNestedOption {
        pub name: String,
        pub inner: Option<InnerData>,
    }

    pub struct StructWithNestedVec {
        pub items: Vec<InnerData>,
    }

    pub enum EnumWithNested {
        Simple(InnerData),
        Complex { data: InnerData, flag: bool },
    }
}

mod target_from {
    use super::source;

    #[macros::derive_from(source::SimpleStruct)]
    pub struct SimpleStruct {
        pub field1: String,
        pub field2: i32,
    }

    #[macros::derive_from(source::StructWithOption)]
    pub struct StructWithOption {
        pub field1: Option<String>,
        pub field2: i32,
    }

    #[macros::derive_from(source::StructWithVec)]
    pub struct StructWithVec {
        pub field1: Vec<String>,
        pub field2: i32,
    }

    #[macros::derive_from(source::StructWithOptionVec)]
    pub struct StructWithOptionVec {
        pub field1: Option<Vec<String>>,
        pub field2: i32,
    }

    #[macros::derive_from(source::TupleStruct)]
    pub struct TupleStruct(pub String, pub i32);

    #[macros::derive_from(source::UnitStruct)]
    pub struct UnitStruct;

    #[macros::derive_from(source::SimpleEnum)]
    pub enum SimpleEnum {
        Variant1,
        Variant2,
    }

    #[macros::derive_from(source::EnumWithData)]
    pub enum EnumWithData {
        Tuple(String, i32),
        Struct { field1: String, field2: i32 },
        Unit,
    }

    #[macros::derive_from(source::EnumWithContainers)]
    pub enum EnumWithContainers {
        Option(Option<String>),
        Vec(Vec<String>),
        OptionVec(Option<Vec<String>>),
        Named { opt: Option<String>, vec: Vec<i32> },
    }

    // Target type for nested conversions with manual From implementation
    #[derive(Debug, Clone, PartialEq)]
    pub struct InnerData {
        pub value: String,
        pub count: i32,
    }

    // Manual From implementation for InnerData
    impl From<source::InnerData> for InnerData {
        fn from(source: source::InnerData) -> Self {
            Self {
                value: source.value.to_uppercase(), // Transform the data
                count: source.count * 2,            // Transform the data
            }
        }
    }

    #[macros::derive_from(source::StructWithNested)]
    pub struct StructWithNested {
        pub name: String,
        pub inner: InnerData,
    }

    #[macros::derive_from(source::StructWithNestedOption)]
    pub struct StructWithNestedOption {
        pub name: String,
        pub inner: Option<InnerData>,
    }

    #[macros::derive_from(source::StructWithNestedVec)]
    pub struct StructWithNestedVec {
        pub items: Vec<InnerData>,
    }

    #[macros::derive_from(source::EnumWithNested)]
    pub enum EnumWithNested {
        Simple(InnerData),
        Complex { data: InnerData, flag: bool },
    }
}

mod target_into {
    use super::source;

    #[macros::derive_into(source::SimpleStruct)]
    pub struct SimpleStruct {
        pub field1: String,
        pub field2: i32,
    }

    #[macros::derive_into(source::StructWithOption)]
    pub struct StructWithOption {
        pub field1: Option<String>,
        pub field2: i32,
    }

    #[macros::derive_into(source::StructWithVec)]
    pub struct StructWithVec {
        pub field1: Vec<String>,
        pub field2: i32,
    }

    #[macros::derive_into(source::StructWithOptionVec)]
    pub struct StructWithOptionVec {
        pub field1: Option<Vec<String>>,
        pub field2: i32,
    }

    #[macros::derive_into(source::TupleStruct)]
    pub struct TupleStruct(pub String, pub i32);

    #[macros::derive_into(source::UnitStruct)]
    pub struct UnitStruct;

    #[macros::derive_into(source::SimpleEnum)]
    pub enum SimpleEnum {
        Variant1,
        Variant2,
    }

    #[macros::derive_into(source::EnumWithData)]
    pub enum EnumWithData {
        Tuple(String, i32),
        Struct { field1: String, field2: i32 },
        Unit,
    }

    #[macros::derive_into(source::EnumWithContainers)]
    pub enum EnumWithContainers {
        Option(Option<String>),
        Vec(Vec<String>),
        OptionVec(Option<Vec<String>>),
        Named { opt: Option<String>, vec: Vec<i32> },
    }

    // Target type for nested conversions with manual From implementation
    #[derive(Debug, Clone, PartialEq)]
    pub struct InnerData {
        pub value: String,
        pub count: i32,
    }

    // Manual From implementation for InnerData (reverse direction)
    impl From<InnerData> for source::InnerData {
        fn from(target: InnerData) -> Self {
            Self {
                value: target.value.to_lowercase(), // Transform back
                count: target.count / 2,            // Transform back
            }
        }
    }

    #[macros::derive_into(source::StructWithNested)]
    pub struct StructWithNested {
        pub name: String,
        pub inner: InnerData,
    }

    #[macros::derive_into(source::StructWithNestedOption)]
    pub struct StructWithNestedOption {
        pub name: String,
        pub inner: Option<InnerData>,
    }

    #[macros::derive_into(source::StructWithNestedVec)]
    pub struct StructWithNestedVec {
        pub items: Vec<InnerData>,
    }

    #[macros::derive_into(source::EnumWithNested)]
    pub enum EnumWithNested {
        Simple(InnerData),
        Complex { data: InnerData, flag: bool },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // derive_from tests - converting FROM source TO target
    #[test]
    fn test_derive_from_simple_struct() {
        let source = source::SimpleStruct {
            field1: "test".to_string(),
            field2: 42,
        };
        let target: target_from::SimpleStruct = source.into();
        assert_eq!(target.field1, "test");
        assert_eq!(target.field2, 42);
    }

    #[test]
    fn test_derive_from_struct_with_option() {
        let source = source::StructWithOption {
            field1: Some("test".to_string()),
            field2: 42,
        };
        let target: target_from::StructWithOption = source.into();
        assert_eq!(target.field1, Some("test".to_string()));
        assert_eq!(target.field2, 42);

        let source_none = source::StructWithOption {
            field1: None,
            field2: 42,
        };
        let target_none: target_from::StructWithOption = source_none.into();
        assert_eq!(target_none.field1, None);
    }

    #[test]
    fn test_derive_from_struct_with_vec() {
        let source = source::StructWithVec {
            field1: vec!["a".to_string(), "b".to_string()],
            field2: 42,
        };
        let target: target_from::StructWithVec = source.into();
        assert_eq!(target.field1, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(target.field2, 42);
    }

    #[test]
    fn test_derive_from_struct_with_option_vec() {
        let source = source::StructWithOptionVec {
            field1: Some(vec!["a".to_string(), "b".to_string()]),
            field2: 42,
        };
        let target: target_from::StructWithOptionVec = source.into();
        assert_eq!(target.field1, Some(vec!["a".to_string(), "b".to_string()]));
        assert_eq!(target.field2, 42);
    }

    #[test]
    fn test_derive_from_tuple_struct() {
        let source = source::TupleStruct("test".to_string(), 42);
        let target: target_from::TupleStruct = source.into();
        assert_eq!(target.0, "test");
        assert_eq!(target.1, 42);
    }

    #[test]
    fn test_derive_from_unit_struct() {
        let source = source::UnitStruct;
        let _target: target_from::UnitStruct = source.into();
    }

    #[test]
    fn test_derive_from_simple_enum() {
        let source1 = source::SimpleEnum::Variant1;
        let target1: target_from::SimpleEnum = source1.into();
        assert!(matches!(target1, target_from::SimpleEnum::Variant1));

        let source2 = source::SimpleEnum::Variant2;
        let target2: target_from::SimpleEnum = source2.into();
        assert!(matches!(target2, target_from::SimpleEnum::Variant2));
    }

    #[test]
    fn test_derive_from_enum_with_data() {
        let source_tuple = source::EnumWithData::Tuple("test".to_string(), 42);
        let target_tuple: target_from::EnumWithData = source_tuple.into();
        match target_tuple {
            target_from::EnumWithData::Tuple(s, i) => {
                assert_eq!(s, "test");
                assert_eq!(i, 42);
            }
            _ => panic!("Wrong variant"),
        }

        let source_struct = source::EnumWithData::Struct {
            field1: "test".to_string(),
            field2: 42,
        };
        let target_struct: target_from::EnumWithData = source_struct.into();
        match target_struct {
            target_from::EnumWithData::Struct { field1, field2 } => {
                assert_eq!(field1, "test");
                assert_eq!(field2, 42);
            }
            _ => panic!("Wrong variant"),
        }

        let source_unit = source::EnumWithData::Unit;
        let target_unit: target_from::EnumWithData = source_unit.into();
        assert!(matches!(target_unit, target_from::EnumWithData::Unit));
    }

    #[test]
    fn test_derive_from_enum_with_containers() {
        let source = source::EnumWithContainers::Option(Some("test".to_string()));
        let target: target_from::EnumWithContainers = source.into();
        match target {
            target_from::EnumWithContainers::Option(opt) => {
                assert_eq!(opt, Some("test".to_string()));
            }
            _ => panic!("Wrong variant"),
        }

        let source_vec = source::EnumWithContainers::Vec(vec!["a".to_string()]);
        let target_vec: target_from::EnumWithContainers = source_vec.into();
        match target_vec {
            target_from::EnumWithContainers::Vec(vec) => {
                assert_eq!(vec, vec!["a".to_string()]);
            }
            _ => panic!("Wrong variant"),
        }

        let source_named = source::EnumWithContainers::Named {
            opt: Some("test".to_string()),
            vec: vec![1, 2, 3],
        };
        let target_named: target_from::EnumWithContainers = source_named.into();
        match target_named {
            target_from::EnumWithContainers::Named { opt, vec } => {
                assert_eq!(opt, Some("test".to_string()));
                assert_eq!(vec, vec![1, 2, 3]);
            }
            _ => panic!("Wrong variant"),
        }

        let source_opt_vec = source::EnumWithContainers::OptionVec(Some(vec!["x".to_string()]));
        let target_opt_vec: target_from::EnumWithContainers = source_opt_vec.into();
        match target_opt_vec {
            target_from::EnumWithContainers::OptionVec(opt_vec) => {
                assert_eq!(opt_vec, Some(vec!["x".to_string()]));
            }
            _ => panic!("Wrong variant"),
        }
    }

    // derive_into tests - converting FROM target TO source
    #[test]
    fn test_derive_into_simple_struct() {
        let target = target_into::SimpleStruct {
            field1: "test".to_string(),
            field2: 42,
        };
        let source: source::SimpleStruct = target.into();
        assert_eq!(source.field1, "test");
        assert_eq!(source.field2, 42);
    }

    #[test]
    fn test_derive_into_struct_with_option() {
        let target = target_into::StructWithOption {
            field1: Some("test".to_string()),
            field2: 42,
        };
        let source: source::StructWithOption = target.into();
        assert_eq!(source.field1, Some("test".to_string()));
        assert_eq!(source.field2, 42);
    }

    #[test]
    fn test_derive_into_struct_with_vec() {
        let target = target_into::StructWithVec {
            field1: vec!["a".to_string(), "b".to_string()],
            field2: 42,
        };
        let source: source::StructWithVec = target.into();
        assert_eq!(source.field1, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(source.field2, 42);
    }

    #[test]
    fn test_derive_into_struct_with_option_vec() {
        let target = target_into::StructWithOptionVec {
            field1: Some(vec!["a".to_string(), "b".to_string()]),
            field2: 42,
        };
        let source: source::StructWithOptionVec = target.into();
        assert_eq!(source.field1, Some(vec!["a".to_string(), "b".to_string()]));
        assert_eq!(source.field2, 42);
    }

    #[test]
    fn test_derive_into_tuple_struct() {
        let target = target_into::TupleStruct("test".to_string(), 42);
        let source: source::TupleStruct = target.into();
        assert_eq!(source.0, "test");
        assert_eq!(source.1, 42);
    }

    #[test]
    fn test_derive_into_unit_struct() {
        let target = target_into::UnitStruct;
        let _source: source::UnitStruct = target.into();
    }

    #[test]
    fn test_derive_into_simple_enum() {
        let target1 = target_into::SimpleEnum::Variant1;
        let source1: source::SimpleEnum = target1.into();
        assert!(matches!(source1, source::SimpleEnum::Variant1));

        let target2 = target_into::SimpleEnum::Variant2;
        let source2: source::SimpleEnum = target2.into();
        assert!(matches!(source2, source::SimpleEnum::Variant2));
    }

    #[test]
    fn test_derive_into_enum_with_data() {
        let target_tuple = target_into::EnumWithData::Tuple("test".to_string(), 42);
        let source_tuple: source::EnumWithData = target_tuple.into();
        match source_tuple {
            source::EnumWithData::Tuple(s, i) => {
                assert_eq!(s, "test");
                assert_eq!(i, 42);
            }
            _ => panic!("Wrong variant"),
        }

        let target_struct = target_into::EnumWithData::Struct {
            field1: "test".to_string(),
            field2: 42,
        };
        let source_struct: source::EnumWithData = target_struct.into();
        match source_struct {
            source::EnumWithData::Struct { field1, field2 } => {
                assert_eq!(field1, "test");
                assert_eq!(field2, 42);
            }
            _ => panic!("Wrong variant"),
        }

        let target_unit = target_into::EnumWithData::Unit;
        let source_unit: source::EnumWithData = target_unit.into();
        assert!(matches!(source_unit, source::EnumWithData::Unit));
    }

    #[test]
    fn test_derive_into_enum_with_containers() {
        let target = target_into::EnumWithContainers::Option(Some("test".to_string()));
        let source: source::EnumWithContainers = target.into();
        match source {
            source::EnumWithContainers::Option(opt) => {
                assert_eq!(opt, Some("test".to_string()));
            }
            _ => panic!("Wrong variant"),
        }

        let target_vec = target_into::EnumWithContainers::Vec(vec!["a".to_string()]);
        let source_vec: source::EnumWithContainers = target_vec.into();
        match source_vec {
            source::EnumWithContainers::Vec(vec) => {
                assert_eq!(vec, vec!["a".to_string()]);
            }
            _ => panic!("Wrong variant"),
        }

        let target_named = target_into::EnumWithContainers::Named {
            opt: Some("test".to_string()),
            vec: vec![1, 2, 3],
        };
        let source_named: source::EnumWithContainers = target_named.into();
        match source_named {
            source::EnumWithContainers::Named { opt, vec } => {
                assert_eq!(opt, Some("test".to_string()));
                assert_eq!(vec, vec![1, 2, 3]);
            }
            _ => panic!("Wrong variant"),
        }

        let target_opt_vec =
            target_into::EnumWithContainers::OptionVec(Some(vec!["x".to_string()]));
        let source_opt_vec: source::EnumWithContainers = target_opt_vec.into();
        match source_opt_vec {
            source::EnumWithContainers::OptionVec(opt_vec) => {
                assert_eq!(opt_vec, Some(vec!["x".to_string()]));
            }
            _ => panic!("Wrong variant"),
        }
    }

    // Tests with nested structs that have manual From implementations
    #[test]
    fn test_derive_from_struct_with_nested() {
        let source = source::StructWithNested {
            name: "outer".to_string(),
            inner: source::InnerData {
                value: "hello".to_string(),
                count: 5,
            },
        };
        let target: target_from::StructWithNested = source.into();
        assert_eq!(target.name, "outer");
        // Manual From impl transforms: uppercase and count * 2
        assert_eq!(target.inner.value, "HELLO");
        assert_eq!(target.inner.count, 10);
    }

    #[test]
    fn test_derive_from_struct_with_nested_option() {
        let source = source::StructWithNestedOption {
            name: "outer".to_string(),
            inner: Some(source::InnerData {
                value: "world".to_string(),
                count: 3,
            }),
        };
        let target: target_from::StructWithNestedOption = source.into();
        assert_eq!(target.name, "outer");
        assert!(target.inner.is_some());
        let inner = target.inner.unwrap();
        assert_eq!(inner.value, "WORLD");
        assert_eq!(inner.count, 6);

        // Test with None
        let source_none = source::StructWithNestedOption {
            name: "outer".to_string(),
            inner: None,
        };
        let target_none: target_from::StructWithNestedOption = source_none.into();
        assert!(target_none.inner.is_none());
    }

    #[test]
    fn test_derive_from_struct_with_nested_vec() {
        let source = source::StructWithNestedVec {
            items: vec![
                source::InnerData {
                    value: "first".to_string(),
                    count: 1,
                },
                source::InnerData {
                    value: "second".to_string(),
                    count: 2,
                },
            ],
        };
        let target: target_from::StructWithNestedVec = source.into();
        assert_eq!(target.items.len(), 2);
        assert_eq!(target.items[0].value, "FIRST");
        assert_eq!(target.items[0].count, 2);
        assert_eq!(target.items[1].value, "SECOND");
        assert_eq!(target.items[1].count, 4);
    }

    #[test]
    fn test_derive_from_enum_with_nested() {
        let source = source::EnumWithNested::Simple(source::InnerData {
            value: "test".to_string(),
            count: 7,
        });
        let target: target_from::EnumWithNested = source.into();
        match target {
            target_from::EnumWithNested::Simple(inner) => {
                assert_eq!(inner.value, "TEST");
                assert_eq!(inner.count, 14);
            }
            _ => panic!("Wrong variant"),
        }

        let source_complex = source::EnumWithNested::Complex {
            data: source::InnerData {
                value: "complex".to_string(),
                count: 4,
            },
            flag: true,
        };
        let target_complex: target_from::EnumWithNested = source_complex.into();
        match target_complex {
            target_from::EnumWithNested::Complex { data, flag } => {
                assert_eq!(data.value, "COMPLEX");
                assert_eq!(data.count, 8);
                assert!(flag);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_derive_into_struct_with_nested() {
        let target = target_into::StructWithNested {
            name: "outer".to_string(),
            inner: target_into::InnerData {
                value: "HELLO".to_string(),
                count: 10,
            },
        };
        let source: source::StructWithNested = target.into();
        assert_eq!(source.name, "outer");
        // Manual From impl transforms: lowercase and count / 2
        assert_eq!(source.inner.value, "hello");
        assert_eq!(source.inner.count, 5);
    }

    #[test]
    fn test_derive_into_struct_with_nested_option() {
        let target = target_into::StructWithNestedOption {
            name: "outer".to_string(),
            inner: Some(target_into::InnerData {
                value: "WORLD".to_string(),
                count: 6,
            }),
        };
        let source: source::StructWithNestedOption = target.into();
        assert_eq!(source.name, "outer");
        assert!(source.inner.is_some());
        let inner = source.inner.unwrap();
        assert_eq!(inner.value, "world");
        assert_eq!(inner.count, 3);
    }

    #[test]
    fn test_derive_into_struct_with_nested_vec() {
        let target = target_into::StructWithNestedVec {
            items: vec![
                target_into::InnerData {
                    value: "FIRST".to_string(),
                    count: 2,
                },
                target_into::InnerData {
                    value: "SECOND".to_string(),
                    count: 4,
                },
            ],
        };
        let source: source::StructWithNestedVec = target.into();
        assert_eq!(source.items.len(), 2);
        assert_eq!(source.items[0].value, "first");
        assert_eq!(source.items[0].count, 1);
        assert_eq!(source.items[1].value, "second");
        assert_eq!(source.items[1].count, 2);
    }

    #[test]
    fn test_derive_into_enum_with_nested() {
        let target = target_into::EnumWithNested::Simple(target_into::InnerData {
            value: "TEST".to_string(),
            count: 14,
        });
        let source: source::EnumWithNested = target.into();
        match source {
            source::EnumWithNested::Simple(inner) => {
                assert_eq!(inner.value, "test");
                assert_eq!(inner.count, 7);
            }
            _ => panic!("Wrong variant"),
        }

        let target_complex = target_into::EnumWithNested::Complex {
            data: target_into::InnerData {
                value: "COMPLEX".to_string(),
                count: 8,
            },
            flag: true,
        };
        let source_complex: source::EnumWithNested = target_complex.into();
        match source_complex {
            source::EnumWithNested::Complex { data, flag } => {
                assert_eq!(data.value, "complex");
                assert_eq!(data.count, 4);
                assert!(flag);
            }
            _ => panic!("Wrong variant"),
        }
    }
}
