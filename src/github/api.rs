use failure::{bail, Error};
use hyper_old_types::header::{Link, RelationType};
use log::{debug, trace};
use reqwest::{
    header::{self, HeaderValue},
    Client, Method, RequestBuilder, Response, StatusCode,
};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fmt;

pub(crate) struct GitHub {
    token: String,
    dry_run: bool,
    client: Client,
}

impl GitHub {
    pub(crate) fn new(token: String, dry_run: bool) -> Self {
        GitHub {
            token,
            dry_run,
            client: Client::new(),
        }
    }

    fn req(&self, method: Method, url: &str) -> Result<RequestBuilder, Error> {
        let url = if url.starts_with("https://") {
            Cow::Borrowed(url)
        } else {
            Cow::Owned(format!("https://api.github.com/{}", url))
        };
        trace!("http request: {} {}", method, url);
        Ok(self
            .client
            .request(method, url.as_ref())
            .header(
                header::AUTHORIZATION,
                HeaderValue::from_str(&format!("token {}", self.token))?,
            )
            .header(
                header::USER_AGENT,
                HeaderValue::from_static(crate::USER_AGENT),
            ))
    }

    fn graphql<R, V>(&self, query: &str, variables: V) -> Result<R, Error>
    where
        R: serde::de::DeserializeOwned,
        V: serde::Serialize,
    {
        #[derive(serde::Serialize)]
        struct Request<'a, V> {
            query: &'a str,
            variables: V,
        }
        let res: GraphResult<R> = self
            .req(Method::POST, "graphql")?
            .json(&Request { query, variables })
            .send()?
            .error_for_status()?
            .json()?;
        if let Some(error) = res.errors.get(0) {
            bail!("graphql error: {}", error.message);
        } else if let Some(data) = res.data {
            Ok(data)
        } else {
            bail!("missing graphql data");
        }
    }

    fn rest_paginated<F>(&self, method: &Method, url: String, mut f: F) -> Result<(), Error>
    where
        F: FnMut(Response) -> Result<(), Error>,
    {
        let mut next = Some(url);
        while let Some(next_url) = next.take() {
            let resp = self
                .req(method.clone(), &next_url)?
                .send()?
                .error_for_status()?;

            // Extract the next page
            if let Some(links) = resp.headers().get(header::LINK) {
                let links: Link = links.to_str()?.parse()?;
                for link in links.values() {
                    if link
                        .rel()
                        .map(|r| r.iter().any(|r| *r == RelationType::Next))
                        .unwrap_or(false)
                    {
                        next = Some(link.link().to_string());
                        break;
                    }
                }
            }

            f(resp)?;
        }
        Ok(())
    }

    pub(crate) fn team(&self, org: &str, team: &str) -> Result<Option<Team>, Error> {
        let mut resp = self
            .req(Method::GET, &format!("orgs/{}/teams/{}", org, team))?
            .send()?;
        match resp.status() {
            StatusCode::OK => Ok(Some(resp.json()?)),
            StatusCode::NOT_FOUND => Ok(None),
            _ => Err(resp.error_for_status().unwrap_err().into()),
        }
    }

    pub(crate) fn create_team(
        &self,
        org: &str,
        name: &str,
        description: &str,
        privacy: TeamPrivacy,
    ) -> Result<Team, Error> {
        #[derive(serde::Serialize)]
        struct Req<'a> {
            name: &'a str,
            description: &'a str,
            privacy: TeamPrivacy,
        }
        if self.dry_run {
            debug!("dry: created team {}/{}", org, name);
            Ok(Team {
                // The None marks that the team is "created" by the dry run and doesn't actually
                // exists on GitHub
                id: None,
                name: name.to_string(),
                description: description.to_string(),
                privacy,
            })
        } else {
            Ok(self
                .req(Method::POST, &format!("orgs/{}/teams", org))?
                .json(&Req {
                    name,
                    description,
                    privacy,
                })
                .send()?
                .error_for_status()?
                .json()?)
        }
    }

    pub(crate) fn edit_team(
        &self,
        team: &Team,
        name: &str,
        description: &str,
        privacy: TeamPrivacy,
    ) -> Result<(), Error> {
        #[derive(serde::Serialize)]
        struct Req<'a> {
            name: &'a str,
            description: &'a str,
            privacy: TeamPrivacy,
        }
        if let (false, Some(id)) = (self.dry_run, team.id) {
            self.req(Method::PATCH, &format!("teams/{}", id))?
                .json(&Req {
                    name,
                    description,
                    privacy,
                })
                .send()?
                .error_for_status()?;
        } else {
            debug!("dry: edit team {}", name)
        }
        Ok(())
    }

    pub(crate) fn usernames(&self, ids: &[usize]) -> Result<HashMap<usize, String>, Error> {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Usernames {
            database_id: usize,
            login: String,
        }
        #[derive(serde::Serialize)]
        struct Params {
            ids: Vec<String>,
        }
        static QUERY: &str = "
            query($ids: [ID!]!) {
                nodes(ids: $ids) {
                    ... on User {
                        databaseId
                        login
                    }
                }
            }
        ";

        let mut result = HashMap::new();
        for chunk in ids.chunks(100) {
            let res: GraphNodes<Usernames> = self.graphql(
                QUERY,
                Params {
                    ids: chunk.iter().map(|id| user_node_id(*id)).collect(),
                },
            )?;
            for node in res.nodes.into_iter().flatten() {
                result.insert(node.database_id, node.login);
            }
        }
        Ok(result)
    }

    pub(crate) fn org_owners(&self, org: &str) -> Result<HashSet<usize>, Error> {
        #[derive(serde::Deserialize, Eq, PartialEq, Hash)]
        struct User {
            id: usize,
        }
        let mut owners = HashSet::new();
        self.rest_paginated(
            &Method::GET,
            format!("orgs/{}/members?role=admin", org),
            |mut resp| {
                let partial: Vec<User> = resp.json()?;
                for owner in partial {
                    owners.insert(owner.id);
                }
                Ok(())
            },
        )?;
        Ok(owners)
    }

    pub(crate) fn team_memberships(
        &self,
        team: &Team,
    ) -> Result<HashMap<usize, TeamMember>, Error> {
        #[derive(serde::Deserialize)]
        struct RespTeam {
            members: RespMembers,
        }
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct RespMembers {
            page_info: GraphPageInfo,
            edges: Vec<RespEdge>,
        }
        #[derive(serde::Deserialize)]
        struct RespEdge {
            role: TeamRole,
            node: RespNode,
        }
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct RespNode {
            database_id: usize,
            login: String,
        }
        #[derive(serde::Serialize)]
        struct Params<'a> {
            team: String,
            cursor: Option<&'a str>,
        }
        static QUERY: &str = "
            query($team: ID!, $cursor: String) {
                node(id: $team) {
                    ... on Team {
                        members(after: $cursor) {
                            pageInfo {
                                endCursor
                                hasNextPage
                            }
                            edges {
                                role
                                node {
                                    databaseId
                                    login
                                }
                            }
                        }
                    }
                }
            }
        ";

        let mut memberships = HashMap::new();
        // Return the empty HashMap on new teams from dry runs
        if let Some(id) = team.id {
            let mut page_info = GraphPageInfo::start();
            while page_info.has_next_page {
                let res: GraphNode<RespTeam> = self.graphql(
                    QUERY,
                    Params {
                        team: team_node_id(id),
                        cursor: page_info.end_cursor.as_deref(),
                    },
                )?;
                if let Some(team) = res.node {
                    page_info = team.members.page_info;
                    for edge in team.members.edges.into_iter() {
                        memberships.insert(
                            edge.node.database_id,
                            TeamMember {
                                id: edge.node.database_id,
                                username: edge.node.login,
                                role: edge.role,
                            },
                        );
                    }
                }
            }
        }

        Ok(memberships)
    }

    pub(crate) fn set_membership(
        &self,
        team: &Team,
        username: &str,
        role: TeamRole,
    ) -> Result<(), Error> {
        #[derive(serde::Serialize)]
        struct Req {
            role: TeamRole,
        }
        if let (false, Some(id)) = (self.dry_run, team.id) {
            self.req(
                Method::PUT,
                &format!("teams/{}/memberships/{}", id, username),
            )?
            .json(&Req { role })
            .send()?
            .error_for_status()?;
        } else {
            debug!("dry: set membership of {} to {}", username, role);
        }
        Ok(())
    }

    pub(crate) fn remove_membership(&self, team: &Team, username: &str) -> Result<(), Error> {
        if let (false, Some(id)) = (self.dry_run, team.id) {
            self.req(
                Method::DELETE,
                &format!("teams/{}/memberships/{}", id, username),
            )?
            .send()?
            .error_for_status()?;
        } else {
            debug!("dry: remove membership of {}", username);
        }
        Ok(())
    }
}

