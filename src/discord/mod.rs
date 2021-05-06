mod api;

use self::api::Discord;
use crate::TeamApi;
use failure::Error;
use log::{info, warn};
use std::collections::HashMap;
use std::str::FromStr;

const RUST_LANG_DISCORD: &str = "The Rust Programming Language";

pub(crate) struct SyncDiscord {
    discord: Discord,
    dry_run: bool,
    teams: Vec<rust_team_data::v1::Team>,
}

impl SyncDiscord {
    pub(crate) fn new(token: String, team_api: &TeamApi, dry_run: bool) -> Result<Self, Error> {
        let teams = team_api.get_teams()?;

        let discord = Discord::new(token);

        Ok(Self {
            discord,
            dry_run,
            teams,
        })
    }

    pub(crate) fn run(&self) -> Result<(), Error> {
        info!("Fetching guild: {}", RUST_LANG_DISCORD);
        let guild = self.discord.get_guild(RUST_LANG_DISCORD)?;

        let guild_id = guild.id;
        let mut guild_roles = guild.roles;

        info!("Fetching users from discord...");
        let mut users = self.get_users(&guild_id)?;

        info!("Computing role updates...");
        let mut role_updates = HashMap::new();
        let mut new_roles = Vec::new();
        let min_managed_role_position =
            self.get_role_updates(&guild_roles, &mut role_updates, &mut new_roles)?;

        info!("Computing user updates...");
        let mut user_updates = HashMap::new();
        for (user_id, user) in &users {
            self.get_user_updates(*user_id, &mut user_updates, &user, &guild_roles)?;
        }

        if !self.dry_run {
            info!("Creating new roles...");

            for new_role in new_roles {
                if !guild_roles.iter().any(|role| role.name == new_role.name) {
                    info!("Adding new role: \"{}\"", new_role.name);
                    let role = self.discord.create_guild_role(
                        &guild_id,
                        &new_role.name,
                        new_role.color,
                    )?;

                    let role_id = usize::from_str(&role.id)?;
                    guild_roles.push(role);

                    if let Some(position) = min_managed_role_position {
                        role_updates
                            .entry(role_id)
                            .or_insert_with(Vec::new)
                            .push(RoleUpdate::ChangePosition(position - 1));
                    }

                    for member in new_role.members {
                        user_updates
                            .entry(*member)
                            .or_insert_with(Vec::new)
                            .push(UserUpdate::AddRole(role_id));
                    }
                } else {
                    info!("A role with the name \"{}\" already exists", new_role.name);
                    continue;
                }
            }

            info!("Applying user updates...");

            for (user_id, updates) in user_updates {
                let user = if let Some(user) = users.get_mut(&user_id) {
                    user
                } else {
                    continue;
                };

                let roles = &mut user.roles;

                for update in updates {
                    match update {
                        UserUpdate::AddRole(id) => {
                            roles.push(id.to_string());
                        }
                        UserUpdate::RemoveRole(id) => {
                            roles.retain(|role_id| role_id != &id.to_string());
                        }
                    }
                }

                self.discord.update_user_roles(&guild_id, user_id, roles)?;
            }

            info!("Applying role updates...");

            for (role_id, updates) in role_updates {
                let mut role = guild_roles
                    .iter_mut()
                    .find(|role| role.id == role_id.to_string())
                    .unwrap();

                let mut positions = Vec::<api::RolePosition>::new();

                for update in updates {
                    match update {
                        RoleUpdate::ChangeColor(color) => {
                            role.color = color;
                        }
                        RoleUpdate::ChangePosition(position) => {
                            positions.push(api::RolePosition {
                                id: role_id.to_string(),
                                position,
                            });
                        }
                    }
                }

                info!("Updating existing roles");
                self.discord.update_guild_role(&guild_id, &role)?;
                if !positions.is_empty() {
                    info!("Updating role positions");
                    self.discord
                        .update_guild_role_positions(&guild_id, &positions)?;
                }
            }
        }

        Ok(())
    }

