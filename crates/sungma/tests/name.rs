//! Resource and subjectset names, parsed at the edge.

use proptest::prelude::*;
use sungma::name::{
    Identity, MAX_IDENTITY_BYTES, MAX_NAMESPACE_BYTES, MAX_QUOTE_BYTES, MAX_RESOURCE_ID_BYTES,
    NameError, NamespaceName, RelationName, ResourceName, SubjectsetName, quote,
};

#[test]
fn a_resource_id_is_at_most_max_resource_id_bytes() {
    let name = |len: usize| format!("file:{}", "x".repeat(len));
    assert!(name(MAX_RESOURCE_ID_BYTES).parse::<ResourceName>().is_ok());
    assert!(matches!(
        name(MAX_RESOURCE_ID_BYTES + 1).parse::<ResourceName>(),
        Err(NameError::ResourceIdTooLong(_))
    ));
}

#[test]
fn a_resource_id_holds_no_whitespace() {
    for text in ["file:a b", "file:a\tb", "file:a\u{a0}b", "file:a\n"] {
        assert!(
            matches!(text.parse::<ResourceName>(), Err(NameError::Whitespace(_))),
            "{text:?}"
        );
    }
    assert!(matches!(
        "file:a b#owner".parse::<SubjectsetName>(),
        Err(NameError::Whitespace(_))
    ));
}

#[test]
fn an_identity_is_opaque_but_holds_no_whitespace() {
    let identity: Identity = "x@y#z:...".parse().unwrap();
    assert_eq!(identity.as_str(), "x@y#z:...");
    assert_eq!("".parse::<Identity>(), Err(NameError::EmptyIdentity));
    for text in ["a b", "a\tb", "a\u{2003}b"] {
        assert!(
            matches!(text.parse::<Identity>(), Err(NameError::Whitespace(_))),
            "{text:?}"
        );
    }
}

#[test]
fn an_identity_is_at_most_max_identity_bytes() {
    assert!("x".repeat(MAX_IDENTITY_BYTES).parse::<Identity>().is_ok());
    assert!(matches!(
        "x".repeat(MAX_IDENTITY_BYTES + 1).parse::<Identity>(),
        Err(NameError::IdentityTooLong(_))
    ));
}

#[test]
fn a_subjectset_name_splits_at_the_first_colon_and_the_last_hash() {
    let name: SubjectsetName = "file:a:b#c#viewer".parse().unwrap();
    assert_eq!(name.resource().namespace().as_str(), "file");
    assert_eq!(name.resource().id().as_str(), "a:b#c");
    assert_eq!(name.relation().as_str(), "viewer");
    assert_eq!(name.to_string(), "file:a:b#c#viewer");
}

#[test]
fn a_resource_name_splits_at_the_first_colon() {
    let name: ResourceName = "folder:a:b".parse().unwrap();
    assert_eq!(name.namespace().as_str(), "folder");
    assert_eq!(name.id().as_str(), "a:b");
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
        "malformed resource 'folder', expected namespace:id"
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
        "malformed subjectset 'file#viewer', expected namespace:id#relation"
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
        "'folder:root#...' names a resource, not a subjectset"
    );
}

/// Namespace and relation names of the grammar, within the length limit.
const NAMESPACE: &str = "[a-zA-Z_][a-zA-Z0-9_]{0,20}(\\.[a-zA-Z_][a-zA-Z0-9_]{0,20}){0,1}";
const RELATION: &str = "[a-zA-Z_][a-zA-Z0-9_]{0,40}";

#[test]
fn the_namespace_and_relation_of_a_name_follow_the_grammar() {
    let cases = [
        ("fi le:a#viewer", NameError::Invalid("fi le".to_owned())),
        ("file:a#view er", NameError::Invalid("view er".to_owned())),
        ("file:a#This", NameError::Reserved("This".to_owned())),
        ("file.:a#viewer", NameError::Invalid("file.".to_owned())),
    ];
    for (text, error) in cases {
        assert_eq!(text.parse::<SubjectsetName>(), Err(error), "{text}");
    }
    let long = "n".repeat(MAX_NAMESPACE_BYTES + 1);
    assert_eq!(
        format!("{long}:a").parse::<ResourceName>(),
        Err(NameError::TooLong(quote(&long), MAX_NAMESPACE_BYTES))
    );
}

