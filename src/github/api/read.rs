use crate::github::api::{
    team_node_id, user_node_id, BranchProtection, GraphNode, GraphNodes, GraphPageInfo, HttpClient,
    Login, OrgAppInstallation, Repo, RepoAppInstallation, RepoTeam, RepoUser, Team, TeamMember,
    TeamRole,
};
use reqwest::Method;
use std::collections::{HashMap, HashSet};

pub(crate) trait GithubRead {
    /// Get user names by user ids
    fn usernames(&self, ids: &[u64]) -> anyhow::Result<HashMap<u64, String>>;

    /// Get the owners of an org
    fn org_owners(&self, org: &str) -> anyhow::Result<HashSet<u64>>;

    /// Get the members of an org
    fn org_members(&self, org: &str) -> anyhow::Result<HashSet<u64>>;

    /// Get the app installations of an org
    fn org_app_installations(&self, org: &str) -> anyhow::Result<Vec<OrgAppInstallation>>;

    /// Get the repositories enabled for an app installation.
    fn app_installation_repos(
        &self,
        installation_id: u64,
    ) -> anyhow::Result<Vec<RepoAppInstallation>>;

    /// Get all teams associated with a org
    ///
    /// Returns a list of tuples of team name and slug
    fn org_teams(&self, org: &str) -> anyhow::Result<Vec<(String, String)>>;

    /// Get the team by name and org
    fn team(&self, org: &str, team: &str) -> anyhow::Result<Option<Team>>;

    fn team_memberships(&self, team: &Team) -> anyhow::Result<HashMap<u64, TeamMember>>;

    /// The GitHub names of users invited to the given team
    fn team_membership_invitations(&self, org: &str, team: &str)
        -> anyhow::Result<HashSet<String>>;

    /// Get a repo by org and name
    fn repo(&self, org: &str, repo: &str) -> anyhow::Result<Option<Repo>>;

    /// Get teams in a repo
    fn repo_teams(&self, org: &str, repo: &str) -> anyhow::Result<Vec<RepoTeam>>;

    /// Get collaborators in a repo
    ///
    /// Only fetches those who are direct collaborators (i.e., not a collaborator through a repo team)
    fn repo_collaborators(&self, org: &str, repo: &str) -> anyhow::Result<Vec<RepoUser>>;

    /// Get branch_protections
    fn branch_protections(
        &self,
        org: &str,
        repo: &str,
    ) -> anyhow::Result<HashMap<String, (String, BranchProtection)>>;
}

pub(crate) struct GitHubApiRead {
    client: HttpClient,
}

impl GitHubApiRead {
    pub(crate) fn from_client(client: HttpClient) -> anyhow::Result<Self> {
        Ok(Self { client })
    }
}

