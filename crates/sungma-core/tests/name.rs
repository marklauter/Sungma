//! Resource and subjectset names, parsed at the edge.

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
