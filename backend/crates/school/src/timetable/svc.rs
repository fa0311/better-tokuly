//! `TimetableService`: thin RPC handlers. Each one authenticates the
//! caller, parses the request and delegates to the module that owns the
//! behaviour (and the authorization that goes with it).
use std::time::Duration;

use chrono::NaiveDate;
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::school::*;

use super::{calendar::Calendar, grid, layout, resolve, terms};
use crate::{School, local, subjects};

const MAX_LISTED_DAYS: i64 = 62;

fn not_shared() -> AppError {
    AppError::not_found("TIMETABLE_NOT_SHARED", "this timetable is not shared with you")
}

impl School {
    /// Bounds how fast one account can write to its school data.
    pub(crate) async fn limit_writes(&self, user_id: Uuid) -> AppResult<()> {
        self.core.cache.rate_limit(&format!("school:write:{user_id}"), 240, Duration::from_secs(60)).await
    }

    /// Bounds how fast one account can read its own school data.
    pub(crate) async fn limit_reads(&self, user_id: Uuid) -> AppResult<()> {
        self.core.cache.rate_limit(&format!("school:read:{user_id}"), 600, Duration::from_secs(60)).await
    }

    /// Bounds the reads that load other people's timetables.
    async fn limit_friend_reads(&self, user_id: Uuid) -> AppResult<()> {
        self.core.cache.rate_limit(&format!("school:friends:{user_id}"), 120, Duration::from_secs(60)).await
    }

    /// The timetable of `owner` (the caller when the ID is empty) as the
    /// caller may see it, loaded for `dates`, or around now.
    async fn calendar_for(
        &self,
        viewer: Uuid,
        owner: &str,
        dates: Option<(NaiveDate, NaiveDate)>,
        at: Time,
    ) -> AppResult<Calendar> {
        let owner = parse_opt_id(owner)?.unwrap_or(viewer);
        self.limit_reads(viewer).await?;
        if owner != viewer {
            self.limit_friend_reads(viewer).await?;
        }
        let calendar = match dates {
            Some((start, end)) => Calendar::load(&self.core, owner, viewer, start, end).await?,
            None => Calendar::load_around(&self.core, owner, viewer, at).await?,
        };
        calendar.ok_or_else(not_shared)
    }
}

impl TimetableService for School {
    async fn list_layout_presets(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ListLayoutPresetsRequest>,
    ) -> ServiceResult<ListLayoutPresetsResponse> {
        handle(async {
            ctx.user_id()?;
            ok(ListLayoutPresetsResponse { presets: layout::preset_infos(), ..Default::default() })
        })
        .await
    }

