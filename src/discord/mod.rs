mod api;

use self::api::Discord;
use crate::TeamApi;
use failure::{bail, Error};
use log::{info, warn};
use std::collections::HashMap;

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
            teams,
            dry_run,
        })
    }

    pub(crate) fn run(&self) -> Result<(), Error> {
        let guild = self.discord.get_guild(RUST_LANG_DISCORD)?;

        let guild_id = guild.id;

        info!("Fetching users from discord...");
        let mut users = self.get_users(&guild_id)?;

        info!("Computing user updates...");
        let mut user_updates = HashMap::new();
        for (user_id, user) in &users {
            self.get_user_updates(*user_id, &mut user_updates, &user);
        }

        info!("Computing role updates...");
        let mut roles = guild.roles;
        let role_updates = self.get_role_updates(&roles)?;

        if !self.dry_run {
            info!("Applying user updates...");

            for (user_id, updates) in user_updates {
                let user = users.get_mut(&user_id).unwrap();

                let roles = &mut user.roles;

                for update in updates {
                    match update {
                        UserUpdate::AddRole(id) => {
                            roles.push(format!("{}", id));
                        }
                        UserUpdate::RemoveRole(id) => {
                            roles.retain(|role_id| role_id != &format!("{}", id));
                        }
                    }
                }

                self.discord.update_user_roles(&guild_id, user_id, roles)?;
            }

            info!("Applying role updates...");

            for (role_id, updates) in role_updates {
                let mut role = roles
                    .iter_mut()
                    .find(|role| role.id == format!("{}", role_id))
                    .unwrap();

                for update in updates {
                    match update {
                        RoleUpdate::ChangeColor(color) => {
                            role.color = color;
                        }
                    }
                }

                self.discord.update_guild_role(&guild_id, &role)?;
            }
        }

        Ok(())
    }

    fn get_users(&self, guild_id: &str) -> Result<HashMap<usize, api::GuildMember>, Error> {
        let mut users = HashMap::new();

        let maybe_discord = &self
            .teams
            .iter()
            .find(|team| team.name == "all")
            .map(|team| team.discord.as_ref())
            .flatten();

        if let Some(discord) = &maybe_discord {
            for member in &discord.members {
                match self.discord.get_member(*member, &guild_id) {
                    Ok(Some(guild_member)) => {
                        users.insert(*member, guild_member);
                    }
                    Ok(None) => {
                        warn!("user {} was not found in the guild", member);
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
    ) {
        let current_roles = &user.roles;

        for team in &self.teams {
            let discord_team = if let Some(discord_team) = &team.discord {
                discord_team
            } else {
                continue;
            };

            let team_members = &discord_team.members;
            let team_role_id = &discord_team.role_id;

            if team_members.contains(&user_id)
                && !current_roles.contains(&format!("{}", team_role_id))
            {
                user_updates
                    .entry(user_id)
                    .or_insert(vec![])
                    .push(UserUpdate::AddRole(*team_role_id));
            }

            if current_roles.contains(&format!("{}", team_role_id))
                && !team_members.contains(&user_id)
            {
                user_updates
                    .entry(user_id)
                    .or_insert(vec![])
                    .push(UserUpdate::RemoveRole(*team_role_id));
            }
        }
    }

    fn get_role_updates(
        &self,
        roles: &[api::Role],
    ) -> Result<HashMap<usize, Vec<RoleUpdate>>, Error> {
        let mut role_updates = HashMap::new();

        for team in &self.teams {
            let discord_team = if let Some(discord_team) = &team.discord {
                discord_team
            } else {
                continue;
            };

            let maybe_role = roles
                .iter()
                .find(|role| role.id == format!("{}", discord_team.role_id));

            if let (Some(role), Some(color)) = (maybe_role, discord_team.color.as_ref()) {
                let color_code = usize::from_str_radix(&color[1..], 16)?;

                if color_code != role.color {
                    role_updates
                        .entry(discord_team.role_id)
                        .or_insert(vec![])
                        .push(RoleUpdate::ChangeColor(color_code));
                }
            }
        }

        Ok(role_updates)
    }
}

#[derive(PartialEq, Debug)]
enum UserUpdate {
    AddRole(usize),
    RemoveRole(usize),
}

enum RoleUpdate {
    ChangeColor(usize),
}
