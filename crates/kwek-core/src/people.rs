//! Resolving `users/{id}` to names and avatars. With user auth the Chat API returns senders
//! without display names; Chat user IDs equal People IDs, so `users/123` is `people/123`.

use async_trait::async_trait;
use serde::Deserialize;

use crate::error::Result;
use crate::google::{Google, q};

const BASE: &str = "https://people.googleapis.com/v1";

#[derive(Debug, Clone, PartialEq)]
pub struct Person {
    /// `users/{id}`
    pub user: String,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
}

#[async_trait]
pub trait PeopleApi: Send + Sync {
    async fn me(&self) -> Result<Person>;
    /// At most 200 per call; unknown users are simply absent from the result.
    async fn resolve(&self, users: &[String]) -> Result<Vec<Person>>;
    /// Job title and department from the directory, for a profile card.
    async fn role(&self, _user: &str) -> Result<Role> {
        Ok(Role::default())
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Role {
    pub title: Option<String>,
    pub department: Option<String>,
}

/// The current (or else first) organization's title and department.
fn role_of(v: &serde_json::Value) -> Role {
    let orgs = v.get("organizations").and_then(|o| o.as_array());
    let org = orgs.and_then(|o| {
        o.iter()
            .find(|x| x.get("current").and_then(|c| c.as_bool()) == Some(true))
            .or_else(|| o.first())
    });
    let field = |k: &str| {
        org.and_then(|o| o.get(k))
            .and_then(|t| t.as_str())
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
    };
    Role {
        title: field("title"),
        department: field("department"),
    }
}

pub struct HttpPeople {
    g: Google,
}

impl HttpPeople {
    pub fn new(g: Google) -> Self {
        Self { g }
    }
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct ApiPerson {
    resource_name: String,
    names: Vec<Named>,
    email_addresses: Vec<Valued>,
    photos: Vec<Photo>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct Named {
    display_name: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Valued {
    value: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Photo {
    url: Option<String>,
    default: Option<bool>,
}

impl ApiPerson {
    fn into_person(self) -> Person {
        let id = self.resource_name.trim_start_matches("people/").to_owned();
        Person {
            user: format!("users/{id}"),
            display_name: self.names.into_iter().find_map(|n| n.display_name),
            email: self.email_addresses.into_iter().find_map(|e| e.value),
            avatar_url: self
                .photos
                .into_iter()
                .find(|p| p.default != Some(true))
                .and_then(|p| p.url),
        }
    }
}

const FIELDS: &str = "names,emailAddresses,photos";

#[async_trait]
impl PeopleApi for HttpPeople {
    async fn me(&self) -> Result<Person> {
        let p: ApiPerson = self.g.get(&format!("{BASE}/people/me?personFields={FIELDS}")).await?;
        Ok(p.into_person())
    }

    async fn resolve(&self, users: &[String]) -> Result<Vec<Person>> {
        if users.is_empty() {
            return Ok(vec![]);
        }
        let mut url = format!(
            "{BASE}/people:batchGet?personFields={FIELDS}\
             &sources=READ_SOURCE_TYPE_PROFILE&sources=READ_SOURCE_TYPE_DOMAIN_CONTACT&sources=READ_SOURCE_TYPE_CONTACT"
        );
        for u in users.iter().take(200) {
            let id = u.trim_start_matches("users/");
            url += &format!("&resourceNames={}", q(&format!("people/{id}")));
        }
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct Resp {
            responses: Vec<Entry>,
        }
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct Entry {
            person: Option<ApiPerson>,
        }
        let r: Resp = self.g.get(&url).await?;
        Ok(r.responses
            .into_iter()
            .filter_map(|e| e.person)
            .map(ApiPerson::into_person)
            .collect())
    }

    async fn role(&self, user: &str) -> Result<Role> {
        let id = user.trim_start_matches("users/");
        let v: serde_json::Value = self
            .g
            .get(&format!(
                "{BASE}/people/{}?personFields=organizations\
                 &sources=READ_SOURCE_TYPE_PROFILE&sources=READ_SOURCE_TYPE_DOMAIN_CONTACT",
                q(id)
            ))
            .await?;
        Ok(role_of(&v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_from_the_current_organization() {
        let r = role_of(&serde_json::json!({ "organizations": [
            { "title": "Intern", "current": false },
            { "title": " Product designer ", "department": "Design", "current": true } ] }));
        assert_eq!(r.title.as_deref(), Some("Product designer"));
        assert_eq!(r.department.as_deref(), Some("Design"));
        assert_eq!(role_of(&serde_json::json!({})), Role::default());
    }

    #[test]
    fn maps_person_and_skips_default_photo() {
        let p: ApiPerson = serde_json::from_str(
            r#"{"resourceName":"people/42","names":[{"displayName":"Ada"}],
                "emailAddresses":[{"value":"ada@x.nl"}],
                "photos":[{"url":"https://p/default","default":true}]}"#,
        )
        .unwrap();
        let p = p.into_person();
        assert_eq!(p.user, "users/42");
        assert_eq!(p.display_name.as_deref(), Some("Ada"));
        assert_eq!(p.email.as_deref(), Some("ada@x.nl"));
        assert_eq!(p.avatar_url, None);
    }
}
