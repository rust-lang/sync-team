use crate::zulip::api::ZulipApi;
use crate::{get_env, run_diffs, ServiceDiff};
use sha2::{Digest, Sha256};

pub(crate) struct Confirmation {
    zulip: ZulipApi,
    stream: String,
    topic: String,
    base_url: String,

    diffs: Vec<ServiceDiff>,
    hash: String,
}

impl Confirmation {
    pub(crate) fn new(diffs: Vec<ServiceDiff>) -> anyhow::Result<Self> {
        let mut hash = Sha256::new();
        hash.update(&serde_json::to_vec(&diffs).unwrap());
        let hash = hex::encode(hash.finalize());

        Ok(Self {
            zulip: ZulipApi::new(
                get_env("ZULIP_USERNAME")?,
                get_env("ZULIP_API_TOKEN")?,
                false,
            ),
            stream: get_env("CONFIRMATION_STREAM")?,
            topic: get_env("CONFIRMATION_TOPIC")?,
            base_url: get_env("CONFIRMATION_BASE_URL")?,
            diffs,
            hash,
        })
    }

    pub(crate) fn run(self) -> anyhow::Result<()> {
        if let Ok(expected) = get_env("CONFIRMATION_EXPECTED_HASH") {
            let approver = get_env("CONFIRMATION_APPROVER")?;
            if self.hash == expected {
                run_diffs(self.diffs)?;
                self.zulip.post_message(
                    &self.stream,
                    &self.topic,
                    &format!("Applied diff `{expected}`\nApproved by: `{approver}`"),
                )?;
            } else {
                let mut message = String::new();
                message.push_str(
                    "🚨 **The diff changed since the approval, please approve again!**\n\n",
                );
                self.send_approval_message(&mut message)?;
            }
        } else {
            self.send_approval_message(&mut String::new())?;
        }

        Ok(())
    }

    fn send_approval_message(&self, buffer: &mut String) -> anyhow::Result<()> {
        for diff in &self.diffs {
            match diff {
                ServiceDiff::GitHub { diff, .. } => {
                    buffer.push_str("\n**GitHub:**\n```text\n");
                    buffer.push_str(&format!("{diff}"));
                    buffer.push_str("```")
                }
            }
        }

        buffer.push('\n');
        buffer.push_str(&format!("Hash: `{}`\n", self.hash));
        buffer.push_str(&format!(
            "[Approve]({}/{}) (requires authentication)\n",
            self.base_url, self.hash
        ));

        self.zulip.post_message(&self.stream, &self.topic, buffer)
    }
}
