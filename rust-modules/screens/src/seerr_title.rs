//! **A title on the household's Seerr server** (docs/seerr.md): the page a Seerr card or a request
//! row opens (`ContentArg::SeerrTitle { kind, tmdb }`), the one place a request is made, and the
//! place a request manager approves or declines one.
//!
//! It reads the Seerr store's view (`SearchLike::seerr`) and asks it through `StoreCmd::Seerr`:
//! the title's detail, its scores and the first page of what TMDB recommends with it when it mounts,
//! the request when the pill is pressed, a decision when Approve or Decline is, the next page of
//! recommendations as the reader walks near that shelf's end, and the detail once more when a
//! request or a decision went through (so the page says what the server now says). A movie's pill
//! requests it; a show's requests every season the server neither holds nor was asked for. For a
//! user Seerr lets choose where a request goes, the pill opens the request menu
//! (`screens::seerr_request`) instead, which sends on the same slot. A title with nothing left to
//! request has no pill.
//!
//! The page is a `plx_ui::cards::Stack` of four sections ([`Sec`]), the backdrop standing behind
//! them and fading as the page scrolls:
//!
//! - **The hero** ([`Rows`]): the title, TMDB's facts (kind, year, runtime, score, genres, network or
//!   studio), the Rotten Tomatoes and IMDb scores, the availability, the newest request (its state,
//!   who asked and when, who answered), the download while one moves (with the amber band), a show's
//!   seasons as chips, each with its own state, the overview cut to the hero's lines, and the
//!   controls. For a user with Manage Requests (`SeerrStore::may_manage_requests`) and a request
//!   waiting for approval, Approve and Decline stand beside the pill; they answer the newest waiting
//!   request (`Slot::Decide`). An overview the hero cuts is a stop of its own above the controls,
//!   with the app's `MORE`; OK on it reads it in full, with its tagline, in the About sheet
//!   (`ContentPanel::SeerrAbout`).
//! - **Information** ([`info`]): release date, age rating, original language, who directed (or
//!   created) and wrote it, budget and box office, collection, and the streaming services in this
//!   television's country. It is read, not focused: it scrolls into view on the way to the shelves.
//! - **Cast & Crew**: the whole cast, a headshot each, the focused one naming the person and whom
//!   they play.
//! - **Related**: what TMDB recommends with it, as the Discover shelves' Request cards; a card opens
//!   its own page.
//!
//! The hero is laid out the way the catalog page's is (`detail_layout::TITLE_BOTTOM` and the
//! `theme::space` rungs, the backdrop through a household server's photo proxy, the shared scrim).
mod info;

use std::collections::HashMap;
use std::ffi::{CStr, CString};

use plx_appkit::hero_layout;
use plx_data::metadata::{Rating, RatingArt};
use plx_data::seerr::{
    AudienceVerdict, Availability, CastMember, CriticsVerdict, Decision, MediaKind, MediaRequest, RequestStatus,
    SeerrData, SeerrError, SeerrOp, TitleDetail, TitleRatings,
};
use plx_data::stores::seerr::{offer, Answer, Offer, SeerrCmd, SeerrStore, Slot};
use plx_data::stores::{StoreCmd, StoreId};
use plx_machine::machine::{
    Canon, Cx, Edge, Effects, EntryId, FocusKey, Fx, GroupId, Handled, InputKind, Key, LogicalState, Machine, Measure,
};
use plx_machine::present::Provenance;
use plx_platform::i18n::msg;
use plx_ui::cards::{CardEvent, CardSource, Kind, RowStyle, SectionSpec, Stack, StackEvent, StackPage, TileLabel};
use plx_ui::consts::{CARD_DY, MARGIN_X, SCR_W, TITLE_DY, UNDER_LABEL_AIR};
use plx_ui::hero_logo::{HeroLogo, LogoRung};
use plx_ui::hstack::{Flow, Item};
use plx_ui::label::{HAlign, Label, VAlign};
use plx_ui::screen::{
    AxisMask, Dir, DrawFrame, EdgeRule, ElemKind, FocusSource, Focusable, GroupKind, GroupSpec, HitSource,
    RenderStrategy, Screen, ScreenEvent, Seat,
};
use plx_ui::text_lift::TextLift;
use plx_ui::vstack::VFlow;
use plx_ui::widgets::{Art, Button, ControlGround, CtlPop, StatusKind, StatusOverlay};
use plx_ui::{detail_layout, fmt, theme, Env, Painter, Rect, View};

use crate::registry::seerr_cards::{
    answered_line, asked_line, dotted, download_line, request_chip, season_state, RequestCard, RequestCards,
};
use crate::registry::{AppFx, ContentArg, ContentPanel, ContentReq, SearchLike};

/// The controls, by element; each is also its pop slot.
pub const PILL_ELEM: u32 = 0;
pub const APPROVE_ELEM: u32 = 1;
pub const DECLINE_ELEM: u32 = 2;
/// The overview, a stop of its own while the hero cuts it.
pub const SYNOPSIS_ELEM: u32 = 4;
/// Cast member `i`'s element is this plus `i`.
const FIRST_CAST_ELEM: u32 = 0x100;
/// The first related title's element; they count up from here, never reused within a page.
const FIRST_RELATED_ELEM: u32 = 0x10_0000;

const HERO_GROUP: GroupId = GroupId(0);
const INFO_GROUP: GroupId = GroupId(1);
const CAST_GROUP: GroupId = GroupId(2);
const RELATED_GROUP: GroupId = GroupId(3);

/// The download band under its line: the card band's height.
const BAND_H: f32 = 5.0;
/// A season chip's line, and the air between two lines of chips.
const CHIP_GAP: f32 = theme::space::XS;
/// Air between the overview's last line and the `MORE` pinned on it (the About card's).
const MORE_GAP: f32 = theme::space::LG;
/// A shelf's heading band (the Person page's: title rung and the gap to the cards).
const SHELF_HEADING: f32 = TITLE_DY + CARD_DY;
static CAST_STYLE: RowStyle = RowStyle::CAST;
static RELATED_STYLE: RowStyle = RowStyle::HOME;
/// The scroll over which the backdrop fades out (Detail's hero fade).
const BACKDROP_FADE: f32 = 400.0;

/// The page's sections, in document order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sec {
    Hero,
    Info,
    Cast,
    Related,
}

