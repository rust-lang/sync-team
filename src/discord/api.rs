use failure::{bail, Error};
use log::{debug, info, trace};
use reqwest::{
    header::{self, HeaderValue},
    Client, Method, RequestBuilder, Response, StatusCode,
};
use std::borrow::Cow;
use std::collections::HashMap;

pub(crate) struct Discord {
    token: String,
    dry_run: bool,
    client: Client,
}

impl Discord {
    pub(crate) fn new(token: String, dry_run: bool) -> Self {
        Self {
            token,
            dry_run,
            client: Client::new(),
        }
    }

    pub(crate) fn get_roles(&self, guild_name: &str) -> Result<HashMap<String, String>, Error> {
        let guilds = self.get_guilds()?;

        let guild_id = if let Some(id) = guilds.get(guild_name) {
            id
        } else {
            bail!("No guild found by name: {}", guild_name);
        };

        Ok(self.req(Method::GET, &format!("/guilds/{}/roles", guild_id))?
            .send()?
            .json::<Vec<Role>>()?
            .into_iter()
            .fold(HashMap::new(), |mut hash_map, role| {
                hash_map.insert(role.name, role.id);
                hash_map
            }))
    }

    fn get_guilds(&self) -> Result<HashMap<String, String>, Error> {
        Ok(self
            .req(Method::GET, "/users/@me/guilds")?
            .send()?
            .json::<Vec<Guild>>()?
            .into_iter()
            .fold(HashMap::new(), |mut hash_map, guild| {
                hash_map.insert(guild.name, guild.id);
                hash_map
            }))
    }

    fn req(&self, method: Method, url: &str) -> Result<RequestBuilder, Error> {
        let url = if url.starts_with("https://") {
            Cow::Borrowed(url)
        } else {
            Cow::Owned(format!("https://discord.com/api{}", url))
        };
        trace!("http request: {} {}", method, url);
        Ok(self
            .client
            .request(method, url.as_ref())
            .header(
                header::AUTHORIZATION,
                HeaderValue::from_str(&format!("Bot {}", self.token))?,
            )
            .header(
                header::USER_AGENT,
                HeaderValue::from_static(crate::USER_AGENT),
            ))
    }
}

#[derive(serde::Deserialize, Debug)]
struct Guild {
    id: String,
    name: String,
}

#[derive(serde::Deserialize, Debug)]
struct Role {
    id: String,
    name: String,
}
