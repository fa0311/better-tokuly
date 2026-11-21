//! `HomeFeedService`: one mixed timeline of friends' activity for the home
//! screen.
//!
//! Every card points at something a client can open (a note, a story, a
//! video, a live stream, a bump, a new place...); this domain never loads
//! those things itself. Ranking is recency with a light touch:
//!
//! - bursts are collapsed and duplicates dropped while selecting (`cards`);
//! - inside a page, one person's cards are spread out so nobody fills the
//!   screen alone (`cards::spread`).
//!
//! Pages are cut in strict recency order *before* spreading, so the cursor
//! stays a plain keyset and no card is skipped or repeated across pages.
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::feed::*;

use crate::{
    Feed,
    activity::ActivityRow,
    cards::{CardQuery, spread},
};

const MAX_PAGE_SIZE: i64 = 50;

impl HomeFeedService for Feed {
    async fn list_home_feed(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListHomeFeedRequest>,
    ) -> ServiceResult<ListHomeFeedResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE_SIZE);
            let after: Option<(Time, Uuid)> = page.decode()?;
            let reader = self.reader_for(user_id).await?;
            let friends = self.visible_friends(user_id, &reader.hidden).await?;

            let mut heads = self
                .card_heads(&CardQuery {
                    authors: &friends,
                    hidden: &reader.hidden,
                    kind_prefix: None,
                    after,
                    limit: page.fetch(),
                    home: true,
                })
                .await?;
            let info = page.finish(&mut heads, ActivityRow::key);
            let bursts = spread(self.bursts(heads, &reader.hidden).await?, |burst| burst.head.user_id);
            ok(ListHomeFeedResponse {
                cards: self.to_cards(&reader, &bursts).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }
}
