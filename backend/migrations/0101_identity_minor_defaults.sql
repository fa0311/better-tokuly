-- The defaults for minors, applied once to the accounts under 18 today that
-- gave their birthdate before those defaults came. As
-- `settings::minor_defaults`, they only narrow: found and asked to be friends
-- by friends of friends at most, notes for followers at most, nearby discovery
-- off, the year of the birthday not shown to everyone. The other domains
-- apply theirs on `identity.user.birthdate_given`.
--
-- Accounts without a birthdate count as minors: `identity.user.privacy_changed`
-- for them (and for every bot, which goes by its owner) has search read their
-- settings again.

WITH minors AS (
    SELECT id,
           -- An empty object means the defaults (`settings::privacy_from_json`).
           CASE WHEN jsonb_typeof(privacy) = 'object' AND privacy <> '{}'::jsonb THEN privacy
                ELSE '{"friendRequestsFrom":"AUDIENCE_EVERYONE","friendListVisibleTo":"AUDIENCE_FRIENDS",
                       "footprintsVisibleTo":"AUDIENCE_FRIENDS","nearbyDiscoverable":true,"shareBattery":true,
                       "shareActivity":true,"shareOnlineStatus":true,"sharePlaceLabels":true,"shareNowPlaying":true,
                       "recordHistory":true,"shareViews":true,"directMessagesFrom":"AUDIENCE_FRIENDS",
                       "callsFrom":"AUDIENCE_FRIENDS","defaultNoteVisibility":"VISIBILITY_PUBLIC",
                       "birthdayVisibleTo":"AUDIENCE_NOBODY","discoverableTo":"AUDIENCE_EVERYONE"}'::jsonb
           END AS privacy
    FROM users
    WHERE birthdate > current_date - make_interval(years => 18) AND NOT bot AND deleted_at IS NULL
),
-- Settings saved before `discoverableTo` had the switch `discoverable`
-- (left out when off): on is everyone, off nobody.
upgraded AS (
    SELECT id,
           (privacy - 'discoverable') || jsonb_build_object('discoverableTo', coalesce(
               privacy ->> 'discoverableTo',
               CASE WHEN privacy -> 'discoverable' = 'true'::jsonb THEN 'AUDIENCE_EVERYONE' ELSE 'AUDIENCE_NOBODY' END
           )) AS privacy
    FROM minors
),
narrowed AS (
    UPDATE users SET privacy = upgraded.privacy
        || CASE WHEN upgraded.privacy ->> 'discoverableTo' = 'AUDIENCE_EVERYONE'
                THEN '{"discoverableTo":"AUDIENCE_FRIENDS_OF_FRIENDS"}'::jsonb ELSE '{}'::jsonb END
        || CASE WHEN upgraded.privacy ->> 'friendRequestsFrom' = 'AUDIENCE_EVERYONE'
                THEN '{"friendRequestsFrom":"AUDIENCE_FRIENDS_OF_FRIENDS"}'::jsonb ELSE '{}'::jsonb END
        || CASE WHEN coalesce(upgraded.privacy ->> 'defaultNoteVisibility', '') IN ('VISIBILITY_FRIENDS', 'VISIBILITY_PRIVATE')
                THEN '{}'::jsonb ELSE '{"defaultNoteVisibility":"VISIBILITY_FOLLOWERS"}'::jsonb END
        || CASE WHEN upgraded.privacy ->> 'birthdayVisibleTo' = 'AUDIENCE_EVERYONE'
                THEN '{"birthdayShowYear":false}'::jsonb ELSE '{}'::jsonb END
        || '{"nearbyDiscoverable":false}'::jsonb
    FROM upgraded
    WHERE users.id = upgraded.id
    RETURNING users.id
)
INSERT INTO outbox (topic, actor_id, data)
SELECT 'identity.user.birthdate_given', id, jsonb_build_object('user_id', id, 'minor', true) FROM narrowed
UNION ALL
SELECT 'identity.user.privacy_changed', id, jsonb_build_object('user_id', id) FROM narrowed;

INSERT INTO outbox (topic, actor_id, data)
SELECT 'identity.user.privacy_changed', id, jsonb_build_object('user_id', id)
FROM users
WHERE birthdate IS NULL AND deleted_at IS NULL;
