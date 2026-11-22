//! The audio side: LiveKit rooms, the tokens people connect with and what
//! a role may do on a connection.
//!
//! The database decides who is inside and in which role; LiveKit carries
//! the sound. Everything here turns a decision the database already holds
//! into LiveKit's terms:
//!
//! - a **token** for someone who enters ([`Spaces::credentials`]): the
//!   participant identity is the user ID, the name their display name, the
//!   metadata their role, and the grant what the role allows — listeners
//!   subscribe, the stage also publishes its microphone;
//! - a **permission update** when a role changes while connected
//!   ([`Spaces::apply_role`]), so it takes effect at once;
//! - a **removal** when someone leaves or is thrown out ([`Spaces::kick`]).
//!
//! Calls that only bring LiveKit in line with a change that is already
//! committed never fail the request: they are logged and the
//! reconciliation ([`crate::presence`]) repairs what was missed.
use std::time::Duration;

use tokuly_core::{
    livekit::{self, LiveKit, ParticipantUpdate, Permission, RoomOptions, TokenOptions, TrackSource},
    prelude::*,
};
use tokuly_proto::spaces as pb;

use crate::{
    Spaces,
    model::{Venue, role},
    rules,
};

/// Seconds LiveKit keeps a room nobody has connected to yet. Whoever
/// enters an empty room has it prepared again, so this only bounds how
/// long an unused room lingers.
const EMPTY_ROOM_TIMEOUT_SECONDS: u32 = 600;
/// Seconds LiveKit keeps a room after the last connection is gone: long
/// enough for a host on a shaky connection to come back to the same room.
const DEPARTURE_TIMEOUT_SECONDS: u32 = 60;

fn audio_not_configured() -> AppError {
    AppError::unavailable("AUDIO_NOT_CONFIGURED", "audio rooms are not configured on this server")
}

/// What the role of a participant means on a LiveKit connection: everyone
/// hears the room, the stage may publish its microphone. Nobody publishes
/// data (nothing here uses LiveKit's data channel).
///
/// Listeners are hidden: LiveKit tells the other clients in a room who is
/// connected, and who is merely listening is not everybody's to know (see
/// `list_participants`). Coming on stage lifts it.
pub(crate) fn permission(role: i16) -> Permission {
    let stage = rules::on_stage(role);
    Permission {
        can_subscribe: true,
        can_publish: stage,
        can_publish_data: false,
        can_publish_sources: vec![TrackSource::Microphone],
        hidden: !stage,
        can_update_metadata: false,
    }
}

/// The participant metadata clients read the role from.
pub(crate) fn metadata(role: i16) -> String {
    serde_json::json!({ "role": rules::role_name(role) }).to_string()
}

/// The token someone with `role` connects to `room` with.
pub(crate) fn token_options(room: &str, user_id: Uuid, name: &str, role: i16, ttl: Duration) -> TokenOptions {
    let permission = permission(role);
    TokenOptions::join(room, &user_id.to_string())
        .name(name)
        .metadata(metadata(role))
        .publish(permission.can_publish)
        .subscribe(permission.can_subscribe)
        .publish_data(permission.can_publish_data)
        .sources(&permission.can_publish_sources)
        .hidden(permission.hidden)
        .ttl(ttl)
}

impl Spaces {
    /// Whether this server can carry audio at all.
    pub fn audio_enabled(&self) -> bool {
        self.livekit.is_configured()
    }

    /// LiveKit, or `AUDIO_NOT_CONFIGURED`.
    pub(crate) fn require_audio(&self) -> AppResult<&LiveKit> {
        if self.livekit.is_configured() { Ok(&self.livekit) } else { Err(audio_not_configured()) }
    }

