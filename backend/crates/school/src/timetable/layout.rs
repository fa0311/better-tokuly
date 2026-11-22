//! Period layouts: the ready-made ones and the checks on custom ones.
use tokuly_core::prelude::*;
use tokuly_proto::{
    common::LocalizedText,
    school::{LayoutPreset, LayoutPresetInfo, Period, PeriodLayout, Weekday},
};
use unicode_segmentation::UnicodeSegmentation as _;

pub(crate) const MAX_PERIODS: usize = 12;
const MAX_LABEL_CHARS: usize = 12;

/// One period as the resolver uses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PeriodSpec {
    pub number: i32,
    pub start_minute: i32,
    pub end_minute: i32,
    pub label: String,
}

/// The shape of a term's week.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Layout {
    pub preset: LayoutPreset,
    pub periods: Vec<PeriodSpec>,
    /// 1 = Monday ... 7 = Sunday, ascending.
    pub weekdays: Vec<i32>,
}

impl Layout {
    pub fn period(&self, number: i32) -> Option<&PeriodSpec> {
        self.periods.iter().find(|period| period.number == number)
    }

    pub fn has_weekday(&self, weekday: i32) -> bool {
        self.weekdays.contains(&weekday)
    }

    pub fn to_proto(&self) -> PeriodLayout {
        PeriodLayout {
            preset: self.preset.into(),
            periods: self.periods_proto(),
            weekdays: self.weekdays.iter().map(|weekday| (*weekday).into()).collect(),
            ..Default::default()
        }
    }

    pub fn periods_proto(&self) -> Vec<Period> {
        self.periods
            .iter()
            .map(|period| Period {
                number: period.number,
                start_minute: period.start_minute,
                end_minute: period.end_minute,
                label: period.label.clone(),
                ..Default::default()
            })
            .collect()
    }

    /// Reads a layout stored as proto JSON. Stored layouts were validated
    /// when written; a row that cannot be read falls back to the default.
    pub fn from_json(value: &serde_json::Value) -> Self {
        serde_json::from_value::<PeriodLayout>(value.clone())
            .ok()
            .and_then(|layout| validate(Some(&layout)).ok())
            .unwrap_or_else(|| preset(LayoutPreset::LAYOUT_PRESET_HIGH_SCHOOL_6).expect("the default preset exists"))
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self.to_proto()).expect("layouts serialize")
    }
}

const MONDAY_TO_FRIDAY: [i32; 5] = [1, 2, 3, 4, 5];

fn hm(hour: i32, minute: i32) -> i32 {
    hour * 60 + minute
}

fn periods(times: &[(i32, i32)]) -> Vec<PeriodSpec> {
    times
        .iter()
        .enumerate()
        .map(|(i, (start, end))| PeriodSpec {
            number: i as i32 + 1,
            start_minute: *start,
            end_minute: *end,
            label: String::new(),
        })
        .collect()
}

