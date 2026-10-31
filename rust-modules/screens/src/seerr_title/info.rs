//! **A Seerr title's Information block** (docs/seerr.md): what TMDB says of the title past the
//! hero's facts, as label and value pairs in three columns under the shelf heading "Information".
//! Pure: the pairs are built from the detail and the television's country, so every wording is
//! tested without a page, and the layout is arithmetic over measured cap heights.
//!
//! A pair with nothing to say is left out, and so is the block when no pair is left. The values are
//! the server's words (names, a collection's title) or numbers the app formats, and a value too wide
//! for its column ends in an ellipsis; the labels are the app's and must fit.
use std::ffi::CString;

use plx_data::seerr::{MediaKind, TitleDetail};
use plx_machine::machine::Measure;
use plx_platform::i18n::msg;
use plx_ui::hstack::{Align, HStack, Item};
use plx_ui::label::{HAlign, Label, VAlign};
use plx_ui::vstack::VFlow;
use plx_ui::{consts, theme, Painter, Rect};

/// The columns the pairs flow through, row by row.
pub const COLS: usize = 3;
/// Air between two columns, and under a pair's value before the next row's label.
const COL_GAP: f32 = theme::space::LG;
const ROW_GAP: f32 = theme::space::MD;
/// A label's cap band to its value's.
const LABEL_GAP: f32 = theme::space::XS;
const LABEL_SIZE: i32 = theme::size::CAPTION;
const VALUE_SIZE: i32 = theme::size::LABEL;

/// The television's country, ISO 3166-1, from the UI language's formatting locale ("nl-NL" is
/// NL); the United States when it names none.
pub fn region() -> String {
    let locale = plx_platform::i18n::current().format_locale();
    locale.rsplit(['-', '_']).next().filter(|r| r.len() == 2 && *r != locale).unwrap_or("US").to_ascii_uppercase()
}

/// An amount of US dollars as the language says it: "$165,000,000".
pub fn dollars(amount: i64) -> String {
    msg::seerr_info_dollars(&plx_platform::i18n::current().number(amount))
}

/// The pairs the block shows for `detail` in `region`, in reading order.
pub fn pairs(kind: MediaKind, detail: &TitleDetail, region: &str) -> Vec<(&'static str, String)> {
    let f = &detail.facts;
    let joined = |names: &[String]| names.join(", ");
    let mut out: Vec<(&'static str, String)> = Vec::new();
    let mut push = |label: &'static str, value: String| {
        if !value.is_empty() {
            out.push((label, value));
        }
    };
    push(msg::browse_detail_released(), f.released.as_deref().map(|d| plx_ui::fmt::pretty_date(d, 0)).unwrap_or_default());
    push(msg::browse_detail_rated(), detail.certification(region).unwrap_or_default().to_string());
    push(msg::seerr_info_original_language(),
        f.original_language.as_deref().map(|code| plx_plex::plex::languages::label(code).to_string()).unwrap_or_default());
    match kind {
        MediaKind::Movie => push(msg::seerr_info_directed_by(), joined(&detail.crew.directors)),
        MediaKind::Tv => push(msg::seerr_info_created_by(), joined(&detail.crew.creators)),
    }
    push(msg::seerr_info_written_by(), joined(&detail.crew.writers));
    push(msg::seerr_info_budget(), f.budget.map(dollars).unwrap_or_default());
    push(msg::seerr_info_revenue(), f.revenue.map(dollars).unwrap_or_default());
    push(msg::browse_collection_kind(), f.collection.clone().unwrap_or_default());
    push(msg::seerr_info_streaming(), joined(detail.providers_in(region)));
    out
}

/// One pair's height: its label's cap band, the gap and its value's.
fn pair_h(measure: &dyn Measure) -> f32 {
    measure.cap_h(LABEL_SIZE) + LABEL_GAP + measure.cap_h(VALUE_SIZE)
}

/// The block's height under its heading band for `count` pairs, the air under its last row
/// included; 0 for none.
pub fn height(count: usize, measure: &dyn Measure) -> f32 {
    let rows = count.div_ceil(COLS);
    if rows == 0 {
        return 0.0;
    }
    rows as f32 * (pair_h(measure) + ROW_GAP)
}

/// A column's width: the safe width shared by [`COLS`] columns a gap apart.
fn col_w() -> f32 {
    (consts::SCR_W - 2.0 * consts::MARGIN_X - (COLS - 1) as f32 * COL_GAP) / COLS as f32
}