/// The status line: what the last decision or request said, else what the server says of the title.
pub fn status_line(detail: &TitleDetail, request: Option<&Answer>, decision: Option<&Answer>) -> Option<&'static str> {
    match decision {
        Some(Answer::Failed(SeerrError::Forbidden)) => return Some(msg::seerr_decide_forbidden()),
        Some(Answer::Failed(SeerrError::AlreadyRequested)) => return Some(msg::seerr_decide_conflict()),
        Some(Answer::Failed(_)) => return Some(msg::seerr_decide_failed()),
        _ => {}
    }
    match request {
        Some(Answer::Ready(SeerrData::Requested)) => return Some(msg::seerr_request_done()),
        Some(Answer::Failed(SeerrError::AlreadyRequested)) => return Some(msg::seerr_request_already()),
        Some(Answer::Failed(SeerrError::Forbidden)) => return Some(msg::seerr_request_forbidden()),
        Some(Answer::Failed(_)) => return Some(msg::seerr_request_failed()),
        _ => {}
    }
    match detail.title.status {
        Availability::Pending => Some(msg::seerr_status_pending()),
        Availability::Processing => Some(msg::seerr_status_processing()),
        Availability::PartlyAvailable => Some(msg::seerr_status_partly()),
        Availability::Available => Some(msg::seerr_status_available()),
        Availability::Blocked => Some(msg::seerr_status_blocked()),
        Availability::Unknown => None,
    }
}

/// TMDB's facts as the meta line says them: kind, year, runtime, two genres, network or studio, each
/// left out when the server did not say. TMDB's score stands in the ratings row ([`ratings`]), named.
pub fn facts(kind: MediaKind, detail: &TitleDetail) -> Vec<String> {
    let f = &detail.facts;
    let mut out = vec![match kind {
        MediaKind::Movie => msg::browse_kind_movie().to_string(),
        MediaKind::Tv => msg::browse_kind_tv_show().to_string(),
    }];
    out.extend(detail.title.year.filter(|&y| y > 0).map(|y| y.to_string()));
    out.extend(f.runtime_min.map(|m| fmt::duration_minutes_in(plx_platform::i18n::current(), i64::from(m))));
    out.extend(f.genres.iter().take(2).cloned());
    out.extend(f.studio.clone());
    out
}

/// The title's scores as the shared ratings row reads them, in provider order: Seerr's Rotten Tomatoes
/// (the critics' tomato, the audience's popcorn) and IMDb, then TMDB's own (`tmdb`, out of 10).
pub fn ratings(r: &TitleRatings, tmdb: Option<f32>) -> Vec<Rating> {
    let mut out = Vec::new();
    if let Some((score, verdict)) = r.critics {
        let art = match verdict {
            CriticsVerdict::Certified => RatingArt::TomatoCertified,
            CriticsVerdict::Fresh => RatingArt::TomatoFresh,
            CriticsVerdict::Rotten => RatingArt::TomatoRotten,
        };
        out.push(Rating { art, value: f64::from(score) / 10.0, critic: true });
    }
    if let Some((score, verdict)) = r.audience {
        let art = match verdict {
            AudienceVerdict::Upright => RatingArt::PopcornUpright,
            AudienceVerdict::Spilled => RatingArt::PopcornSpilled,
        };
        out.push(Rating { art, value: f64::from(score) / 10.0, critic: false });
    }
    if let Some(score) = r.imdb {
        out.push(Rating { art: RatingArt::Imdb, value: f64::from(score), critic: false });
    }
    if let Some(score) = tmdb {
        out.push(Rating { art: RatingArt::Tmdb, value: f64::from(score), critic: false });
    }
    out
}

/// The newest request of the title, and the newest still waiting for approval.
fn newest(detail: &TitleDetail) -> Option<&MediaRequest> {
    detail.requests.first()
}

pub fn pending(detail: &TitleDetail) -> Option<&MediaRequest> {
    detail.requests.iter().find(|r| r.status == RequestStatus::PendingApproval)
}

/// The request line: the newest request's state, who asked and when, who answered.
pub fn request_line(detail: &TitleDetail) -> Option<String> {
    let request = newest(detail)?;
    Some(dotted(&[request_chip(request), &asked_line(request), &answered_line(request)]))
}

/// A show's seasons as their chips say them: "Season 1 · Available".
pub fn season_chips(detail: &TitleDetail) -> Vec<String> {
    detail.seasons.iter().map(|s| dotted(&[&fmt::season_identity(i64::from(s.number), ""), season_state(s.status)])).collect()
}

/// Chips `widths` wide, laid in lines no wider than `w` from `x`: each chip's `(x, line)`.
fn chip_lines(widths: &[f32], x: f32, w: f32) -> Vec<(f32, usize)> {
    let (mut out, mut line, mut flow) = (Vec::new(), 0, Flow::new(x, theme::space::XS));
    for &cw in widths {
        if flow.next_x() > x && flow.next_x() + cw > x + w {
            (line, flow) = (line + 1, Flow::new(x, theme::space::XS));
        }
        out.push((flow.push(Item::new(cw)).map_or(x, |s| s.x), line));
    }
    out
}

/// The overview as the hero draws it: its lines, faded for a `MORE` when it is cut. ONE builder for
/// the height the hero reserves, the cut it reports and the block it paints.
fn overview<'a>(summary: &'a str, measure: &'a dyn Measure) -> plx_ui::text_view::TextView<'a> {
    plx_ui::hero_synopsis(summary, "").with_measure(measure).fade_for_more(MORE_GAP)
}

/// The hero's rows, each a named rung under the last one's measured bottom (page space: the hero is
/// the page's first section and starts at its top).
pub struct Rows {
    meta: f32,
    ratings: Option<f32>,
    status: Option<f32>,
    request: Option<f32>,
    download: Option<f32>,
    seasons: Option<f32>,
    summary: f32,
    summary_h: f32,
    pill: f32,
}

impl Rows {
    /// The hero's height: its controls and the air under them.
    fn height(&self) -> f32 {
        self.pill + hero_layout::CD + theme::space::XL
    }
}

/// What the rows hold, measured.
struct Content<'a> {
    ratings: bool,
    status: bool,
    request: bool,
    download: bool,
    chip_lines: usize,
    summary: &'a str,
}

fn rows(c: &Content<'_>, measure: &dyn Measure) -> Rows {
    let line = |on: bool, sz| if on { Item::new(measure.cap_h(sz)) } else { Item::absent() };
    let mut flow = VFlow::new(detail_layout::TITLE_BOTTOM, theme::space::SM);
    flow.push(Item::new(0.0));
    let meta = flow.push(Item::new(measure.cap_h(theme::size::BODY))).map_or(0.0, |s| s.y);
    let ratings = flow.push(line(c.ratings, theme::size::LABEL)).map(|s| s.y);
    let status = flow.push(line(c.status, theme::size::LABEL)).map(|s| s.y);
    let request = flow.push(line(c.request, theme::size::LABEL)).map(|s| s.y);
    let download = flow.push(if c.download {
        Item::new(measure.cap_h(theme::size::LABEL) + theme::space::XS + BAND_H)
    } else {
        Item::absent()
    }).map(|s| s.y);
    let chip_h = measure.cap_h(theme::size::CAPTION) + 2.0 * 6.0;
    let seasons = flow.push(if c.chip_lines > 0 {
        Item::new(c.chip_lines as f32 * chip_h + (c.chip_lines - 1) as f32 * CHIP_GAP)
    } else {
        Item::absent()
    }).map(|s| s.y);
    let summary_h = if c.summary.is_empty() { 0.0 } else { overview(c.summary, measure).measure_h(detail_layout::HERO_TEXT_W) };
    let summary_item = if c.summary.is_empty() { Item::new(0.0).before(0.0) } else { Item::new(summary_h).before(theme::space::MD) };
    let summary = flow.push(summary_item).map_or(0.0, |s| s.y);
    let pill = flow.push(Item::new(hero_layout::CD).before(theme::space::LG)).map_or(0.0, |s| s.y);
    Rows { meta, ratings, status, request, download, seasons, summary, summary_h, pill }
}

