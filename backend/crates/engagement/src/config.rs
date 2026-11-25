//! Engagement-specific settings.
//!
//! The server flattens every domain's settings into one command line, so
//! each argument carries an `engagement` prefix in its ID and its flag.
use clap::Args;

#[derive(Debug, Clone, Args)]
pub struct EngagementConfig {
    /// Comments (and replies) one user may write per minute.
    #[arg(
        id = "engagement_comments_per_minute",
        long = "engagement-comments-per-minute",
        env = "TOKULY_ENGAGEMENT_COMMENTS_PER_MINUTE",
        default_value_t = 10
    )]
    pub comments_per_minute: u32,

    /// Comments (and replies) one user may write per hour.
    #[arg(
        id = "engagement_comments_per_hour",
        long = "engagement-comments-per-hour",
        env = "TOKULY_ENGAGEMENT_COMMENTS_PER_HOUR",
        default_value_t = 120
    )]
    pub comments_per_hour: u32,

    /// Reports one user may file per hour.
    #[arg(
        id = "engagement_reports_per_hour",
        long = "engagement-reports-per-hour",
        env = "TOKULY_ENGAGEMENT_REPORTS_PER_HOUR",
        default_value_t = 20
    )]
    pub reports_per_hour: u32,

    /// Days after which an unanswered report no longer stops its reporter
    /// from reporting the same subject again.
    #[arg(
        id = "engagement_report_repeat_days",
        long = "engagement-report-repeat-days",
        env = "TOKULY_ENGAGEMENT_REPORT_REPEAT_DAYS",
        default_value_t = 7
    )]
    pub report_repeat_days: i32,

    /// Bookmark collections one user may have, counting the default one.
    #[arg(
        id = "engagement_max_collections",
        long = "engagement-max-collections",
        env = "TOKULY_ENGAGEMENT_MAX_COLLECTIONS",
        default_value_t = 100
    )]
    pub max_collections: i64,

    /// Subjects one bookmark collection may hold.
    #[arg(
        id = "engagement_max_bookmarks",
        long = "engagement-max-bookmarks",
        env = "TOKULY_ENGAGEMENT_MAX_BOOKMARKS",
        default_value_t = 5000
    )]
    pub max_bookmarks: i32,

    /// Distinct emoji one subject may carry. Once reached, only emoji
    /// already on it can be added (Discord's rule for messages).
    #[arg(
        id = "engagement_reaction_emoji_per_subject",
        long = "engagement-reaction-emoji-per-subject",
        env = "TOKULY_ENGAGEMENT_REACTION_EMOJI_PER_SUBJECT",
        default_value_t = 50
    )]
    pub reaction_emoji_per_subject: i32,

    /// Emoji one person may hold at once on a subject that takes several.
    #[arg(
        id = "engagement_reactions_per_person",
        long = "engagement-reactions-per-person",
        env = "TOKULY_ENGAGEMENT_REACTIONS_PER_PERSON",
        default_value_t = 20
    )]
    pub reactions_per_person: i32,

    /// Subject types on which one person may hold several distinct emoji
    /// (`SubjectType` names without the `SUBJECT_TYPE_` prefix, comma
    /// separated). Every other type takes one reaction per person.
    #[arg(
        id = "engagement_multi_reaction_types",
        long = "engagement-multi-reaction-types",
        env = "TOKULY_ENGAGEMENT_MULTI_REACTION_TYPES",
        value_delimiter = ',',
        default_values_t = MULTI_REACTION_TYPES.iter().map(|name| name.to_string()).collect::<Vec<_>>()
    )]
    pub multi_reaction_types: Vec<String>,
}

/// The default of `TOKULY_ENGAGEMENT_MULTI_REACTION_TYPES`: the subjects of
/// group conversation. See [`EngagementConfig::multi_reaction_types`].
pub const MULTI_REACTION_TYPES: &[&str] = &["COMMUNITY_THREAD", "COMMUNITY_REPLY", "COMMUNITY_EVENT", "PLAN"];

