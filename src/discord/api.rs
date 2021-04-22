use failure::{bail, Error};
use log::{info, trace};
use reqwest::{
    header::{self, HeaderValue},
    Client, Method, RequestBuilder, Response,
};
use serde_json::json;
use std::borrow::Cow;
use std::collections::HashMap;

pub(crate) struct Discord {
    token: String,
    client: Client,
}

impl Discord {
    pub(crate) fn new(token: String) -> Self {
        Self {
            token,
            client: Client::new(),
        }
    }

    pub(crate) fn get_guild(&self, name: &str) -> Result<Guild, Error> {
        #[derive(serde::Deserialize)]
        struct PartialGuild {
            name: String,
            id: String,
        }

        let maybe_partial_guild = self
            .req(Method::GET, "/v8/users/@me/guilds")?
            .send()?
            .json::<Vec<PartialGuild>>()?
            .into_iter()
            .find(|guild| guild.name == name);

        let partial_guild = if let Some(partial_guild) = maybe_partial_guild {
            partial_guild
        } else {
            bail!("No guild found by name: {}", &name);
        };

        Ok(self
            .req(Method::GET, &format!("/v8/guilds/{}", partial_guild.id))?
            .send()?
            .json::<Guild>()?)
    }

    pub(crate) fn get_member(
        &self,
        member_id: usize,
        guild_id: &str,
    ) -> Result<Option<GuildMember>, Error> {
        let request = || {
            let f = self.req(
                Method::GET,
                &format!("/v8/guilds/{}/members/{}", guild_id, member_id),
            )?;

            Ok(f)
        };

        with_rate_limiting(request).map(|maybe_res| {
            if let Some(mut res) = maybe_res {
                Some(res.json::<GuildMember>().ok()?)
            } else {
                None
            }
        })
    }

    pub(crate) fn get_roles(&self, guild_id: &str) -> Result<Vec<Role>, Error> {
        Ok(self
            .req(Method::GET, &format!("/v8/guilds/{}/roles", guild_id))?
            .send()?
            .json::<Vec<Role>>()?)
    }

    pub(crate) fn update_user_roles(
        &self,
        guild_id: &str,
        user_id: usize,
        roles: &[String],
    ) -> Result<(), Error> {
        let request = || {
            Ok(self
                .req(
                    Method::PATCH,
                    &format!("/v8/guilds/{}/members/{}", guild_id, user_id),
                )?
                .json(&json!({ "roles": roles })))
        };
        with_rate_limiting(request)?;
        Ok(())
    }

    pub(crate) fn update_guild_role(&self, guild_id: &str, role: &Role) -> Result<(), Error> {
        let request = || {
            Ok(self
                .req(
                    Method::PATCH,
                    &format!("/v8/guilds/{}/roles/{}", guild_id, &role.id),
                )?
                .json(&json!({
                    "name": role.name,
                    "color": role.color,
                })))
        };
        with_rate_limiting(request)?;
        Ok(())
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

// Discord has [rate limits] on their REST api.
//
// [rate limits]: https://discord.com/developers/docs/topics/rate-limits
fn with_rate_limiting<F>(f: F) -> Result<Option<Response>, Error>
where
    F: Fn() -> Result<RequestBuilder, Error>,
{
    use std::str::FromStr;
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    loop {
        let res = f()?.send()?;

        match res.status().as_u16() {
            200 => return Ok(Some(res)),
            400 => bail!("bad request"),
            401 => bail!("invalid auth token"),
            403 => bail!("insufficient permissions"),
            404 => return Ok(None),
            429 => {
                let future_moment =
                    if let Some(header) = res.headers().get("x-ratelimit-reset-after") {
                        f64::from_str(header.to_str()?)?
                    } else {
                        bail!("no x-ratelimit-reset header found in 429 response")
                    };

                info!("rate limited: delaying for {} seconds", future_moment);
                thread::sleep(Duration::from_secs_f64(future_moment));
            }
            c => bail!("unexpected status code: {}", c),
        }
    }
}

#[derive(serde::Deserialize, Debug)]
pub(crate) struct Guild {
    pub id: String,
    pub name: String,
    pub roles: Vec<Role>,
}

#[derive(serde::Deserialize, Debug)]
pub(crate) struct Role {
    pub id: String,
    pub name: String,
    pub color: usize,
}

#[derive(serde::Deserialize, Debug)]
pub(crate) struct GuildMember {
    user: DiscordUser,
    pub roles: Vec<String>,
}

#[derive(serde::Deserialize, Debug)]
pub(crate) struct DiscordUser {
    id: String,
}