    fn get_users(&self, guild_id: &str) -> Result<HashMap<usize, api::GuildMember>, Error> {
        let mut users = HashMap::new();

        let maybe_all = &self.teams.iter().find(|team| team.name == "all");

        let all = if let Some(all) = maybe_all {
            all
        } else {
            return Ok(users);
        };

        for discord_team in &all.discord {
            for member in &discord_team.members {
                match self.discord.get_member(*member, &guild_id) {
                    Ok(Some(guild_member)) => {
                        users.insert(*member, guild_member);
                    }
                    Ok(None) => {
                        warn!("User {} was not found in the guild", member);
                        continue;
                    }
                    Err(res) => return Err(res),
                }
            }
        }

        Ok(users)
    }

    fn get_user_updates(
        &self,
        user_id: usize,
        user_updates: &mut HashMap<usize, Vec<UserUpdate>>,
        user: &api::GuildMember,
        guild_roles: &[api::Role],
    ) -> Result<(), Error> {
        let current_roles = &user.roles;

        for team in &self.teams {
            for discord_team in &team.discord {
                let team_members = &discord_team.members;
                let team_role_id = if let Some(role) = guild_roles
                    .iter()
                    .find(|guild_role| guild_role.name == discord_team.name)
                {
                    usize::from_str(&role.id)?
                } else {
                    warn!("Role not found in guild: {}", discord_team.name);
                    continue;
                };

                let team_role_id_str = team_role_id.to_string();

                if team_members.contains(&user_id) && !current_roles.contains(&team_role_id_str) {
                    user_updates
                        .entry(user_id)
                        .or_insert_with(Vec::new)
                        .push(UserUpdate::AddRole(team_role_id));
                }

                if current_roles.contains(&team_role_id_str) && !team_members.contains(&user_id) {
                    user_updates
                        .entry(user_id)
                        .or_insert_with(Vec::new)
                        .push(UserUpdate::RemoveRole(team_role_id));
                }
            }
        }

        Ok(())
    }

    fn get_role_updates<'m>(
        &'m self,
        guild_roles: &[api::Role],
        role_updates: &mut HashMap<usize, Vec<RoleUpdate>>,
        new_roles: &mut Vec<NewRole<'m>>,
    ) -> Result<Option<usize>, Error> {
        let mut min_managed_role_position = None;

        for team in &self.teams {
            for discord_team in &team.discord {
                if let Some(role) = guild_roles
                    .iter()
                    .find(|guild_role| guild_role.name == discord_team.name)
                {
                    if let Some(position) = min_managed_role_position {
                        if position > role.position {
                            min_managed_role_position = Some(role.position);
                        }
                    } else {
                        min_managed_role_position = Some(role.position);
                    }

                    if let Some(color) = discord_team.color.as_ref() {
                        let color_code = usize::from_str_radix(&color[1..], 16)?;

                        if color_code != role.color {
                            role_updates
                                .entry(usize::from_str(&role.id)?)
                                .or_insert_with(Vec::new)
                                .push(RoleUpdate::ChangeColor(color_code));
                        }
                    }
                } else {
                    new_roles.push(NewRole {
                        name: &discord_team.name,
                        color: if let Some(color) = discord_team.color.as_ref() {
                            usize::from_str_radix(&color[1..], 16)?
                        } else {
                            0
                        },
                        members: &discord_team.members,
                    });
                };
            }
        }

        Ok(min_managed_role_position)
    }
}

#[derive(PartialEq, Debug)]
enum UserUpdate {
    AddRole(usize),
    RemoveRole(usize),
}

enum RoleUpdate {
    ChangeColor(usize),
    ChangePosition(usize),
}

struct NewRole<'m> {
    name: &'m str,
    color: usize,
    members: &'m Vec<usize>,
}