impl GithubRead for GitHubApiRead {
    fn usernames(&self, ids: &[u64]) -> anyhow::Result<HashMap<u64, String>> {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Usernames {
            database_id: u64,
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
            let res: GraphNodes<Usernames> = self.client.graphql(
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

    fn org_owners(&self, org: &str) -> anyhow::Result<HashSet<u64>> {
        #[derive(serde::Deserialize, Eq, PartialEq, Hash)]
        struct User {
            id: u64,
        }
        let mut owners = HashSet::new();
        self.client.rest_paginated(
            &Method::GET,
            format!("orgs/{org}/members?role=admin"),
            |resp: Vec<User>| {
                owners.extend(resp.into_iter().map(|u| u.id));
                Ok(())
            },
        )?;
        Ok(owners)
    }

    fn org_members(&self, org: &str) -> anyhow::Result<HashSet<u64>> {
        #[derive(serde::Deserialize, Eq, PartialEq, Hash)]
        struct User {
            id: u64,
        }
        let mut members = HashSet::new();
        self.client.rest_paginated(
            &Method::GET,
            format!("orgs/{org}/members"),
            |resp: Vec<User>| {
                members.extend(resp.into_iter().map(|u| u.id));
                Ok(())
            },
        )?;
        Ok(members)
    }

    fn org_app_installations(&self, org: &str) -> anyhow::Result<Vec<OrgAppInstallation>> {
        #[derive(serde::Deserialize, Debug)]
        struct InstallationPage {
            installations: Vec<OrgAppInstallation>,
        }

        let mut installations = Vec::new();
        self.client.rest_paginated(
            &Method::GET,
            format!("orgs/{org}/installations"),
            |response: InstallationPage| {
                installations.extend(response.installations);
                Ok(())
            },
        )?;
        Ok(installations)
    }

    fn app_installation_repos(
        &self,
        installation_id: u64,
    ) -> anyhow::Result<Vec<RepoAppInstallation>> {
        #[derive(serde::Deserialize, Debug)]
        struct InstallationPage {
            repositories: Vec<RepoAppInstallation>,
        }

        let mut installations = Vec::new();
        self.client.rest_paginated(
            &Method::GET,
            format!("user/installations/{installation_id}/repositories"),
            |response: InstallationPage| {
                installations.extend(response.repositories);
                Ok(())
            },
        )?;
        Ok(installations)
    }

    fn org_teams(&self, org: &str) -> anyhow::Result<Vec<(String, String)>> {
        let mut teams = Vec::new();

        self.client.rest_paginated(
            &Method::GET,
            format!("orgs/{org}/teams"),
            |resp: Vec<Team>| {
                teams.extend(resp.into_iter().map(|t| (t.name, t.slug)));
                Ok(())
            },
        )?;

        Ok(teams)
    }

    fn team(&self, org: &str, team: &str) -> anyhow::Result<Option<Team>> {
        self.client
            .send_option(Method::GET, &format!("orgs/{org}/teams/{team}"))
    }

    fn team_memberships(&self, team: &Team) -> anyhow::Result<HashMap<u64, TeamMember>> {
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
            database_id: u64,
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
                let res: GraphNode<RespTeam> = self.client.graphql(
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

    fn team_membership_invitations(
        &self,
        org: &str,
        team: &str,
    ) -> anyhow::Result<HashSet<String>> {
        let mut invites = HashSet::new();

        self.client.rest_paginated(
            &Method::GET,
            format!("orgs/{org}/teams/{team}/invitations"),
            |resp: Vec<Login>| {
                invites.extend(resp.into_iter().map(|l| l.login));
                Ok(())
            },
        )?;

        Ok(invites)
    }

    fn repo(&self, org: &str, repo: &str) -> anyhow::Result<Option<Repo>> {
        self.client
            .send_option(Method::GET, &format!("repos/{org}/{repo}"))
    }

    fn repo_teams(&self, org: &str, repo: &str) -> anyhow::Result<Vec<RepoTeam>> {
        let mut teams = Vec::new();

        self.client.rest_paginated(
            &Method::GET,
            format!("repos/{org}/{repo}/teams"),
            |resp: Vec<RepoTeam>| {
                teams.extend(resp);
                Ok(())
            },
        )?;

        Ok(teams)
    }

    fn repo_collaborators(&self, org: &str, repo: &str) -> anyhow::Result<Vec<RepoUser>> {
        let mut users = Vec::new();

        self.client.rest_paginated(
            &Method::GET,
            format!("repos/{org}/{repo}/collaborators?affiliation=direct"),
            |resp: Vec<RepoUser>| {
                users.extend(resp);
                Ok(())
            },
        )?;

        Ok(users)
    }

    fn branch_protections(
        &self,
        org: &str,
        repo: &str,
    ) -> anyhow::Result<HashMap<String, (String, BranchProtection)>> {
        #[derive(serde::Serialize)]
        struct Params<'a> {
            org: &'a str,
            repo: &'a str,
        }
        static QUERY: &str = "
            query($org:String!,$repo:String!) {
                repository(owner:$org, name:$repo) {
                    branchProtectionRules(first:100) {
                        nodes { 
                            id,
                            pattern,
                            isAdminEnforced,
                            dismissesStaleReviews,
                            requiredStatusCheckContexts,
                            requiredApprovingReviewCount,
                            requiresApprovingReviews
                            pushAllowances(first: 100) {
                                nodes {
                                    actor {
                                        ... on Actor {
                                            login
                                        }
                                        ... on Team {
                                            organization {
                                                login
                                            },
                                            name
                                        }
                                    }
                                }
                            }
                         }
                    }
                }
            }
        ";

        #[derive(serde::Deserialize)]
        struct Wrapper {
            repository: Respository,
        }
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Respository {
            branch_protection_rules: GraphNodes<BranchProtectionWrapper>,
        }
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct BranchProtectionWrapper {
            id: String,
            #[serde(flatten)]
            protection: BranchProtection,
        }

        let mut result = HashMap::new();
        let res: Wrapper = self.client.graphql(QUERY, Params { org, repo })?;
        for node in res
            .repository
            .branch_protection_rules
            .nodes
            .into_iter()
            .flatten()
        {
            result.insert(node.protection.pattern.clone(), (node.id, node.protection));
        }
        Ok(result)
    }
}