    /// Turns a failed LiveKit call a request depends on into the error
    /// clients are told: `AUDIO_NOT_CONFIGURED` or `AUDIO_UNAVAILABLE`.
    pub(crate) fn audio_error(error: AppError) -> AppError {
        if error.reason == livekit::NOT_CONFIGURED {
            return audio_not_configured();
        }
        tracing::error!(error = %error, "LiveKit call failed");
        AppError::unavailable("AUDIO_UNAVAILABLE", "audio is unavailable right now, try again in a moment")
    }

    /// Makes sure the LiveKit room of a venue exists, so its settings are
    /// ours rather than the server's defaults.
    pub(crate) async fn open_room(&self, venue: Venue) -> AppResult<()> {
        let options = RoomOptions::named(&venue.room())
            .empty_timeout(EMPTY_ROOM_TIMEOUT_SECONDS)
            .departure_timeout(DEPARTURE_TIMEOUT_SECONDS);
        self.require_audio()?.create_room(&options).await.map_err(Self::audio_error)?;
        Ok(())
    }

    /// What a client needs to connect to `venue` as `user_id` in `role`.
    pub(crate) fn credentials(
        &self,
        venue: Venue,
        user_id: Uuid,
        name: &str,
        role: i16,
    ) -> AppResult<pb::AudioCredentials> {
        let livekit = self.require_audio()?;
        let room = venue.room();
        let ttl = Duration::from_secs(self.cfg.token_ttl_seconds);
        let token = livekit.token(&token_options(&room, user_id, name, role, ttl)).map_err(Self::audio_error)?;
        Ok(pb::AudioCredentials {
            url: livekit.public_url().map_err(Self::audio_error)?.to_owned(),
            room,
            token,
            expires_at: ts(now() + chrono::Duration::seconds(self.cfg.token_ttl_seconds as i64)),
            can_publish: rules::on_stage(role),
            ..Default::default()
        })
    }

    /// Brings a connected participant's permissions and metadata in line
    /// with their role. Best effort; someone who is not connected (yet)
    /// gets the role through their token.
    pub(crate) async fn apply_role(&self, venue: Venue, user_id: Uuid, role: i16) {
        if !self.audio_enabled() {
            return;
        }
        let update =
            ParticipantUpdate { permission: Some(permission(role)), metadata: Some(metadata(role)), name: None };
        if let Err(e) = self.livekit.update_participant(&venue.room(), &user_id.to_string(), &update).await {
            tracing::warn!(?venue, %user_id, error = %e, "LiveKit permissions could not be updated; the reconciliation will retry");
        }
    }

    /// Disconnects someone from a venue's LiveKit room. Best effort.
    pub(crate) async fn kick(&self, venue: Venue, user_id: Uuid) {
        if !self.audio_enabled() {
            return;
        }
        if let Err(e) = self.livekit.remove_participant(&venue.room(), &user_id.to_string()).await {
            tracing::warn!(?venue, %user_id, error = %e, "participant could not be removed from LiveKit; the reconciliation will retry");
        }
    }

    /// Closes a venue's LiveKit room, disconnecting whoever is still
    /// connected. Returns whether LiveKit confirmed that the room is gone.
    pub(crate) async fn close_room(&self, venue: Venue) -> bool {
        if !self.audio_enabled() {
            return true;
        }
        match self.livekit.delete_room(&venue.room()).await {
            Ok(_) => true,
            Err(e) => {
                tracing::warn!(?venue, error = %e, "LiveKit room could not be closed; it will be retried");
                false
            }
        }
    }

    /// Mutes every microphone track someone publishes. Returns how many
    /// were muted.
    pub(crate) async fn mute_microphone(&self, venue: Venue, user_id: Uuid) -> AppResult<i32> {
        let livekit = self.require_audio()?;
        let room = venue.room();
        let identity = user_id.to_string();
        let Some(participant) = livekit.get_participant(&room, &identity).await.map_err(Self::audio_error)? else {
            return Ok(0);
        };
        let mut muted = 0;
        for track in participant.tracks.iter().filter(|track| track.kind == livekit::TrackKind::Audio && !track.muted) {
            if livekit.mute_track(&room, &identity, &track.sid, true).await.map_err(Self::audio_error)?.is_some() {
                muted += 1;
            }
        }
        Ok(muted)
    }
}