/// The cast shelf's cards: a headshot each, through the household server's photo proxy.
struct CastCards<'a> {
    cast: &'a [CastMember],
    /// Each member's photo URL ([`plx_plex::plex::seerr::tmdb_image`]), parallel to `cast`.
    photos: &'a [Option<String>],
    art_server: Option<u16>,
}

impl<H: plx_machine::machine::Host<Elem = u32>> CardSource<H> for CastCards<'_> {
    fn len(&self) -> usize {
        self.cast.len().min(self.photos.len())
    }
    fn elem(&self, i: usize) -> u32 {
        FIRST_CAST_ELEM + i as u32
    }
    fn index_of(&self, e: &u32) -> Option<usize> {
        e.checked_sub(FIRST_CAST_ELEM).map(|i| i as usize).filter(|&i| i < CardSource::<H>::len(self))
    }
    /// A member TMDB has no photo of draws the shared person glyph (`Art::Person` with no key).
    fn art(&self, i: usize) -> Art<'_> {
        match (self.photos.get(i).and_then(Option::as_deref), self.art_server) {
            (Some(url), Some(sid)) => Art::Person { sid, key: url, res: (300, 300) },
            _ => Art::Person { sid: 0, key: "", res: (300, 300) },
        }
    }
    fn label(&self, i: usize) -> TileLabel {
        self.cast.get(i).map_or_else(TileLabel::default, |c| TileLabel::titled(&c.name, &c.character))
    }
}

/// The page's state as the stack reads it: the identity and the decisions it holds, the controls'
/// motion, and what it last read off the store.
struct Page {
    entry: EntryId,
    kind: MediaKind,
    tmdb: i64,
    /// The request answer the detail was last read again for, so a request that went through
    /// refreshes the detail once.
    refreshed: bool,
    /// The request this page last approved or declined, and whether the detail was read again once
    /// that answer landed.
    deciding: Option<i64>,
    decided: bool,
    pop: CtlPop<3>,
    summary_lift: TextLift,
    /// The hero's stops, the controls first, then the overview while it is cut: what `plain_*`
    /// reads, kept by [`Page::sync`].
    elems: Vec<u32>,
    /// The Information block's pairs, and the cast's photo URLs, kept by [`Page::sync`].
    info: Vec<(&'static str, String)>,
    photos: Vec<Option<String>>,
    related: Vec<RequestCard>,
    related_elems: Vec<u32>,
    related_read: u32,
    related_more: bool,
    keys: HashMap<(MediaKind, i64), u32>,
    next: u32,
    /// What the last [`Page::sync`] read: the store's generation, the language, the decision.
    synced: Option<(u32, usize, Option<i64>, bool)>,
    rev: u64,
}

impl Page {
    fn new(entry: EntryId, kind: MediaKind, tmdb: i64) -> Self {
        Self {
            entry, kind, tmdb, refreshed: false, deciding: None, decided: false, pop: CtlPop::new(),
            summary_lift: TextLift::new(), elems: Vec::new(), info: Vec::new(), photos: Vec::new(),
            related: Vec::new(), related_elems: Vec::new(), related_read: 0, related_more: false,
            keys: HashMap::new(), next: FIRST_RELATED_ELEM, synced: None, rev: 0,
        }
    }

    fn detail_slot(&self) -> Slot {
        Slot::Detail(self.kind, self.tmdb)
    }

    fn request_slot(&self) -> Slot {
        Slot::Request(self.kind, self.tmdb)
    }

