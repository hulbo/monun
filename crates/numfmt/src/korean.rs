//! Korean in format codes, as Excel on Korean Windows shows it: weekday and month names (`aaa`,
//! `aaaa`, and `ddd`, `mmm`, `AM/PM` under a Korean locale tag), the system long date and time
//! (`[$-F800]`, `[$-F400]`) and the `[DBNum1]`, `[DBNum2]` and `[DBNum4]` numerals.

/// Korean Windows' long date (`yyyy'년' M'월' d'일' dddd`), what `[$-F800]` shows.
pub(crate) const LONG_DATE: &str = "yyyy\"년\" m\"월\" d\"일\" dddd";
/// Korean Windows' long time (`tt h:mm:ss`), what `[$-F400]` shows.
pub(crate) const LONG_TIME: &str = "AM/PM h:mm:ss";

const WEEKDAYS: [&str; 7] = ["일", "월", "화", "수", "목", "금", "토"];

/// The weekday (0 = Sunday): `월` short, `월요일` long.
pub(crate) fn weekday(day: usize, long: bool) -> String {
    let name = WEEKDAYS.get(day).copied().unwrap_or("");
    if long { format!("{name}요일") } else { name.to_string() }
}

/// The month name, short and long alike: `11월`.
pub(crate) fn month(month: u32) -> String {
    format!("{month}월")
}

/// What a `[$-…]` tag's locale asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Locale {
    /// Korean (LCID 0x0412, `ko`, `ko-KR`): Korean names.
    Korean,
    /// The system long date (`F800`, `x-sysdate`) in place of the section's own codes.
    LongDate,
    /// The system time (`F400`, `x-systime`) in place of the section's own codes.
    LongTime,
    /// Another locale, or none: names stay English.
    Other,
}

/// Reads the locale after the `-` of a `[$sym-locale]` tag: a hex LCID whose low 16 bits name
/// the language (the bits above choose a calendar or digits), or a language tag.
pub(crate) fn locale(tag: &str) -> Locale {
    let tag = tag.trim().to_ascii_lowercase();
    match tag.as_str() {
        "ko" | "ko-kr" => return Locale::Korean,
        "x-sysdate" => return Locale::LongDate,
        "x-systime" => return Locale::LongTime,
        _ => {}
    }
    match u32::from_str_radix(&tag, 16).map(|id| id & 0xFFFF) {
        Ok(0x0412) => Locale::Korean,
        Ok(0xF800) => Locale::LongDate,
        Ok(0xF400) => Locale::LongTime,
        _ => Locale::Other,
    }
}

/// Digits and units of a `[DBNumN]` style. `[DBNum3]` (full-width digits) is not drawn: its
/// numbers show as they would without it.
struct Numerals {
    digits: [char; 10],
    /// Tens, hundreds and thousands within a group of four digits.
    units: [char; 3],
    /// 10^4, 10^8, 10^12, 10^16.
    groups: [char; 4],
}

const HANJA: Numerals =
    Numerals {
        digits: ['零', '一', '二', '三', '四', '五', '六', '七', '八', '九'], units: ['十', '百', '千'], groups: ['萬', '億', '兆', '京']
    };
const HANJA_FORMAL: Numerals =
    Numerals {
        digits: ['零', '壹', '貳', '參', '四', '五', '六', '七', '八', '九'], units: ['拾', '百', '阡'], groups: ['萬', '億', '兆', '京']
    };
const HANGUL: Numerals =
    Numerals {
        digits: ['영', '일', '이', '삼', '사', '오', '육', '칠', '팔', '구'], units: ['십', '백', '천'], groups: ['만', '억', '조', '경']
    };

fn numerals(dbnum: u8) -> Option<&'static Numerals> {
    match dbnum {
        1 => Some(&HANJA),
        2 => Some(&HANJA_FORMAL),
        4 => Some(&HANGUL),
        _ => None,
    }
}

/// Whether `[DBNumN]` changes how numbers look.
pub(crate) fn draws(dbnum: u8) -> bool {
    numerals(dbnum).is_some()
}

/// `text` with each ASCII digit written as a `[DBNumN]` numeral (`[DBNum4]000` shows 123 as
/// `일이삼`); everything else stays.
pub(crate) fn digits(text: &str, dbnum: u8) -> String {
    let Some(n) = numerals(dbnum) else { return text.to_string() };
    text.chars().map(|c| c.to_digit(10).and_then(|d| n.digits.get(d as usize).copied()).unwrap_or(c)).collect()
}

/// A non-negative number as `[DBNumN]General` shows it: the whole part written out with units
/// (see [`with_units`]), a fraction digit by digit (`123.45` is `일백이십삼.사오`).
pub(crate) fn general(v: f64, dbnum: u8) -> String {
    let text = crate::decimal::format_general_fit(v, 21).unwrap_or_default();
    if text.contains(['E', 'e']) {
        return digits(&text, dbnum);
    }
    let (int, frac) = text.split_once('.').unwrap_or((&text, ""));
    let mut out = with_units(int, dbnum);
    if !frac.is_empty() {
        out.push('.');
        out.push_str(&digits(frac, dbnum));
    }
    out
}

/// A whole number given as ASCII digits, written out with units in groups of four, as a cheque
/// is: 12345 is `일만이천삼백사십오` (`[DBNum4]`), `一萬二千三百四十五` (`[DBNum1]`). Each unit
/// keeps its digit (10 is `일십`), zeros are left out, and zero itself is the zero numeral.
/// Digits past the last group unit (10^20 and up) are written one by one.
pub(crate) fn with_units(int: &str, dbnum: u8) -> String {
    let Some(n) = numerals(dbnum) else { return int.to_string() };
    let ds: Vec<u32> = int.trim_start_matches('0').chars().filter_map(|c| c.to_digit(10)).collect();
    if ds.is_empty() {
        return n.digits[0].to_string();
    }
    if ds.len() > 4 * (n.groups.len() + 1) {
        return digits(int, dbnum);
    }
    let digit = |d: u32| n.digits.get(d as usize).copied().unwrap_or('?');
    let mut out = String::new();
    let len = ds.len();
    let mut group_has_digit = false;
    for (i, &d) in ds.iter().enumerate() {
        let place = len - 1 - i;
        if d != 0 {
            out.push(digit(d));
            if let Some(&unit) = (place % 4).checked_sub(1).and_then(|u| n.units.get(u)) {
                out.push(unit);
            }
            group_has_digit = true;
        }
        if place.is_multiple_of(4) && place > 0 {
            if group_has_digit && let Some(&group) = n.groups.get(place / 4 - 1) {
                out.push(group);
            }
            group_has_digit = false;
        }
    }
    out
}