/// The role everyone has in a hangout's voice room: all of them may talk.
pub(crate) const HANGOUT_ROLE: i16 = role::SPEAKER;

#[cfg(test)]
mod tests {
    use tokuly_core::livekit::VideoGrant;

    use super::*;

    const SECRET: &str = "a-secret-of-at-least-thirty-two-characters";

    /// A LiveKit nothing listens behind: tokens are minted in-process.
    fn livekit() -> LiveKit {
        LiveKit::new("http://127.0.0.1:9", "ws://voice.example", "devkey", SECRET).unwrap()
    }

    fn grant_for(role: i16) -> (tokuly_core::livekit::Claims, Uuid) {
        let livekit = livekit();
        let user_id = new_id();
        let room = Venue::Space(new_id()).room();
        let token = livekit.token(&token_options(&room, user_id, "はなこ", role, Duration::from_secs(600))).unwrap();
        let claims = livekit.decode_token(&token).unwrap();
        assert_eq!(claims.video.room, room);
        (claims, user_id)
    }

    #[test]
    fn listeners_can_only_subscribe() {
        let (claims, user_id) = grant_for(role::LISTENER);
        assert_eq!(claims.sub, user_id.to_string(), "the participant identity is the user");
        assert_eq!(claims.name, "はなこ");
        assert_eq!(claims.metadata, r#"{"role":"listener"}"#);
        assert_eq!(
            claims.video,
            VideoGrant {
                room: claims.video.room.clone(),
                room_join: true,
                can_publish: Some(false),
                can_subscribe: Some(true),
                can_publish_data: Some(false),
                can_publish_sources: vec!["microphone".into()],
                hidden: true,
                ..Default::default()
            }
        );
    }

    #[test]
    fn the_stage_publishes_its_microphone_and_nothing_else() {
        for (role, name) in [(role::SPEAKER, "speaker"), (role::COHOST, "cohost"), (role::HOST, "host")] {
            let (claims, _) = grant_for(role);
            assert_eq!(claims.metadata, format!(r#"{{"role":"{name}"}}"#));
            assert_eq!(claims.video.can_publish, Some(true), "{name}");
            assert_eq!(claims.video.can_subscribe, Some(true), "{name}");
            assert_eq!(claims.video.can_publish_data, Some(false), "nothing uses the data channel: {name}");
            assert_eq!(claims.video.can_publish_sources, ["microphone"], "audio only: {name}");
            // Moderation goes through this API, never through LiveKit.
            assert!(
                !claims.video.room_admin
                    && !claims.video.room_create
                    && !claims.video.room_list
                    && !claims.video.hidden
            );
            assert!(claims.video.room_join);
        }
    }

    #[test]
    fn a_role_change_is_the_same_permission_the_token_would_carry() {
        for role in [role::LISTENER, role::SPEAKER, role::COHOST, role::HOST] {
            let options = token_options("room", new_id(), "", role, Duration::from_secs(60));
            assert_eq!(permission(role), Permission::of(&options), "role {role}");
        }
        assert!(!permission(role::LISTENER).can_publish && !permission(role::LISTENER).can_publish_data);
        assert!(permission(role::LISTENER).can_subscribe);
        assert!(permission(HANGOUT_ROLE).can_publish, "everyone talks in a hangout");
    }

    #[test]
    fn tokens_expire_when_asked() {
        let livekit = livekit();
        let token =
            livekit.token(&token_options("room", new_id(), "", role::LISTENER, Duration::from_secs(90))).unwrap();
        let claims = livekit.decode_token(&token).unwrap();
        let lifetime = claims.exp - claims.nbf;
        assert!((90..=105).contains(&lifetime), "{lifetime}");
    }
}