#[derive(serde::Deserialize)]
struct GraphResult<T> {
    data: Option<T>,
    #[serde(default)]
    errors: Vec<GraphError>,
}

#[derive(serde::Deserialize)]
struct GraphError {
    message: String,
}

#[derive(serde::Deserialize)]
struct GraphNodes<T> {
    nodes: Vec<Option<T>>,
}

#[derive(serde::Deserialize)]
struct GraphNode<T> {
    node: Option<T>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphPageInfo {
    end_cursor: Option<String>,
    has_next_page: bool,
}

impl GraphPageInfo {
    fn start() -> Self {
        GraphPageInfo {
            end_cursor: None,
            has_next_page: true,
        }
    }
}

#[derive(serde::Deserialize, Debug)]
pub(crate) struct Team {
    /// The ID returned by the GitHub API can't be empty, but the None marks teams "created" during
    /// a dry run and not actually present on GitHub, so other methods can avoid acting on them.
    id: Option<usize>,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) privacy: TeamPrivacy,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Eq, PartialEq, Copy, Clone)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TeamPrivacy {
    Closed,
    Secret,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Eq, PartialEq, Copy, Clone)]
#[serde(rename_all(serialize = "snake_case", deserialize = "SCREAMING_SNAKE_CASE"))]
pub(crate) enum TeamRole {
    Member,
    Maintainer,
}

impl fmt::Display for TeamRole {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            TeamRole::Member => write!(f, "member"),
            TeamRole::Maintainer => write!(f, "maintainer"),
        }
    }
}

#[derive(Debug)]
pub(crate) struct TeamMember {
    pub(crate) id: usize,
    pub(crate) username: String,
    pub(crate) role: TeamRole,
}

fn user_node_id(id: usize) -> String {
    base64::encode(&format!("04:User{}", id))
}

fn team_node_id(id: usize) -> String {
    base64::encode(&format!("04:Team{}", id))
}