    /// The detail this page asked for, once it has landed.
    fn detail<'a>(&self, seerr: Option<&'a SeerrStore>) -> Option<&'a TitleDetail> {
        seerr?.title_detail(self.kind, self.tmdb)
    }

    fn scores(&self, seerr: Option<&SeerrStore>) -> Vec<Rating> {
        let tmdb = self.detail(seerr).and_then(|d| d.facts.rating);
        match seerr.and_then(|s| s.answer(&Slot::Ratings(self.kind, self.tmdb))) {
            Some(Answer::Ready(SeerrData::Ratings(r))) => ratings(r, tmdb),
            _ => ratings(&TitleRatings::default(), tmdb),
        }
    }

    fn request<'a>(&self, seerr: Option<&'a SeerrStore>) -> Option<&'a Answer> {
        seerr?.answer(&self.request_slot())
    }

    /// The answer to this page's last decision.
    fn decision<'a>(&self, seerr: Option<&'a SeerrStore>) -> Option<&'a Answer> {
        seerr?.answer(&Slot::Decide(self.deciding?))
    }

    fn pill(&self, seerr: Option<&SeerrStore>) -> Offer {
        self.detail(seerr).map_or(Offer::Nothing, |detail| offer(detail, self.request(seerr)))
    }

    /// The request Approve and Decline answer: the newest one waiting, for a user who may decide,
    /// unless this page's decision on it already went through.
    fn decidable<'a>(&self, seerr: Option<&'a SeerrStore>) -> Option<&'a MediaRequest> {
        let store = seerr?;
        if !store.may_manage_requests() {
            return None;
        }
        let request = pending(self.detail(seerr)?)?;
        let done = self.deciding == Some(request.id) && matches!(self.decision(seerr), Some(Answer::Ready(SeerrData::Decided)));
        (!done).then_some(request)
    }

    /// Which decision is on its way, if one is.
    fn sending(&self, seerr: Option<&SeerrStore>) -> Option<Decision> {
        let id = self.deciding?;
        if !matches!(self.decision(seerr), Some(Answer::Loading)) {
            return None;
        }
        match seerr?.asked(&Slot::Decide(id)) {
            Some(SeerrOp::Decide { decision, .. }) => Some(*decision),
            _ => None,
        }
    }

    /// The controls on the row, left to right, with their labels.
    fn controls(&self, seerr: Option<&SeerrStore>) -> Vec<(u32, &'static CStr)> {
        let mut out = Vec::new();
        let state = self.pill(seerr);
        if state != Offer::Nothing {
            let label = match (&state, self.kind) {
                (Offer::Sending, _) => msg::seerr_title_requesting_c(),
                (_, MediaKind::Tv) => msg::seerr_title_request_seasons_c(),
                _ => msg::seerr_title_request_c(),
            };
            out.push((PILL_ELEM, label));
        }
        if self.decidable(seerr).is_some() {
            let sending = self.sending(seerr);
            out.push((APPROVE_ELEM, if sending == Some(Decision::Approve) {
                msg::seerr_decide_approving_c()
            } else {
                msg::seerr_decide_approve_c()
            }));
            out.push((DECLINE_ELEM, if sending == Some(Decision::Decline) {
                msg::seerr_decide_declining_c()
            } else {
                msg::seerr_decide_decline_c()
            }));
        }
        out
    }

    fn rows(&self, seerr: Option<&SeerrStore>, measure: &dyn Measure) -> Option<Rows> {
        let detail = self.detail(seerr)?;
        let chips = season_chips(detail);
        let widths: Vec<f32> = chips.iter().map(|c| plx_ui::widgets::keyline_chip_w(c, measure)).collect();
        let chip_lines = chip_lines(&widths, MARGIN_X, detail_layout::HERO_TEXT_W)
            .last().map_or(0, |(_, line)| line + 1);
        Some(rows(&Content {
            ratings: !self.scores(seerr).is_empty(),
            status: status_line(detail, self.request(seerr), self.decision(seerr)).is_some(),
            request: request_line(detail).is_some(),
            download: download_line(&detail.downloads).is_some(),
            chip_lines,
            summary: detail.title.overview.as_deref().unwrap_or(""),
        }, measure))
    }

    /// Whether the hero cuts the overview: then it is a stop, and OK reads it in full.
    fn overview_cut(&self, seerr: Option<&SeerrStore>, measure: &dyn Measure) -> bool {
        self.detail(seerr).and_then(|d| d.title.overview.as_deref())
            .is_some_and(|s| !s.is_empty() && overview(s, measure).truncates(detail_layout::HERO_TEXT_W))
    }

    /// Each control's rect in page space, in the row's order.
    fn control_rects(&self, seerr: Option<&SeerrStore>, measure: &dyn Measure) -> Vec<(u32, Rect)> {
        let controls = self.controls(seerr);
        let Some(rows) = self.rows(seerr, measure).filter(|_| !controls.is_empty()) else { return Vec::new() };
        let widths: Vec<f32> = controls.iter()
            .map(|(_, label)| hero_layout::pill_w(measure, label, theme::size::BODY, true, None).max(hero_layout::PW))
            .collect();
        controls.iter().enumerate().map(|(i, (elem, _))| (*elem, hero_layout::row_rect(&widths, i, rows.pill))).collect()
    }

    fn control_rect(&self, elem: u32, seerr: Option<&SeerrStore>, measure: &dyn Measure) -> Option<Rect> {
        self.control_rects(seerr, measure).into_iter().find(|(e, _)| *e == elem).map(|(_, r)| r)
    }

    /// The overview's lifted plate in page space.
    fn overview_plate(rows: &Rows) -> Rect {
        Rect::new(MARGIN_X - theme::space::SM, rows.summary - theme::space::SM,
            detail_layout::HERO_TEXT_W + 2.0 * theme::space::SM, rows.summary_h + 2.0 * theme::space::SM)
    }

    /// Bring what the page keeps from the store up to it: the hero's stops, the Information pairs,
    /// the cast's photos and the Related cards. Only when the store, the language or the decision
    /// moved; a change in what the sections read moves the revision.
    fn sync(&mut self, seerr: Option<&SeerrStore>, art: impl Fn(&str) -> Option<String>, measure: &dyn Measure) {
        let stamp = (seerr.map_or(0, SeerrStore::gen), std::ptr::from_ref(plx_platform::i18n::current()) as usize,
            self.deciding, self.decided);
        if self.synced == Some(stamp) {
            return;
        }
        self.synced = Some(stamp);
        let mut elems: Vec<u32> = self.controls(seerr).into_iter().map(|(elem, _)| elem).collect();
        if self.overview_cut(seerr, measure) {
            elems.push(SYNOPSIS_ELEM);
        }
        let detail = self.detail(seerr);
        let info = detail.map(|d| info::pairs(self.kind, d, &info::region())).unwrap_or_default();
        let photos: Vec<Option<String>> = detail.map_or_else(Vec::new, |d| {
            d.cast.iter().map(|c| c.profile.as_deref().and_then(&art)).collect()
        });
        let (related, read, more) = seerr.map_or((Vec::new(), 0, false), |s| self.related_of(s));
        let moved = elems != self.elems || info.len() != self.info.len() || photos.len() != self.photos.len()
            || related.len() != self.related.len();
        self.elems = elems;
        self.info = info;
        self.photos = photos;
        if related != self.related || (read, more) != (self.related_read, self.related_more) {
            self.related_elems = related.iter().map(|c| self.intern(c.kind, c.tmdb)).collect();
            (self.related, self.related_read, self.related_more) = (related, read, more);
        }
        if moved {
            self.rev += 1;
        }
    }

    /// The recommendations the landed pages hold, a title once, this one left out: the cards, the
    /// pages read and whether the server has more.
    fn related_of(&self, seerr: &SeerrStore) -> (Vec<RequestCard>, u32, bool) {
        let (mut cards, mut read, mut more) = (Vec::<RequestCard>::new(), 0, false);
        while let Some(Answer::Ready(SeerrData::Titles(page))) =
            seerr.answer(&Slot::Recommendations(self.kind, self.tmdb, read + 1))
        {
            read += 1;
            more = read < page.pages;
            for title in &page.items {
                let Some(card) = RequestCard::of(title) else { continue };
                if (card.kind, card.tmdb) != (self.kind, self.tmdb) && !cards.iter().any(|c| (c.kind, c.tmdb) == (card.kind, card.tmdb)) {
                    cards.push(card);
                }
            }
        }
        (cards, read, more)
    }

    fn intern(&mut self, kind: MediaKind, tmdb: i64) -> u32 {
        let next = &mut self.next;
        *self.keys.entry((kind, tmdb)).or_insert_with(|| {
            let elem = *next;
            *next += 1;
            elem
        })
    }

    /// The related title `elem` stands for.
    fn related_of_elem(&self, elem: u32) -> Option<&RequestCard> {
        self.related_elems.iter().position(|e| *e == elem).and_then(|i| self.related.get(i))
    }

    fn hero_draw<H: SearchLike>(&self, f: &mut DrawFrame<'_, '_, H>, p: Painter, detail: &TitleDetail) {
        let seerr = H::seerr(f.cx);
        let measure = f.measure;
        let Some(rows) = self.rows(seerr, measure) else { return };
        let x = MARGIN_X;
        let band_h = plx_ui::hero_logo::band_h(LogoRung::Hero);
        HeroLogo::new(0, "", &detail.title.title, LogoRung::Hero)
            .draw(p, Rect::new(x, detail_layout::TITLE_BOTTOM - band_h, detail_layout::HERO_TEXT_W, band_h), measure);

        // Whole facts drop off the end until the line fits the hero's text column: a genre or a
        // studio is the server's word and may be long, and half a fact says nothing.
        let meta = facts(self.kind, detail);
        let mut runs: Vec<&str> = meta.iter().map(String::as_str).collect();
        let run = plx_ui::widgets::DottedRun::new(theme::size::BODY, theme::TEXT_SECONDARY, theme::space::SM).measured(measure);
        while runs.len() > 1 && run.width(measure, &runs) > detail_layout::HERO_TEXT_W {
            runs.pop();
        }
        let (cap_top, _) = plx_gfx::text::text_cap_band(theme::size::BODY, 0);
        run.draw(p, &runs, x, rows.meta - cap_top);

        if let Some(y) = rows.ratings {
            crate::registry::captions::draw_ratings(p, &self.scores(seerr), x, y, x + detail_layout::HERO_TEXT_W, measure);
        }
        if let (Some(y), Some(text)) = (rows.status, status_line(detail, self.request(seerr), self.decision(seerr))) {
            line(p, text, y, true, theme::TEXT_PRIMARY, measure);
        }
        if let (Some(y), Some(text)) = (rows.request, request_line(detail)) {
            line(p, &text, y, false, theme::TEXT_SECONDARY, measure);
        }
        if let (Some(y), Some(text), Some(progress)) =
            (rows.download, download_line(&detail.downloads), plx_data::seerr::progress(&detail.downloads))
        {
            line(p, &text, y, false, theme::TEXT_SECONDARY, measure);
            let top = y + measure.cap_h(theme::size::LABEL) + theme::space::XS;
            plx_ui::widgets::progress_bar(p, Rect::new(x, top, detail_layout::HERO_TEXT_W, BAND_H), BAND_H * 0.5, BAND_H,
                progress.fraction);
        }
        if let Some(y) = rows.seasons {
            let chips = season_chips(detail);
            let widths: Vec<f32> = chips.iter().map(|c| plx_ui::widgets::keyline_chip_w(c, measure)).collect();
            let chip_h = measure.cap_h(theme::size::CAPTION) + 2.0 * 6.0;
            for (chip, (cx, line)) in chips.iter().zip(chip_lines(&widths, x, detail_layout::HERO_TEXT_W)) {
                let cy = y + line as f32 * (chip_h + CHIP_GAP) + chip_h * 0.5;
                plx_ui::widgets::keyline_chip(p, cx, cy, chip, theme::TEXT_SECONDARY, measure);
            }
        }
        if let Some(summary) = detail.title.overview.as_deref().filter(|s| !s.is_empty()) {
            let view = overview(summary, measure);
            let cut = self.elems.contains(&SYNOPSIS_ELEM);
            let focused = self.focused(f.cx.focus.current, SYNOPSIS_ELEM);
            plx_ui::text_lift::draw_focused(p, Self::overview_plate(&rows), plx_ui::widgets::TEXT_BLOCK_HL_RAD,
                &self.summary_lift, plx_ui::text_lift::CENTRE, |p| {
                    view.draw(p, Rect::new(x, rows.summary, detail_layout::HERO_TEXT_W, rows.summary_h));
                    if cut {
                        view.draw_more(p, x, rows.summary, detail_layout::HERO_TEXT_W, rows.summary_h, focused);
                    }
                });
        }

        let labels = self.controls(seerr);
        for (elem, rect) in self.control_rects(seerr, measure) {
            let Some((_, label)) = labels.iter().find(|(e, _)| *e == elem) else { continue };
            Button::new(label.as_ptr(), theme::size::BODY, rect)
                .focused(self.focused(f.cx.focus.current, elem))
                .ground(ControlGround::Keyed)
                .scale(self.pop.scale(elem as usize))
                .draw(&Env::inert(), p);
        }
    }

    fn focused(&self, focus: Option<FocusKey<u32>>, elem: u32) -> bool {
        focus.is_some_and(|k| k.entry == self.entry && k.elem == elem)
    }
}