#[test]
fn a_name_has_checked_and_unchecked_constructors() {
    assert_eq!(
        NamespaceName::try_from("drive.file").map(|name| name.to_string()),
        Ok("drive.file".to_owned())
    );
    assert_eq!(
        RelationName::try_from("this"),
        Err(NameError::Reserved("this".to_owned()))
    );
    assert_eq!(RelationName::new_unchecked("this").as_ref(), "this");
    assert!(NamespaceName::new_unchecked("a b") < NamespaceName::new_unchecked("b"));
}

#[test]
fn a_name_error_quotes_a_bounded_and_escaped_cut_of_the_text() {
    let long = format!("{}{}", "x".repeat(300), char::from_u32(0x202e).unwrap());
    let error = format!("{long}:a").parse::<ResourceName>().unwrap_err();
    assert_eq!(
        error,
        NameError::TooLong(format!("{}...", "x".repeat(61)), MAX_NAMESPACE_BYTES)
    );
    let hidden = format!("a{}b:x", char::from_u32(0x202e).unwrap());
    let error = hidden.parse::<ResourceName>().unwrap_err().to_string();
    assert!(error.contains("a\\u{202e}b"), "{error}");
}

proptest! {
    /// A namespace name, any id without whitespace, and a relation name make a
    /// subjectset name that parses back into its parts.
    #[test]
    fn a_subjectset_name_parses_into_its_parts(
        namespace in NAMESPACE,
        id in "[!-~]{1,64}",
        relation in RELATION,
    ) {
        prop_assume!(!relation.eq_ignore_ascii_case("this"));
        let text = format!("{namespace}:{id}#{relation}");
        let name: SubjectsetName = text.parse().unwrap();
        prop_assert_eq!(name.resource().namespace().as_str(), &namespace);
        prop_assert_eq!(name.resource().id().as_str(), &id);
        prop_assert_eq!(name.relation().as_str(), &relation);
        prop_assert_eq!(name.to_string(), text);
    }

    #[test]
    fn a_resource_name_parses_into_its_parts(namespace in NAMESPACE, id in "[!-~]{1,64}") {
        let text = format!("{namespace}:{id}");
        let name: ResourceName = text.parse().unwrap();
        prop_assert_eq!(name.namespace().as_str(), &namespace);
        prop_assert_eq!(name.id().as_str(), &id);
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

#[test]
fn a_quote_counts_its_escapes_toward_its_bound() {
    let hidden = char::from_u32(0x200b).unwrap().to_string().repeat(20);
    let NameError::Invalid(quoted) = format!("{hidden}:a").parse::<ResourceName>().unwrap_err()
    else {
        panic!("expected an invalid name");
    };
    // Each escape is 8 bytes, so 7 fit beside the 3-byte cut mark.
    assert_eq!(quoted, format!("{}...", "\\u{200b}".repeat(7)));
    for code in [0x1bca0, 0x1d173] {
        let c = char::from_u32(code).unwrap();
        let error = format!("a{c}:x")
            .parse::<ResourceName>()
            .unwrap_err()
            .to_string();
        assert!(!error.contains(c), "{code:x}: {error}");
    }
}

#[test]
fn a_cut_quote_fits_its_bound_with_the_cut_mark() {
    let text = |len: usize| "x".repeat(len);
    assert_eq!(quote(&text(MAX_QUOTE_BYTES)), text(MAX_QUOTE_BYTES));
    let cut = quote(&text(MAX_QUOTE_BYTES + 1));
    assert_eq!(cut, format!("{}...", text(MAX_QUOTE_BYTES - 3)));
    assert_eq!(cut.len(), MAX_QUOTE_BYTES);
}