impl EngagementConfig {
    /// The subject type numbers of [`Self::multi_reaction_types`]; an error
    /// names an entry that is no subject type.
    pub fn multi_reaction_type_numbers(&self) -> anyhow::Result<Vec<i32>> {
        use buffa::Enumeration as _;
        use tokuly_proto::common::SubjectType;
        self.multi_reaction_types
            .iter()
            .map(|name| name.trim())
            .filter(|name| !name.is_empty())
            .map(|name| {
                let upper = name.to_ascii_uppercase();
                let full = if upper.starts_with("SUBJECT_TYPE_") { upper } else { format!("SUBJECT_TYPE_{upper}") };
                match SubjectType::from_proto_name(&full) {
                    Some(kind)
                        if kind != SubjectType::SUBJECT_TYPE_UNSPECIFIED
                            && kind != SubjectType::SUBJECT_TYPE_COMMENT =>
                    {
                        Ok(kind as i32)
                    }
                    _ => Err(anyhow::anyhow!(
                        "TOKULY_ENGAGEMENT_MULTI_REACTION_TYPES: {name:?} is not a subject type that takes emoji"
                    )),
                }
            })
            .collect()
    }
}

impl Default for EngagementConfig {
    fn default() -> Self {
        Self {
            comments_per_minute: 10,
            comments_per_hour: 120,
            reports_per_hour: 20,
            report_repeat_days: 7,
            max_collections: 100,
            max_bookmarks: 5000,
            reaction_emoji_per_subject: 50,
            reactions_per_person: 20,
            multi_reaction_types: MULTI_REACTION_TYPES.iter().map(|name| name.to_string()).collect(),
        }
    }
}

impl EngagementConfig {
    /// The settings the staff console (「サーバー設定」) may change, by
    /// `tokuly.toml` key. Everything else is set in tokuly.toml or the
    /// environment only: addresses, credentials, egress guards, paths,
    /// money and privacy floors never belong here (backend/README.md,
    /// "Declaring a setting").
    pub const CONSOLE_EDITABLE: &'static [&'static str] = &[
        "comments_per_minute",
        "comments_per_hour",
        "reports_per_hour",
        "report_repeat_days",
        "max_collections",
        "max_bookmarks",
        "reaction_emoji_per_subject",
        "reactions_per_person",
        "multi_reaction_types",
    ];
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        engagement: EngagementConfig,
    }

    #[test]
    fn command_line_defaults_match_default() {
        use clap::CommandFactory as _;
        Cli::command().debug_assert();
        let parsed = Cli::parse_from(["tokuly"]).engagement;
        let default = EngagementConfig::default();
        assert_eq!(
            (
                parsed.comments_per_minute,
                parsed.comments_per_hour,
                parsed.reports_per_hour,
                parsed.report_repeat_days,
                parsed.max_collections,
                parsed.max_bookmarks,
                parsed.reaction_emoji_per_subject,
                parsed.reactions_per_person,
                parsed.multi_reaction_types
            ),
            (
                default.comments_per_minute,
                default.comments_per_hour,
                default.reports_per_hour,
                default.report_repeat_days,
                default.max_collections,
                default.max_bookmarks,
                default.reaction_emoji_per_subject,
                default.reactions_per_person,
                default.multi_reaction_types
            )
        );
    }

    #[test]
    fn flags_are_prefixed() {
        let parsed =
            Cli::parse_from(["tokuly", "--engagement-comments-per-minute", "3", "--engagement-max-bookmarks", "9"])
                .engagement;
        assert_eq!((parsed.comments_per_minute, parsed.max_bookmarks), (3, 9));
    }

    #[test]
    fn multi_reaction_types_are_subject_type_names() {
        use tokuly_proto::common::SubjectType;
        let parsed =
            Cli::parse_from(["tokuly", "--engagement-multi-reaction-types", "plan, subject_type_space,"]).engagement;
        assert_eq!(
            parsed.multi_reaction_type_numbers().unwrap(),
            [SubjectType::SUBJECT_TYPE_PLAN as i32, SubjectType::SUBJECT_TYPE_SPACE as i32]
        );
        assert_eq!(EngagementConfig::default().multi_reaction_type_numbers().unwrap().len(), 4);
        for wrong in ["videos", "COMMENT", "UNSPECIFIED"] {
            let cfg = EngagementConfig { multi_reaction_types: vec![wrong.into()], ..Default::default() };
            assert!(cfg.multi_reaction_type_numbers().is_err(), "{wrong}");
        }
    }
}