/// Draw `pairs` from `top`, row by row through the columns.
pub fn draw(p: Painter, pairs: &[(&'static str, String)], top: f32, measure: &dyn Measure) {
    let w = col_w();
    let items = [Item::new(w); COLS];
    let columns = HStack::new(&items, COL_GAP, Align::Leading(consts::MARGIN_X));
    let mut rows = VFlow::new(top, ROW_GAP);
    for row in pairs.chunks(COLS) {
        let Some(span) = rows.push(Item::new(pair_h(measure))) else { continue };
        for ((label, value), (x, _)) in row.iter().zip((0..COLS).filter_map(|i| columns.span(i))) {
            let line = |text: &str, y: f32, size: i32, ink| {
                let fitted = plx_machine::machine::elide_by(text, w, false, |s| measure.width_str(s, size, false));
                let Ok(text) = CString::new(fitted) else { return };
                Label::new(text.as_ptr(), size, ink).h(HAlign::Left).v(VAlign::CapTop)
                    .draw(p, Rect::new(x, y, w, measure.cap_h(size)));
            };
            line(label, span.y, LABEL_SIZE, theme::TEXT_SECONDARY);
            line(value, span.y + measure.cap_h(LABEL_SIZE) + LABEL_GAP, VALUE_SIZE, theme::TEXT_PRIMARY);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plx_data::seerr::{Crew, TitleFacts};

    fn detail() -> TitleDetail {
        TitleDetail {
            facts: TitleFacts { released: Some("2024-03-01".into()), original_language: Some("ja".into()),
                budget: Some(165_000_000), collection: Some("s0000c011".into()), ..Default::default() },
            crew: Crew { directors: vec!["s0000d1".into(), "s0000d2".into()], writers: Vec::new(),
                creators: vec!["s0000c1".into()] },
            certifications: vec![("US".into(), "PG-13".into()), ("NL".into(), "12".into())],
            providers: vec![("NL".into(), vec!["s0000p1".into(), "s0000p2".into()])],
            ..Default::default()
        }
    }

    #[test]
    fn a_titles_pairs_say_what_tmdb_said_in_this_country_and_leave_out_the_rest() {
        let got = pairs(MediaKind::Movie, &detail(), "NL");
        assert_eq!(got, [
            (msg::browse_detail_released(), plx_ui::fmt::pretty_date("2024-03-01", 0)),
            (msg::browse_detail_rated(), "12".to_string()),
            (msg::seerr_info_original_language(), "日本語".to_string()),
            (msg::seerr_info_directed_by(), "s0000d1, s0000d2".to_string()),
            (msg::seerr_info_budget(), dollars(165_000_000)),
            (msg::browse_collection_kind(), "s0000c011".to_string()),
            (msg::seerr_info_streaming(), "s0000p1, s0000p2".to_string()),
        ], "no writers and no box office: no pairs for them");
        let elsewhere = pairs(MediaKind::Tv, &detail(), "FR");
        assert!(elsewhere.contains(&(msg::browse_detail_rated(), "PG-13".to_string())), "the United States' rating stands in");
        assert!(elsewhere.contains(&(msg::seerr_info_created_by(), "s0000c1".to_string())), "a show names its creators");
        assert!(!elsewhere.iter().any(|(l, _)| *l == msg::seerr_info_streaming() || *l == msg::seerr_info_directed_by()),
            "no service in this country, and a show names no director");
        assert!(pairs(MediaKind::Movie, &TitleDetail::default(), "NL").is_empty());
    }

    #[test]
    fn dollars_group_as_the_language_does_and_the_country_comes_from_the_locale() {
        assert_eq!(dollars(165_000_000), "$165,000,000");
        let r = region();
        assert!(r.len() == 2 && r.bytes().all(|b| b.is_ascii_uppercase()), "{r}");
    }

    #[test]
    fn the_block_takes_a_row_per_three_pairs() {
        let m = plx_ui::fixture::FixtureMeasure;
        assert_eq!(height(0, &m), 0.0);
        assert_eq!(height(3, &m), height(1, &m));
        assert!((height(4, &m) - 2.0 * height(1, &m)).abs() < 0.01);
    }
}

#[cfg(test)]
mod fit_tests {
    use super::*;

    /// The labels are the app's words: each fits its column in every shipped language, measured with
    /// the television's own glyph widths (a value is the server's, and may elide).
    #[test]
    fn every_label_fits_its_column_in_every_language() {
        let measure = plx_base::fontcov::advances::ShippedMeasure;
        let mut misfits = Vec::new();
        for language in plx_platform::i18n::SHIPPED {
            let _guard = plx_platform::i18n::language_on_this_thread_for_test(language);
            for label in [msg::browse_detail_released(), msg::browse_detail_rated(), msg::seerr_info_original_language(),
                msg::seerr_info_directed_by(), msg::seerr_info_created_by(), msg::seerr_info_written_by(),
                msg::seerr_info_budget(), msg::seerr_info_revenue(), msg::browse_collection_kind(), msg::seerr_info_streaming()]
            {
                let w = measure.width_str(label, LABEL_SIZE, false);
                if w > col_w() {
                    misfits.push(format!("{} {label:?} {w}px > {}px", language.tag(), col_w()));
                }
            }
        }
        assert!(misfits.is_empty(), "{misfits:?}");
    }
}