/// The periods of a ready-made layout; `None` for CUSTOM / UNSPECIFIED.
pub(crate) fn preset(preset: LayoutPreset) -> Option<Layout> {
    use LayoutPreset::*;
    let times: Vec<(i32, i32)> = match preset {
        // 50-minute periods, 10-minute breaks, lunch after the fourth.
        LAYOUT_PRESET_HIGH_SCHOOL_6 | LAYOUT_PRESET_HIGH_SCHOOL_7 => {
            let mut times = vec![
                (hm(8, 50), hm(9, 40)),
                (hm(9, 50), hm(10, 40)),
                (hm(10, 50), hm(11, 40)),
                (hm(11, 50), hm(12, 40)),
                (hm(13, 30), hm(14, 20)),
                (hm(14, 30), hm(15, 20)),
            ];
            if preset == LAYOUT_PRESET_HIGH_SCHOOL_7 {
                times.push((hm(15, 30), hm(16, 20)));
            }
            times
        }
        LAYOUT_PRESET_HIGH_SCHOOL_7_SHORT => vec![
            (hm(8, 40), hm(9, 25)),
            (hm(9, 35), hm(10, 20)),
            (hm(10, 30), hm(11, 15)),
            (hm(11, 25), hm(12, 10)),
            (hm(13, 0), hm(13, 45)),
            (hm(13, 55), hm(14, 40)),
            (hm(14, 50), hm(15, 35)),
        ],
        LAYOUT_PRESET_UNIVERSITY_90 => vec![
            (hm(9, 0), hm(10, 30)),
            (hm(10, 40), hm(12, 10)),
            (hm(13, 0), hm(14, 30)),
            (hm(14, 40), hm(16, 10)),
            (hm(16, 20), hm(17, 50)),
        ],
        LAYOUT_PRESET_UNIVERSITY_100 => vec![
            (hm(9, 0), hm(10, 40)),
            (hm(10, 50), hm(12, 30)),
            (hm(13, 20), hm(15, 0)),
            (hm(15, 10), hm(16, 50)),
            (hm(17, 0), hm(18, 40)),
        ],
        LAYOUT_PRESET_UNIVERSITY_105 => vec![
            (hm(8, 30), hm(10, 15)),
            (hm(10, 25), hm(12, 10)),
            (hm(13, 0), hm(14, 45)),
            (hm(14, 55), hm(16, 40)),
            (hm(16, 50), hm(18, 35)),
        ],
        LAYOUT_PRESET_CUSTOM | LAYOUT_PRESET_UNSPECIFIED => return None,
    };
    Some(Layout { preset, periods: periods(&times), weekdays: MONDAY_TO_FRIDAY.to_vec() })
}

/// Every ready-made layout with its name, for `ListLayoutPresets`.
pub(crate) fn preset_infos() -> Vec<LayoutPresetInfo> {
    use LayoutPreset::*;
    let text = |ja: &str, en: &str| LocalizedText { ja: ja.into(), en: en.into(), ..Default::default() };
    [
        (
            LAYOUT_PRESET_HIGH_SCHOOL_6,
            text("高校・6時間授業", "High school, 6 periods"),
            text("50分授業、8:50開始", "50-minute periods from 8:50"),
        ),
        (
            LAYOUT_PRESET_HIGH_SCHOOL_7,
            text("高校・7時間授業", "High school, 7 periods"),
            text("50分授業、8:50開始", "50-minute periods from 8:50"),
        ),
        (
            LAYOUT_PRESET_HIGH_SCHOOL_7_SHORT,
            text("高校・45分×7時間", "High school, 7 short periods"),
            text("45分授業、8:40開始", "45-minute periods from 8:40"),
        ),
        (
            LAYOUT_PRESET_UNIVERSITY_90,
            text("大学・90分授業", "University, 90 minutes"),
            text("5コマ、9:00開始", "5 periods from 9:00"),
        ),
        (
            LAYOUT_PRESET_UNIVERSITY_100,
            text("大学・100分授業", "University, 100 minutes"),
            text("5コマ、9:00開始", "5 periods from 9:00"),
        ),
        (
            LAYOUT_PRESET_UNIVERSITY_105,
            text("大学・105分授業", "University, 105 minutes"),
            text("5コマ、8:30開始", "5 periods from 8:30"),
        ),
    ]
    .into_iter()
    .map(|(id, name, description)| LayoutPresetInfo {
        preset: id.into(),
        name: name.into(),
        description: description.into(),
        layout: preset(id).expect("listed presets exist").to_proto().into(),
        ..Default::default()
    })
    .collect()
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::invalid("LAYOUT_INVALID", message.into())
}