fn line(p: Painter, text: &str, y: f32, bold: bool, ink: [f32; 4], measure: &dyn Measure) {
    let w = detail_layout::HERO_TEXT_W;
    let fitted = plx_machine::machine::elide_by(text, w, false, |s| measure.width_str(s, theme::size::LABEL, bold));
    let Ok(text) = CString::new(fitted) else { return };
    let label = Label::new(text.as_ptr(), theme::size::LABEL, ink).v(VAlign::CapTop).h(HAlign::Left);
    if bold { label.bold() } else { label }.draw(p, Rect::new(MARGIN_X, y, w, measure.cap_h(theme::size::LABEL)));
}

/// A household server whose photo proxy fetches TMDB art (the first household source Search
/// reads); `None` draws the neutral ground and blank faces.
fn art_server<H: SearchLike>(cx: &Cx<'_, H>) -> Option<u16> {
    H::search(cx).scope().sources().iter().find(|source| source.household).map(|source| source.sid.raw())
}

/// The information block's height: its heading band, its rows and the air a block keeps under it
/// before the next heading; 0 when it says nothing.
fn info_h(count: usize, measure: &dyn Measure) -> f32 {
    if count == 0 { 0.0 } else { SHELF_HEADING + info::height(count, measure) + theme::space::LG }
}

/// The two shelves.
enum Cards<'a> {
    Cast(CastCards<'a>),
    Related(RequestCards<'a>),
}

impl<H: plx_machine::machine::Host<Elem = u32>> CardSource<H> for Cards<'_> {
    fn len(&self) -> usize {
        match self { Cards::Cast(c) => CardSource::<H>::len(c), Cards::Related(c) => CardSource::<H>::len(c) }
    }
    fn elem(&self, i: usize) -> u32 {
        match self { Cards::Cast(c) => CardSource::<H>::elem(c, i), Cards::Related(c) => CardSource::<H>::elem(c, i) }
    }
    fn index_of(&self, e: &u32) -> Option<usize> {
        match self { Cards::Cast(c) => CardSource::<H>::index_of(c, e), Cards::Related(c) => CardSource::<H>::index_of(c, e) }
    }
    fn art(&self, i: usize) -> Art<'_> {
        match self { Cards::Cast(c) => CardSource::<H>::art(c, i), Cards::Related(c) => CardSource::<H>::art(c, i) }
    }
    fn label(&self, i: usize) -> TileLabel {
        match self { Cards::Cast(c) => CardSource::<H>::label(c, i), Cards::Related(c) => CardSource::<H>::label(c, i) }
    }
    fn more(&self) -> bool {
        match self { Cards::Cast(_) => false, Cards::Related(c) => CardSource::<H>::more(c) }
    }
    fn page_epoch(&self) -> u32 {
        match self { Cards::Cast(_) => 0, Cards::Related(c) => CardSource::<H>::page_epoch(c) }
    }
}

