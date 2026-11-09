use std::sync::Arc;

use askama::Template;
use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use sungma_bff::User;

/// What every page needs to know about this Author.
pub(crate) struct Author {
    /// The identity provider's name, on the sign-in button.
    pub provider: String,
}

#[derive(Template)]
#[template(path = "signin.html")]
struct SignIn<'a> {
    provider: &'a str,
    version: &'static str,
}

#[derive(Template)]
#[template(path = "home.html")]
struct Home {
    you: You,
}

/// The signed-in person as the pages show them.
#[derive(Debug, PartialEq, Eq)]
struct You {
    /// Their name, or the next best thing their ID token holds.
    name: String,
    /// What their menu shows: their username, or their name.
    handle: String,
    subject: String,
    /// The end of their subject id, to tell it apart at a glance.
    short: String,
}

impl You {
    fn of(user: User) -> Self {
        let short = short(&user.subject);
        let name = user
            .name
            .clone()
            .or_else(|| user.username.clone())
            .or_else(|| user.email.clone())
            .unwrap_or_else(|| short.clone());
        let handle = user.username.unwrap_or_else(|| name.clone());
        Self {
            name,
            handle,
            subject: user.subject,
            short,
        }
    }
}

/// The last group of a subject id after an ellipsis, at most twelve
/// characters, such as `…2d5e1b0a9f43`, or the whole id when that is no
/// longer.
fn short(subject: &str) -> String {
    let group = subject.rsplit('-').next().unwrap_or(subject);
    let skip = group.chars().count().saturating_sub(12);
    let tail: String = group.chars().skip(skip).collect();
    if tail.len() == subject.len() {
        tail
    } else {
        format!("…{tail}")
    }
}

/// The sign-in page.
pub(crate) async fn sign_in(State(author): State<Arc<Author>>) -> Response {
    render(&SignIn {
        provider: &author.provider,
        version: env!("CARGO_PKG_VERSION"),
    })
}

/// Home, for the signed-in person.
pub(crate) async fn home(user: User) -> Response {
    render(&Home { you: You::of(user) })
}

fn render(page: &impl Template) -> Response {
    match page.render() {
        Ok(html) => Html(html).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(name: Option<&str>, username: Option<&str>, email: Option<&str>) -> User {
        User {
            subject: "8f3c2a1e-6b4d-4e2a-9c7f-2d5e1b0a9f43".to_owned(),
            name: name.map(str::to_owned),
            username: username.map(str::to_owned),
            email: email.map(str::to_owned),
        }
    }

    #[test]
    fn a_subject_id_shows_its_last_group() {
        assert_eq!(
            short("8f3c2a1e-6b4d-4e2a-9c7f-2d5e1b0a9f43"),
            "…2d5e1b0a9f43"
        );
    }

    #[test]
    fn a_subject_id_without_groups_shows_its_last_twelve_characters() {
        assert_eq!(short("za9UxpH7XVxqrtpEbThoqvn2"), "…rtpEbThoqvn2");
    }

    #[test]
    fn a_short_subject_id_shows_whole() {
        assert_eq!(short("alice"), "alice");
        assert_eq!(short("abcdefghijkl"), "abcdefghijkl");
    }

    #[test]
    fn a_long_last_group_shows_its_last_twelve_characters() {
        assert_eq!(short("a-bcdefghijklmn"), "…cdefghijklmn");
    }

    #[test]
    fn a_person_shows_by_name_with_their_username_as_handle() {
        let you = You::of(user(Some("Mark Lauter"), Some("mark"), Some("m@x.com")));
        assert_eq!(you.name, "Mark Lauter");
        assert_eq!(you.handle, "mark");
        assert_eq!(you.short, "…2d5e1b0a9f43");
    }

    #[test]
    fn a_person_without_a_name_shows_by_username_then_email_then_subject() {
        assert_eq!(
            You::of(user(None, Some("mark"), Some("m@x.com"))).name,
            "mark"
        );
        let by_email = You::of(user(None, None, Some("m@x.com")));
        assert_eq!(
            (by_email.name.as_str(), by_email.handle.as_str()),
            ("m@x.com", "m@x.com")
        );
        let by_subject = You::of(user(None, None, None));
        assert_eq!(by_subject.name, "…2d5e1b0a9f43");
    }

    #[test]
    fn a_person_without_a_username_has_their_name_as_handle() {
        assert_eq!(You::of(user(Some("Mark"), None, None)).handle, "Mark");
    }
}
