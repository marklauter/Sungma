//! Resource and subjectset names, parsed at the edge.

use proptest::prelude::*;
use sungma_core::name::{NameError, ResourceName, SubjectsetName};

#[test]
fn a_subjectset_name_splits_at_the_first_colon_and_the_last_hash() {
    let name: SubjectsetName = "file:a:b#c#viewer".parse().unwrap();
    assert_eq!(name.resource().theory(), "file");
    assert_eq!(name.resource().id(), "a:b#c");
    assert_eq!(name.relation(), "viewer");
    assert_eq!(name.to_string(), "file:a:b#c#viewer");
}

#[test]
fn a_resource_name_splits_at_the_first_colon() {
    let name: ResourceName = "folder:a:b".parse().unwrap();
    assert_eq!(name.theory(), "folder");
    assert_eq!(name.id(), "a:b");
    assert_eq!(name.to_string(), "folder:a:b");
}

#[test]
fn a_malformed_resource_name_is_refused() {
    for text in ["folder", ":root", "folder:"] {
        assert_eq!(
            text.parse::<ResourceName>(),
            Err(NameError::Resource(text.to_owned()))
        );
    }
    assert_eq!(
        "folder".parse::<ResourceName>().unwrap_err().to_string(),
        r#"malformed resource "folder", expected theory:id"#
    );
}

#[test]
fn a_malformed_subjectset_name_is_refused() {
    for text in [
        "file:design.md",
        "design.md#viewer",
        "file:design.md#",
        ":x#viewer",
        "file:#viewer",
    ] {
        assert_eq!(
            text.parse::<SubjectsetName>(),
            Err(NameError::Subjectset(text.to_owned()))
        );
    }
    assert_eq!(
        "file#viewer"
            .parse::<SubjectsetName>()
            .unwrap_err()
            .to_string(),
        r#"malformed subjectset "file#viewer", expected theory:id#relation"#
    );
}

#[test]
fn a_resource_itself_is_not_a_subjectset() {
    let error = "folder:root#...".parse::<SubjectsetName>().unwrap_err();
    assert_eq!(
        error,
        NameError::ResourceMember("folder:root#...".to_owned())
    );
    assert_eq!(
        error.to_string(),
        r#""folder:root#..." names a resource, not a subjectset"#
    );
}

proptest! {
    /// A theory without `:`, any id, and a relation without `#` other than
    /// `...` make a subjectset name that parses back into its parts.
    #[test]
    fn a_subjectset_name_parses_into_its_parts(
        theory in "[^:]+",
        id in ".+",
        relation in "[^#]+",
    ) {
        prop_assume!(relation != "...");
        let text = format!("{theory}:{id}#{relation}");
        let name: SubjectsetName = text.parse().unwrap();
        prop_assert_eq!(name.resource().theory(), &theory);
        prop_assert_eq!(name.resource().id(), &id);
        prop_assert_eq!(name.relation(), &relation);
        prop_assert_eq!(name.to_string(), text);
    }

    #[test]
    fn a_resource_name_parses_into_its_parts(theory in "[^:]+", id in ".+") {
        let text = format!("{theory}:{id}");
        let name: ResourceName = text.parse().unwrap();
        prop_assert_eq!(name.theory(), &theory);
        prop_assert_eq!(name.id(), &id);
        prop_assert_eq!(name.to_string(), text);
    }

    /// Text that parses as a name prints back unchanged.
    #[test]
    fn a_parsed_name_prints_as_written(text in "[ab:#.]{0,8}") {
        if let Ok(name) = text.parse::<SubjectsetName>() {
            prop_assert_eq!(name.to_string(), text.as_str());
        }
        if let Ok(name) = text.parse::<ResourceName>() {
            prop_assert_eq!(name.to_string(), text.as_str());
        }
    }
}