    async fn list_subjects(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListSubjectsRequest>,
    ) -> ServiceResult<ListSubjectsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_reads(user_id).await?;
            let subjects = self.list_subjects(user_id, request.include_archived).await?;
            ok(ListSubjectsResponse { subjects, ..Default::default() })
        })
        .await
    }

    async fn create_subject(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateSubjectRequest>,
    ) -> ServiceResult<CreateSubjectResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_writes(user_id).await?;
            let subject = self.create_subject(user_id, &request.to_owned_message()).await?;
            ok(CreateSubjectResponse { subject: subject.to_proto().into(), ..Default::default() })
        })
        .await
    }

    async fn update_subject(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateSubjectRequest>,
    ) -> ServiceResult<UpdateSubjectResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_writes(user_id).await?;
            let subject = self.update_subject(user_id, &request.to_owned_message()).await?;
            ok(UpdateSubjectResponse { subject: subject.to_proto().into(), ..Default::default() })
        })
        .await
    }

    async fn delete_subject(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteSubjectRequest>,
    ) -> ServiceResult<DeleteSubjectResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_writes(user_id).await?;
            let (archived, removed_entry_count) = self.delete_subject(user_id, parse_id(request.subject_id)?).await?;
            self.reschedule_class_soon(user_id, self.now()).await?;
            ok(DeleteSubjectResponse { archived, removed_entry_count, ..Default::default() })
        })
        .await
    }

    async fn list_terms(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ListTermsRequest>,
    ) -> ServiceResult<ListTermsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_reads(user_id).await?;
            let rows = terms::of_user(&self.core, user_id).await?;
            let mut terms = Vec::with_capacity(rows.len());
            for row in &rows {
                terms.push(self.term_for_owner(row).await?);
            }
            ok(ListTermsResponse { terms, ..Default::default() })
        })
        .await
    }

    async fn get_term(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetTermRequest>,
    ) -> ServiceResult<GetTermResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_reads(user_id).await?;
            let term_id = parse_id(request.term_id)?;
            let mut conn = self.core.db.acquire().await?;
            let term = terms::own(&mut conn, user_id, term_id, false).await?;
            let data = grid::load_one(&mut conn, &term).await?;
            drop(conn);
            let subjects = subjects::map_of(&self.core, user_id).await?;
            ok(GetTermResponse {
                term: self.term_for_owner(&term).await?.into(),
                entries: data.entries.iter().map(|entry| grid::entry_proto(term_id, entry, &subjects, true)).collect(),
                changes: data.changes.iter().map(|change| grid::change_proto(term_id, change, &subjects)).collect(),
                day_overrides: data
                    .overrides
                    .iter()
                    .map(|day_override| grid::override_proto(term_id, day_override))
                    .collect(),
                ..Default::default()
            })
        })
        .await
    }

    async fn create_term(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateTermRequest>,
    ) -> ServiceResult<CreateTermResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_writes(user_id).await?;
            let term = self.create_term(user_id, &request.to_owned_message()).await?;
            self.reschedule_class_soon(user_id, self.now()).await?;
            ok(CreateTermResponse { term: self.term_for_owner(&term).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn update_term(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateTermRequest>,
    ) -> ServiceResult<UpdateTermResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_writes(user_id).await?;
            let (term, removed_entry_count) = self.update_term(user_id, &request.to_owned_message()).await?;
            self.reschedule_class_soon(user_id, self.now()).await?;
            ok(UpdateTermResponse {
                term: self.term_for_owner(&term).await?.into(),
                removed_entry_count,
                ..Default::default()
            })
        })
        .await
    }

    async fn delete_term(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteTermRequest>,
    ) -> ServiceResult<DeleteTermResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_writes(user_id).await?;
            self.delete_term(user_id, parse_id(request.term_id)?).await?;
            self.reschedule_class_soon(user_id, self.now()).await?;
            ok(DeleteTermResponse::default())
        })
        .await
    }

    async fn put_timetable_entries(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, PutTimetableEntriesRequest>,
    ) -> ServiceResult<PutTimetableEntriesResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_writes(user_id).await?;
            let request = request.to_owned_message();
            let (entries, replaced_entry_ids) =
                self.put_entries(user_id, parse_id(&request.term_id)?, &request.entries).await?;
            self.reschedule_class_soon(user_id, self.now()).await?;
            ok(PutTimetableEntriesResponse { entries, replaced_entry_ids, ..Default::default() })
        })
        .await
    }

    async fn delete_timetable_entries(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteTimetableEntriesRequest>,
    ) -> ServiceResult<DeleteTimetableEntriesResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_writes(user_id).await?;
            let entry_ids: Vec<Uuid> = request.entry_ids.iter().map(|id| parse_id(id)).collect::<AppResult<_>>()?;
            let deleted_count = self.delete_entries(user_id, parse_id(request.term_id)?, &entry_ids).await?;
            self.reschedule_class_soon(user_id, self.now()).await?;
            ok(DeleteTimetableEntriesResponse { deleted_count, ..Default::default() })
        })
        .await
    }

    async fn create_timetable_change(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateTimetableChangeRequest>,
    ) -> ServiceResult<CreateTimetableChangeResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_writes(user_id).await?;
            let change = self.create_change(user_id, &request.to_owned_message()).await?;
            self.reschedule_class_soon(user_id, self.now()).await?;
            ok(CreateTimetableChangeResponse { change: change.into(), ..Default::default() })
        })
        .await
    }

    async fn delete_timetable_change(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteTimetableChangeRequest>,
    ) -> ServiceResult<DeleteTimetableChangeResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_writes(user_id).await?;
            self.delete_change(user_id, parse_id(request.change_id)?).await?;
            self.reschedule_class_soon(user_id, self.now()).await?;
            ok(DeleteTimetableChangeResponse::default())
        })
        .await
    }

    async fn create_day_override(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateDayOverrideRequest>,
    ) -> ServiceResult<CreateDayOverrideResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_writes(user_id).await?;
            let day_override = self.create_day_override(user_id, &request.to_owned_message()).await?;
            self.reschedule_class_soon(user_id, self.now()).await?;
            ok(CreateDayOverrideResponse { day_override: day_override.into(), ..Default::default() })
        })
        .await
    }

    async fn delete_day_override(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteDayOverrideRequest>,
    ) -> ServiceResult<DeleteDayOverrideResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_writes(user_id).await?;
            self.delete_day_override(user_id, parse_id(request.day_override_id)?).await?;
            self.reschedule_class_soon(user_id, self.now()).await?;
            ok(DeleteDayOverrideResponse::default())
        })
        .await
    }

    async fn get_day(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetDayRequest>,
    ) -> ServiceResult<GetDayResponse> {
        handle(async {
            let viewer = ctx.user_id()?;
            let at = self.now();
            let date = local::parse_opt_date(request.date)?;
            let calendar = self.calendar_for(viewer, request.user_id, date.map(|date| (date, date)), at).await?;
            let date = date.unwrap_or_else(|| calendar.today(at));
            ok(GetDayResponse { day: calendar.day_proto(date).into(), ..Default::default() })
        })
        .await
    }

    async fn list_days(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListDaysRequest>,
    ) -> ServiceResult<ListDaysResponse> {
        handle(async {
            let viewer = ctx.user_id()?;
            let start = local::parse_date(request.start_date)?;
            let end = local::parse_date(request.end_date)?;
            ensure!(
                end >= start && (end - start).num_days() <= MAX_LISTED_DAYS,
                invalid,
                "DATE_RANGE_INVALID",
                "ask for up to {MAX_LISTED_DAYS} days after the start date"
            );
            let calendar = self.calendar_for(viewer, request.user_id, Some((start, end)), self.now()).await?;
            ok(ListDaysResponse {
                days: resolve::dates(start, end).map(|date| calendar.day_proto(date)).collect(),
                ..Default::default()
            })
        })
        .await
    }

    async fn get_now(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetNowRequest>,
    ) -> ServiceResult<GetNowResponse> {
        handle(async {
            let viewer = ctx.user_id()?;
            let at = self.now();
            let calendar = self.calendar_for(viewer, request.user_id, None, at).await?;
            ok(GetNowResponse { now: calendar.now_proto(at).into(), ..Default::default() })
        })
        .await
    }

    async fn list_friend_timetables(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListFriendTimetablesRequest>,
    ) -> ServiceResult<ListFriendTimetablesResponse> {
        handle(async {
            let viewer = ctx.user_id()?;
            self.limit_friend_reads(viewer).await?;
            let page = Page::new(
                request.page.as_option().map(|p| p.limit).unwrap_or(0),
                request.page.as_option().map(|p| p.cursor).unwrap_or(""),
                20,
                30,
            );
            let (timetables, page) = self.list_friend_timetables(viewer, &page).await?;
            ok(ListFriendTimetablesResponse { timetables, page: page.into(), ..Default::default() })
        })
        .await
    }

    async fn get_friend_timetable(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetFriendTimetableRequest>,
    ) -> ServiceResult<GetFriendTimetableResponse> {
        handle(async {
            let viewer = ctx.user_id()?;
            self.limit_friend_reads(viewer).await?;
            ok(self.friend_timetable(viewer, parse_id(request.user_id)?, parse_opt_id(request.term_id)?).await?)
        })
        .await
    }

    async fn copy_friend_timetable(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CopyFriendTimetableRequest>,
    ) -> ServiceResult<CopyFriendTimetableResponse> {
        handle(async {
            let viewer = ctx.user_id()?;
            self.core.cache.rate_limit(&format!("school:copy:{viewer}"), 20, Duration::from_secs(3600)).await?;
            let response = self.copy_friend_timetable(viewer, &request.to_owned_message()).await?;
            self.reschedule_class_soon(viewer, self.now()).await?;
            ok(response)
        })
        .await
    }

    async fn find_common_free_periods(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, FindCommonFreePeriodsRequest>,
    ) -> ServiceResult<FindCommonFreePeriodsResponse> {
        handle(async {
            let viewer = ctx.user_id()?;
            self.core.cache.rate_limit(&format!("school:free:{viewer}"), 30, Duration::from_secs(60)).await?;
            ok(self.common_free_periods(viewer, &request.to_owned_message()).await?)
        })
        .await
    }
}
