use std::collections::HashMap;

use reqwest::blocking::Client;
use serde::Deserialize;

const ZULIP_BASE_URL: &str = "https://rust-lang.zulipchat.com/api/v1";

/// Access to the Zulip API
#[derive(Clone)]
pub(crate) struct ZulipApi {
    client: Client,
    username: String,
    token: String,
    dry_run: bool,
}

impl ZulipApi {
    /// Create a new `ZulipApi` instance
    pub(crate) fn new(username: String, token: String, dry_run: bool) -> Self {
        Self {
            client: Client::new(),
            username,
            token,
            dry_run,
        }
    }

    /// Creates a Zulip user group with the supplied name, description, and members
    ///
    /// This is a noop if the user group already exists.
    pub(crate) fn create_user_group(
        &self,
        user_group_name: &str,
        description: &str,
        member_ids: &[u64],
    ) -> anyhow::Result<()> {
        log::info!(
            "creating Zulip user group '{}' with description '{}' and member ids: {:?}",
            user_group_name,
            description,
            member_ids
        );
        if self.dry_run {
            return Ok(());
        }

        let member_ids = serialize_as_array(member_ids);
        let mut form = HashMap::new();
        form.insert("name", user_group_name);
        form.insert("description", description);
        form.insert("members", &member_ids);

        let r = self.req(reqwest::Method::POST, "/user_groups/create", Some(form))?;
        if r.status() == 400 {
            let body = r.json::<serde_json::Value>()?;
            let err = || {
                anyhow::format_err!(
                    "got 400 when creating user group {}: {}",
                    user_group_name,
                    body
                )
            };
            let error = body.get("msg").ok_or_else(err)?.as_str().ok_or_else(err)?;
            if error.contains("already exists") {
                log::debug!("Zulip user group '{}' already existed", user_group_name);
                return Ok(());
            } else {
                return Err(err());
            }
        }

        r.error_for_status()?;

        Ok(())
    }

    /// Get all user groups of the Rust Zulip instance
    pub(crate) fn get_user_groups(&self) -> anyhow::Result<Vec<ZulipUserGroup>> {
        let response = self
            .req(reqwest::Method::GET, "/user_groups", None)?
            .error_for_status()?
            .json::<ZulipUserGroups>()?
            .user_groups;

        Ok(response)
    }

    /// Get all users of the Rust Zulip instance
    pub(crate) fn get_users(&self) -> anyhow::Result<Vec<ZulipUser>> {
        let response = self
            .req(reqwest::Method::GET, "/users", None)?
            .error_for_status()?
            .json::<ZulipUsers>()?
            .members;

        Ok(response)
    }

    pub(crate) fn update_user_group_members(
        &self,
        user_group_id: u64,
        add_ids: &[u64],
        remove_ids: &[u64],
    ) -> anyhow::Result<()> {
        if add_ids.is_empty() && remove_ids.is_empty() {
            log::debug!(
                "user group {} does not need to have its group members updated",
                user_group_id
            );
            return Ok(());
        }

        log::info!(
            "updating user group {} by adding {:?} and removing {:?}",
            user_group_id,
            add_ids,
            remove_ids
        );

        if self.dry_run {
            return Ok(());
        }

        let add_ids = serialize_as_array(add_ids);
        let remove_ids = serialize_as_array(remove_ids);
        let mut form = HashMap::new();
        form.insert("add", add_ids.as_str());
        form.insert("delete", remove_ids.as_str());

        let path = format!("/user_groups/{user_group_id}/members");
        let response = self.req(reqwest::Method::POST, &path, Some(form))?;

        if response.status() == 400 {
            log::warn!(
                "failed to update group membership with a bad request: {}",
                response
                    .text()
                    .unwrap_or_else(|_| String::from("<BODY NOT DECODABLE>"))
            );
            return Ok(());
        }

        response.error_for_status()?;
        Ok(())
    }

    /// Perform a request against the Zulip API
    fn req(
        &self,
        method: reqwest::Method,
        path: &str,
        form: Option<HashMap<&str, &str>>,
    ) -> anyhow::Result<reqwest::blocking::Response> {
        let mut req = self
            .client
            .request(method, format!("{ZULIP_BASE_URL}{path}"))
            .basic_auth(&self.username, Some(&self.token));
        if let Some(form) = form {
            req = req.form(&form);
        }

        Ok(req.send()?)
    }
}

/// Serialize a slice of numbers as a JSON array
fn serialize_as_array(items: &[u64]) -> String {
    let items = items
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",");
    format!("[{items}]")
}

/// A collection of Zulip users
#[derive(Deserialize)]
struct ZulipUsers {
    members: Vec<ZulipUser>,
}

/// A single Zulip user
#[derive(Deserialize)]
pub(crate) struct ZulipUser {
    // Note: users may hide their emails
    #[serde(rename = "delivery_email")]
    pub(crate) email: Option<String>,
    pub(crate) user_id: u64,
}

/// A collection of Zulip user groups
#[derive(Deserialize)]
struct ZulipUserGroups {
    user_groups: Vec<ZulipUserGroup>,
}

/// A single Zulip user group
#[derive(Deserialize)]
pub(crate) struct ZulipUserGroup {
    pub(crate) id: u64,
    pub(crate) name: String,
    pub(crate) members: Vec<u64>,
}