/// Checks a layout from a request and fills in what it leaves out: no layout
/// at all is the six-period high-school day, a preset without periods gets
/// the preset's, no weekdays means Monday to Friday. Periods are renumbered
/// 1..n in the order given, which must be the order of time.
pub(crate) fn validate(input: Option<&PeriodLayout>) -> AppResult<Layout> {
    let Some(input) = input else {
        return Ok(preset(LayoutPreset::LAYOUT_PRESET_HIGH_SCHOOL_6).expect("the default preset exists"));
    };
    let requested = input.preset.as_known().ok_or_else(|| invalid("unknown layout preset"))?;

    let mut weekdays = Vec::new();
    for weekday in &input.weekdays {
        let weekday = weekday
            .as_known()
            .filter(|w| *w != Weekday::WEEKDAY_UNSPECIFIED)
            .ok_or_else(|| invalid("unknown weekday"))? as i32;
        if !weekdays.contains(&weekday) {
            weekdays.push(weekday);
        }
    }
    weekdays.sort_unstable();
    if weekdays.is_empty() {
        weekdays = MONDAY_TO_FRIDAY.to_vec();
    }

    if input.periods.is_empty() {
        let mut layout = preset(requested)
            .or_else(|| {
                (requested == LayoutPreset::LAYOUT_PRESET_UNSPECIFIED)
                    .then(|| preset(LayoutPreset::LAYOUT_PRESET_HIGH_SCHOOL_6))
                    .flatten()
            })
            .ok_or_else(|| invalid("a custom layout needs its periods"))?;
        layout.weekdays = weekdays;
        return Ok(layout);
    }

    ensure!(input.periods.len() <= MAX_PERIODS, invalid, "LAYOUT_INVALID", "a day has at most {MAX_PERIODS} periods");
    let mut periods = Vec::with_capacity(input.periods.len());
    let mut previous_end = 0;
    for (index, period) in input.periods.iter().enumerate() {
        ensure!(
            (0..1440).contains(&period.start_minute)
                && period.end_minute <= 1440
                && period.end_minute > period.start_minute,
            invalid,
            "LAYOUT_INVALID",
            "period {} must start and end within the day, the end after the start",
            index + 1
        );
        ensure!(
            period.start_minute >= previous_end,
            invalid,
            "LAYOUT_INVALID",
            "periods must be in order of time and must not overlap"
        );
        previous_end = period.end_minute;
        let label = period.label.trim();
        ensure!(
            label.graphemes(true).count() <= MAX_LABEL_CHARS && !label.chars().any(char::is_control),
            invalid,
            "LAYOUT_INVALID",
            "period labels have at most {MAX_LABEL_CHARS} characters"
        );
        periods.push(PeriodSpec {
            number: index as i32 + 1,
            start_minute: period.start_minute,
            end_minute: period.end_minute,
            label: label.to_owned(),
        });
    }
    // Hand-entered periods that happen to be a preset's keep the preset's name.
    let preset = match preset(requested) {
        Some(known) if known.periods == periods => requested,
        _ => LayoutPreset::LAYOUT_PRESET_CUSTOM,
    };
    Ok(Layout { preset, periods, weekdays })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn period(start: i32, end: i32) -> Period {
        Period { start_minute: start, end_minute: end, ..Default::default() }
    }

    #[test]
    fn presets_have_the_usual_school_days() {
        let high_school = preset(LayoutPreset::LAYOUT_PRESET_HIGH_SCHOOL_6).unwrap();
        assert_eq!(high_school.periods.len(), 6);
        assert!(high_school.periods.iter().all(|p| p.end_minute - p.start_minute == 50));
        assert_eq!(high_school.period(1).unwrap().start_minute, 8 * 60 + 50);
        // Lunch between the fourth and the fifth period.
        assert_eq!(high_school.period(5).unwrap().start_minute - high_school.period(4).unwrap().end_minute, 50);
        assert_eq!(preset(LayoutPreset::LAYOUT_PRESET_HIGH_SCHOOL_7).unwrap().periods.len(), 7);
        assert!(
            preset(LayoutPreset::LAYOUT_PRESET_HIGH_SCHOOL_7_SHORT)
                .unwrap()
                .periods
                .iter()
                .all(|p| p.end_minute - p.start_minute == 45)
        );
        for (id, minutes) in [
            (LayoutPreset::LAYOUT_PRESET_UNIVERSITY_90, 90),
            (LayoutPreset::LAYOUT_PRESET_UNIVERSITY_100, 100),
            (LayoutPreset::LAYOUT_PRESET_UNIVERSITY_105, 105),
        ] {
            let layout = preset(id).unwrap();
            assert_eq!(layout.periods.len(), 5);
            assert!(layout.periods.iter().all(|p| p.end_minute - p.start_minute == minutes), "{id:?}");
            assert_eq!(layout.weekdays, vec![1, 2, 3, 4, 5]);
        }
        assert!(preset(LayoutPreset::LAYOUT_PRESET_CUSTOM).is_none());
        // Every listed preset is a valid layout by the rules for custom ones.
        for info in preset_infos() {
            let layout = info.layout.as_option().unwrap();
            let mut custom = layout.clone();
            custom.preset = LayoutPreset::LAYOUT_PRESET_CUSTOM.into();
            assert!(validate(Some(&custom)).is_ok());
            assert!(!info.name.as_option().unwrap().ja.is_empty());
        }
        assert_eq!(preset_infos().len(), 6);
    }

    #[test]
    fn missing_parts_are_filled_in() {
        let default = validate(None).unwrap();
        assert_eq!(default.preset, LayoutPreset::LAYOUT_PRESET_HIGH_SCHOOL_6);
        let university = validate(Some(&PeriodLayout {
            preset: LayoutPreset::LAYOUT_PRESET_UNIVERSITY_90.into(),
            weekdays: vec![
                Weekday::WEEKDAY_SATURDAY.into(),
                Weekday::WEEKDAY_MONDAY.into(),
                Weekday::WEEKDAY_MONDAY.into(),
            ],
            ..Default::default()
        }))
        .unwrap();
        assert_eq!(university.periods.len(), 5);
        assert_eq!(university.weekdays, vec![1, 6], "sorted and without duplicates");
        assert_eq!(validate(Some(&PeriodLayout::default())).unwrap(), default);
    }

    #[test]
    fn custom_periods_are_renumbered_and_checked() {
        let custom = validate(Some(&PeriodLayout {
            periods: vec![
                Period { number: 7, label: " 朝学習 ".into(), ..period(480, 500) },
                period(510, 560),
                period(560, 610),
            ],
            ..Default::default()
        }))
        .unwrap();
        assert_eq!(custom.preset, LayoutPreset::LAYOUT_PRESET_CUSTOM);
        assert_eq!(custom.periods.iter().map(|p| p.number).collect::<Vec<_>>(), vec![1, 2, 3]);
        assert_eq!(custom.periods[0].label, "朝学習");
        assert_eq!(Layout::from_json(&custom.to_json()), custom, "layouts survive storage");

        let rejects =
            |periods: Vec<Period>| validate(Some(&PeriodLayout { periods, ..Default::default() })).unwrap_err().reason;
        assert_eq!(rejects(vec![period(600, 600)]), "LAYOUT_INVALID");
        assert_eq!(rejects(vec![period(600, 500)]), "LAYOUT_INVALID");
        assert_eq!(rejects(vec![period(-5, 50)]), "LAYOUT_INVALID");
        assert_eq!(rejects(vec![period(1400, 1500)]), "LAYOUT_INVALID");
        assert_eq!(rejects(vec![period(540, 600), period(590, 650)]), "LAYOUT_INVALID", "overlap");
        assert_eq!(rejects(vec![period(600, 650), period(500, 550)]), "LAYOUT_INVALID", "out of order");
        assert_eq!(rejects((0..13).map(|i| period(i * 60, i * 60 + 50)).collect()), "LAYOUT_INVALID");
        assert_eq!(rejects(vec![Period { label: "x".repeat(13), ..period(500, 550) }]), "LAYOUT_INVALID");
        let custom_without_periods =
            PeriodLayout { preset: LayoutPreset::LAYOUT_PRESET_CUSTOM.into(), ..Default::default() };
        assert_eq!(validate(Some(&custom_without_periods)).unwrap_err().reason, "LAYOUT_INVALID");
    }

    #[test]
    fn hand_entered_preset_periods_keep_the_preset() {
        let preset_layout = preset(LayoutPreset::LAYOUT_PRESET_UNIVERSITY_90).unwrap().to_proto();
        assert_eq!(validate(Some(&preset_layout)).unwrap().preset, LayoutPreset::LAYOUT_PRESET_UNIVERSITY_90);
        let mut shifted = preset_layout;
        shifted.periods[0].start_minute += 5;
        assert_eq!(validate(Some(&shifted)).unwrap().preset, LayoutPreset::LAYOUT_PRESET_CUSTOM);
    }
}