impl<H: SearchLike> StackPage<H> for Page {
    type Key = Sec;
    type Cards<'a> = Cards<'a>;

    /// Every change in the store (which moves the hero's rows) and every change [`Page::sync`] saw in
    /// what the sections hold.
    fn revision(&self, cx: &Cx<'_, H>) -> u64 {
        u64::from(H::seerr(cx).map_or(0, SeerrStore::gen)) << 32 | self.rev
    }

    fn sections(&self, cx: &Cx<'_, H>, out: &mut Vec<SectionSpec<Sec>>) {
        let seerr = H::seerr(cx);
        let hero = self.rows(seerr, cx.measure).map_or(0.0, |rows| rows.height());
        out.push(SectionSpec::new(Sec::Hero, Kind::Custom { height: hero, focusable: !self.elems.is_empty() }, HERO_GROUP));
        out.push(SectionSpec::new(Sec::Info, Kind::Custom { height: info_h(self.info.len(), cx.measure), focusable: false },
            INFO_GROUP));
        out.push(SectionSpec::new(Sec::Cast, Kind::Shelf { style: &CAST_STYLE, heading: SHELF_HEADING }, CAST_GROUP));
        out.push(SectionSpec::new(Sec::Related, Kind::Shelf { style: &RELATED_STYLE, heading: SHELF_HEADING }, RELATED_GROUP));
    }

    fn fallback(&self, _cx: &Cx<'_, H>, out: &mut Vec<Sec>) {
        out.extend([Sec::Hero, Sec::Cast, Sec::Related]);
    }

    fn cards<'a>(&'a self, cx: &'a Cx<'_, H>, k: Sec) -> Option<Cards<'a>> {
        match k {
            Sec::Cast => {
                let cast = self.detail(H::seerr(cx)).map_or(&[][..], |d| d.cast.as_slice());
                (!cast.is_empty()).then(|| Cards::Cast(CastCards { cast, photos: &self.photos, art_server: art_server(cx) }))
            }
            Sec::Related => (!self.related.is_empty()).then(|| Cards::Related(RequestCards {
                elems: &self.related_elems, cards: &self.related, art_server: art_server(cx), more: self.related_more,
                epoch: self.related_read,
            })),
            Sec::Hero | Sec::Info => None,
        }
    }

    /// A Seerr title has no item menu, and a headshot none either: a hold is not taught here.
    fn card_has_menu(&self, _cx: &Cx<'_, H>, _k: Sec, _elem: &u32) -> bool {
        false
    }

    fn plain_len(&self, k: Sec) -> usize {
        if k == Sec::Hero { self.elems.len() } else { 0 }
    }

    fn plain_elem(&self, k: Sec, n: usize) -> Option<u32> {
        if k == Sec::Hero { self.elems.get(n).copied() } else { None }
    }

    /// Left and Right walk the controls; Up from a control reaches the overview above them while it
    /// is a stop, and Down from it comes back to the first control.
    fn plain_step(&self, k: Sec, at: usize, dir: Dir) -> Option<usize> {
        if k != Sec::Hero {
            return None;
        }
        let controls = self.elems.iter().take_while(|e| **e != SYNOPSIS_ELEM).count();
        let overview = (controls < self.elems.len()).then_some(controls);
        match dir {
            Dir::Left if at < controls => at.checked_sub(1),
            Dir::Right if at + 1 < controls => Some(at + 1),
            Dir::Up if at < controls => overview,
            Dir::Down if Some(at) == overview && controls > 0 => Some(0),
            _ => None,
        }
    }

    fn element_rect(&self, cx: &Cx<'_, H>, k: Sec, n: usize, section: Rect) -> Rect {
        let seerr = H::seerr(cx);
        let local = match StackPage::<H>::plain_elem(self, k, n) {
            Some(SYNOPSIS_ELEM) => self.rows(seerr, cx.measure).map(|rows| Self::overview_plate(&rows)),
            Some(elem) => self.control_rect(elem, seerr, cx.measure),
            None => None,
        };
        local.map_or(section, |r| Rect::new(r.x, r.y + section.y, r.w, r.h))
    }

    fn focus_rect(&self, cx: &Cx<'_, H>, k: Sec, section: Rect) -> Rect {
        let rects: Vec<Rect> = (0..StackPage::<H>::plain_len(self, k)).map(|n| StackPage::<H>::element_rect(self, cx, k, n, section)).collect();
        rects.iter().copied().reduce(|a, b| {
            let (x0, y0) = (a.x.min(b.x), a.y.min(b.y));
            Rect::new(x0, y0, (a.x + a.w).max(b.x + b.w) - x0, (a.y + a.h).max(b.y + b.h) - y0)
        }).unwrap_or(section)
    }

    fn plain_group(&self, _cx: &Cx<'_, H>, _k: Sec, id: GroupId, extent: Rect) -> GroupSpec {
        GroupSpec {
            id,
            kind: GroupKind::Free,
            seat: Seat::First,
            reachable: AxisMask::BOTH,
            edge: [EdgeRule::Stop, EdgeRule::Geometric, EdgeRule::Stop, EdgeRule::Stop],
            extent,
            len: self.elems.len(),
            elem: ElemKind::Control,
        }
    }

    fn shelf_foot(&self, _cx: &Cx<'_, H>, _k: Sec) -> f32 {
        UNDER_LABEL_AIR
    }

    fn draw_heading(&self, k: Sec, f: &mut DrawFrame<'_, '_, H>, r: Rect, lift: f32) {
        let title = match k {
            Sec::Cast => msg::browse_detail_cast_c(),
            Sec::Related => msg::browse_detail_related_c(),
            Sec::Hero | Sec::Info => return,
        };
        let p = f.painter.alpha(f.page_alpha);
        plx_ui::cards::draw_heading_c(p, title, c"", c"", MARGIN_X, r.y - lift, 1.0, f.measure);
    }

    fn custom_draw(&self, k: Sec, f: &mut DrawFrame<'_, '_, H>, r: Rect, _scroll: f32) {
        let p = f.painter.alpha(f.page_alpha);
        match k {
            Sec::Hero => {
                let Some(detail) = self.detail(H::seerr(f.cx)) else { return };
                self.hero_draw(f, p.translate(0.0, r.y), detail);
            }
            Sec::Info => {
                if self.info.is_empty() || !plx_ui::on_axis(r.y, r.h, plx_ui::consts::SCR_H, 0.0) {
                    return;
                }
                plx_ui::cards::draw_heading_c(p, msg::browse_detail_information_c(), c"", c"", MARGIN_X, r.y, 1.0, f.measure);
                info::draw(p, &self.info, r.y + SHELF_HEADING, f.measure);
            }
            Sec::Cast | Sec::Related => {}
        }
    }
}

