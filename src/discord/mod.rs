mod api;

use self::api::Discord;
use crate::TeamApi;
use failure::Error;
use log::{debug, info};

const RUST_LANG_DISCORD: &'static str = "The Rust Programming Language";
const TEST: &'static str = "Test";

pub(crate) struct SyncDiscord {
    discord: Discord,
    teams: Vec<rust_team_data::v1::Team>,
}

impl SyncDiscord {
    pub(crate) fn new(token: String, team_api: &TeamApi, dry_run: bool) -> Result<Self, Error> {
        let teams = team_api.get_teams()?;
        let discord = Discord::new(token, dry_run);

        Ok(Self { discord, teams })
    }

    pub(crate) fn run(&self) -> Result<(), Error> {
        dbg!(self.discord.get_roles(TEST)?);

        let teams = self.teams.iter().filter(|team| team.name == "community").collect::<Vec<_>>();
        dbg!(teams);
        Ok(())
    }
}
