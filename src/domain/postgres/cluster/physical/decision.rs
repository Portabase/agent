use super::manifest;
use crate::services::api::models::agent::backup::ParentOffer;
use chrono::{DateTime, Duration, Utc};

#[derive(Debug, PartialEq)]
pub enum Decision {
    Full {
        reason: String,
    },
    Incremental {
        parent_backup_id: String,
        parent_manifest: Vec<u8>,
    },
}

fn full(reason: impl Into<String>) -> Decision {
    Decision::Full {
        reason: reason.into(),
    }
}

pub fn decide(
    offer: Option<&ParentOffer>,
    now: DateTime<Utc>,
    full_every_days: u32,
    server_system_identifier: Option<u64>,
) -> Decision {
    let Some(offer) = offer else {
        return full("no parent offered (first backup or chain reset by the dashboard)");
    };
    if !offer.scheduled_fulls
        && now - offer.chain_started_at >= Duration::days(i64::from(full_every_days))
    {
        return full(format!(
            "chain is older than {full_every_days} day(s)"
        ));
    }
    let parent_manifest = match manifest::decode(&offer.manifest) {
        Ok(parent_manifest) => parent_manifest,
        Err(error) => return full(format!("parent manifest could not be decoded: {error:#}")),
    };
    if let (Some(server), Some(parent)) = (
        server_system_identifier,
        manifest::system_identifier(&parent_manifest),
    ) {
        if server != parent {
            return full("system identifier changed since the parent backup");
        }
    }
    Decision::Incremental {
        parent_backup_id: offer.backup_id.clone(),
        parent_manifest,
    }
}