/// The page of one Seerr title.
pub struct SeerrTitleScreen {
    page: Page,
    stack: Stack<Sec>,
}

impl SeerrTitleScreen {
    /// The heartbeat word, byte-identical to the route word (`app/words.rs`).
    pub const WORD: &'static str = "seerr_title";

    pub const SHAPE: &'static str = "SeerrTitleScreen{entry:u32,tv:bool,tmdb:u64,refreshed:bool,deciding:opt<i64>,decided:bool,next:u32,related:[{tv:bool,tmdb:u64,elem:u32}],stack:Stack{scroll:{pos:f32,vel:f32},target:f32,sections:[Shelf]}}";

    pub fn new(entry: EntryId, kind: MediaKind, tmdb: i64) -> Self {
        Self { page: Page::new(entry, kind, tmdb), stack: Stack::new(entry).reveal_on_move(true) }
    }

    fn ask<H: SearchLike>(slot: Slot, op: SeerrOp, fx: &mut Effects<'_, H>) {
        fx.push(Fx::App(AppFx::Store(StoreId::Seerr, StoreCmd::Seerr(SeerrCmd::Ask { slot, op }))));
    }

    fn ask_detail<H: SearchLike>(&self, fx: &mut Effects<'_, H>) {
        let (kind, tmdb) = (self.page.kind, self.page.tmdb);
        Self::ask(Slot::Detail(kind, tmdb), SeerrOp::Detail { kind, tmdb }, fx);
    }

    /// The controls on the hero's row, left to right, with their labels.
    #[cfg(test)]
    fn controls(&self, seerr: Option<&SeerrStore>) -> Vec<(u32, &'static CStr)> {
        self.page.controls(seerr)
    }

    /// Each control's rect in page space, in the row's order.
    #[cfg(test)]
    fn control_rects(&self, seerr: Option<&SeerrStore>, measure: &dyn Measure) -> Vec<(u32, Rect)> {
        self.page.control_rects(seerr, measure)
    }

    /// Bring the page up to the store, and lay it out again when what it holds moved.
    fn sync<H: SearchLike>(&mut self, cx: &Cx<'_, H>) {
        let rev = self.page.rev;
        self.page.sync(H::seerr(cx), |path| plx_plex::plex::seerr::tmdb_image(path, "w185"), cx.measure);
        if self.page.rev != rev {
            self.stack.reveal(&self.page, cx);
        }
    }

    fn activate<H: SearchLike>(&mut self, elem: u32, cx: &Cx<'_, H>, fx: &mut Effects<'_, H>) -> Handled {
        let seerr = H::seerr(cx);
        let (kind, tmdb) = (self.page.kind, self.page.tmdb);
        match elem {
            PILL_ELEM => {
                let Offer::Request(seasons) = self.page.pill(seerr) else { return Handled::Yes };
                if seerr.is_some_and(SeerrStore::may_choose_options) {
                    let scroll = self.stack.scroll();
                    if let Some(rect) = self.page.control_rect(PILL_ELEM, seerr, cx.measure) {
                        let anchor = [rect.x, rect.y - scroll, rect.w, rect.h].map(f32::to_bits);
                        fx.push(Fx::App(AppFx::Content(ContentReq::Panel(ContentPanel::SeerrRequest { kind, tmdb, anchor }))));
                    }
                } else {
                    let op = SeerrOp::Request { kind, tmdb, seasons, options: Default::default() };
                    Self::ask(self.page.request_slot(), op, fx);
                    fx.invalidate(Provenance::Input);
                }
            }
            APPROVE_ELEM | DECLINE_ELEM => {
                let Some(id) = self.page.decidable(seerr).map(|r| r.id) else { return Handled::Yes };
                if self.page.sending(seerr).is_some() {
                    return Handled::Yes;
                }
                let decision = if elem == APPROVE_ELEM { Decision::Approve } else { Decision::Decline };
                (self.page.deciding, self.page.decided) = (Some(id), false);
                Self::ask(Slot::Decide(id), SeerrOp::Decide { id, decision }, fx);
                fx.invalidate(Provenance::Input);
            }
            SYNOPSIS_ELEM => {
                if self.page.elems.contains(&SYNOPSIS_ELEM) {
                    fx.push(Fx::App(AppFx::Content(ContentReq::Panel(ContentPanel::SeerrAbout { kind, tmdb }))));
                    fx.invalidate(Provenance::Input);
                }
            }
            _ => match self.page.related_of_elem(elem) {
                Some(card) => {
                    let (kind, tmdb) = (card.kind, card.tmdb);
                    fx.push(Fx::App(AppFx::Content(ContentReq::Push(ContentArg::SeerrTitle { kind, tmdb }))));
                    fx.invalidate(Provenance::Input);
                }
                // A headshot: there is no Seerr person page to open.
                None if (FIRST_CAST_ELEM..FIRST_RELATED_ELEM).contains(&elem) => {}
                None => return Handled::No,
            },
        }
        Handled::Yes
    }

    /// A request or a decision that went through is read back once: the detail is asked again. A
    /// request on its way (from the pill or the request menu) owes a fresh read-back. A decision
    /// the server says was already made (409) is read back too: somebody else answered first.
    fn follow<H: SearchLike>(&mut self, cx: &Cx<'_, H>, fx: &mut Effects<'_, H>) {
        let seerr = H::seerr(cx);
        let request = self.page.request(seerr);
        if matches!(request, Some(Answer::Loading)) {
            self.page.refreshed = false;
        }
        if matches!(request, Some(Answer::Ready(SeerrData::Requested))) && !self.page.refreshed {
            self.page.refreshed = true;
            self.ask_detail(fx);
        }
        let landed = matches!(self.page.decision(seerr),
            Some(Answer::Ready(SeerrData::Decided) | Answer::Failed(SeerrError::AlreadyRequested)));
        if landed && !self.page.decided {
            self.page.decided = true;
            self.ask_detail(fx);
        }
    }

    fn draw_ground<H: SearchLike>(&self, f: &DrawFrame<'_, '_, H>, p: Painter) {
        plx_gfx::gfx::frame_clear(theme::CLEAR_RGB.0, theme::CLEAR_RGB.1, theme::CLEAR_RGB.2);
        let alpha = plx_ui::hero_alpha(self.stack.scroll(), BACKDROP_FADE);
        let backdrop = self.page.detail(H::seerr(f.cx)).and_then(|d| d.title.backdrop.as_deref())
            .and_then(|path| plx_plex::plex::seerr::tmdb_image(path, "w1280"));
        if let (Some(url), Some(server), true) = (backdrop, art_server(f.cx), alpha > 0.0) {
            let (tex, w, h) = plx_ui::widgets::resolve_tex_wh_on(server, &url, SCR_W as i32,
                plx_ui::consts::SCR_H as i32, 0);
            if tex != 0 {
                p.alpha(alpha).tex_uv(tex, plx_gfx::gfx::UV_FULL, Rect::FULL.cover(w, h), 0.0, theme::TINT_WHITE);
            }
        }
        plx_ui::widgets::hero_scrim(p.alpha(alpha), 1.0, false);
    }

    fn draw_verdict(&self, p: Painter, text: &str) {
        let (sz, bold, ink) = StatusOverlay::verdict_face(StatusKind::Failed);
        let Ok(text) = CString::new(text) else { return };
        let band = Rect::new(0.0, StatusOverlay::FULL_ANCHOR_TOP, SCR_W, sz as f32);
        let label = Label::new(text.as_ptr(), sz, ink).h(HAlign::Center).v(VAlign::CapTop);
        if bold { label.bold() } else { label }.draw(p, band);
    }

    fn view<H: SearchLike>(&self) -> impl Focusable<H> + '_ {
        self.stack.view(&self.page)
    }
}

plx_ui::focusable_via_view!(SeerrTitleScreen, H: [SearchLike], view);

impl LogicalState for SeerrTitleScreen {
    fn write(&self, c: &mut Canon) {
        let page = &self.page;
        c.u32(page.entry.0).bool(page.kind == MediaKind::Tv).u64(page.tmdb as u64).bool(page.refreshed);
        match page.deciding {
            Some(id) => c.bool(true).u64(id as u64),
            None => c.bool(false).u64(0),
        };
        c.bool(page.decided).u32(page.next);
        c.seq(page.related.len());
        for (card, elem) in page.related.iter().zip(&page.related_elems) {
            c.bool(card.kind == MediaKind::Tv).u64(card.tmdb as u64).u32(*elem);
        }
        self.stack.write(c);
    }
    fn probe(&self, out: &mut String) {
        let page = &self.page;
        out.push_str(&format!("seerr_title tv={} refreshed={} deciding={:?} related={}", page.kind == MediaKind::Tv,
            page.refreshed, page.deciding, page.related.len()));
    }
}

impl<H: SearchLike> Machine<H> for SeerrTitleScreen {
    type Ev = ScreenEvent<H>;

    fn step(&mut self, ev: &Self::Ev, cx: &Cx<'_, H>, fx: &mut Effects<'_, H>) -> Handled {
        let card = match self.stack.on(&self.page, ev, cx, fx) {
            Some(StackEvent::Card(Sec::Related, CardEvent::Want(_))) => {
                // The reader walked near the shelf's end: its next page.
                let (kind, tmdb, page) = (self.page.kind, self.page.tmdb, self.page.related_read + 1);
                let slot = Slot::Recommendations(kind, tmdb, page);
                if self.page.related_more && H::seerr(cx).is_some_and(|s| s.asked(&slot).is_none()) {
                    Self::ask(slot, SeerrOp::Recommendations { kind, tmdb, page }, fx);
                }
                None
            }
            Some(StackEvent::Card(_, card)) => Some(card),
            _ => None,
        };
        match ev {
            ScreenEvent::Mount => {
                let (kind, tmdb) = (self.page.kind, self.page.tmdb);
                self.ask_detail(fx);
                Self::ask(Slot::Ratings(kind, tmdb), SeerrOp::Ratings { kind, tmdb }, fx);
                Self::ask(Slot::Recommendations(kind, tmdb, 1), SeerrOp::Recommendations { kind, tmdb, page: 1 }, fx);
                Handled::Yes
            }
            ScreenEvent::Tick(t) => {
                self.sync(cx);
                let shown = self.page.controls(H::seerr(cx));
                let focused = cx.focus.current.filter(|k| k.entry == self.page.entry).map(|k| k.elem);
                self.page.pop.step(focused.filter(|elem| shown.iter().any(|(e, _)| e == elem)).map(|elem| elem as usize),
                    t.dt());
                self.page.summary_lift.step(focused == Some(SYNOPSIS_ELEM), t.dt());
                self.follow(cx, fx);
                Handled::Yes
            }
            ScreenEvent::Enter(_) | ScreenEvent::Uncover | ScreenEvent::StoreChanged(..) => {
                fx.invalidate(Provenance::Nav);
                Handled::Yes
            }
            ScreenEvent::FocusMoved { .. } => {
                fx.invalidate(Provenance::Input);
                Handled::Yes
            }
            ScreenEvent::Activate(elem) => {
                let handled = self.activate(*elem, cx, fx);
                self.sync(cx);
                handled
            }
            ScreenEvent::PressCommit(_) => {
                let handled = match (card, cx.focus.current.filter(|k| k.entry == self.page.entry)) {
                    (Some(CardEvent::Activate(elem)), _) => self.activate(elem, cx, fx),
                    (_, Some(key)) => self.activate(key.elem, cx, fx),
                    _ => Handled::No,
                };
                self.sync(cx);
                handled
            }
            ScreenEvent::Input(input) => match input.kind {
                InputKind::Key { key: Key::Back, edge: Edge::Down, .. } => {
                    fx.push(Fx::App(AppFx::Content(ContentReq::Back)));
                    Handled::Yes
                }
                _ => Handled::No,
            },
            _ => Handled::No,
        }
    }
}

impl<H: SearchLike> Screen<H> for SeerrTitleScreen {
    fn redraw_focused(&self, f: &mut DrawFrame<'_, '_, H>, focus: Option<FocusKey<u32>>) {
        self.stack.view(&self.page).redraw_focused(f, focus);
    }
    fn opener_departing(&self, key: FocusKey<u32>) -> bool {
        self.stack.view(&self.page).opener_departing::<H>(key)
    }
    fn name(&self) -> &'static str {
        Self::WORD
    }
    fn state(&self) -> &dyn LogicalState {
        self
    }
    fn crumb(&self, _: &Cx<'_, H>) -> Option<std::borrow::Cow<'_, str>> {
        None
    }
    fn prepare(&mut self, _: &mut plx_ui::frame::Budget, _: &Cx<'_, H>) {}
    fn draw(&mut self, f: &mut DrawFrame<'_, '_, H>) {
        let p = f.painter.alpha(f.page_alpha);
        self.draw_ground(f, p);
        let seerr = H::seerr(f.cx);
        match seerr.and_then(|s| s.answer(&self.page.detail_slot())) {
            Some(Answer::Ready(SeerrData::Detail(_))) => self.stack.view(&self.page).paint(f),
            Some(Answer::Failed(_)) => self.draw_verdict(p, msg::seerr_title_unreachable()),
            _ => {}
        }
    }
    fn render(&self) -> RenderStrategy {
        RenderStrategy::Page
    }
    fn focus_source(&self) -> FocusSource {
        FocusSource::Engine
    }
    fn hit_source(&self) -> HitSource {
        HitSource::Engine
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
#[path = "seerr_title_tests.rs"]
mod tests;
