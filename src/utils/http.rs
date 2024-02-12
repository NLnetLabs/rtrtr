use chrono::{DateTime, Utc};
use chrono::format::{Item, Fixed, Numeric, Pad};


//------------ Parsing Etags -------------------------------------------------

/// An iterator over the etags in an If-Not-Match header value.
///
/// This does not handle the "*" value.
///
/// One caveat: The iterator stops when it encounters bad formatting which
/// makes this indistinguishable from reaching the end of a correctly
/// formatted value. As a consequence, we will 304 a request that has the
/// right tag followed by garbage.
pub struct EtagsIter<'a>(&'a str);

impl<'a> EtagsIter<'a> {
    pub fn new(value: &'a str) -> Self {
        Self(value)
    }
}

impl<'a> Iterator for EtagsIter<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        // Skip white space and check if we are done.
        self.0 = self.0.trim_start();
        if self.0.is_empty() {
            return None
        }

        // We either have to have a lone DQUOTE or one prefixed by W/
        let prefix_len = if self.0.starts_with('"') {
            1
        }
        else if self.0.starts_with("W/\"") {
            3
        }
        else {
            return None
        };

        // Find the end of the tag which is after the next DQUOTE.
        let end = match self.0[prefix_len..].find('"') {
            Some(index) => index + prefix_len + 1,
            None => return None
        };

        let res = &self.0[0..end];

        // Move past the second DQUOTE and any space.
        self.0 = self.0[end..].trim_start();

        // If we have a comma, skip over that and any space.
        if self.0.starts_with(',') {
            self.0 = self.0[1..].trim_start();
        }

        Some(res)
    }
}


//------------ Parsing and Constructing HTTP Dates ---------------------------

/// Definition of the preferred date format (aka IMF-fixdate).
///
/// The definition allows for relaxed parsing: It accepts additional white
/// space and ignores case for textual representations. It does, however,
/// construct the correct representation when formatting.
const IMF_FIXDATE: &[Item<'static>] = &[
    Item::Space(""),
    Item::Fixed(Fixed::ShortWeekdayName),
    Item::Space(""),
    Item::Literal(","),
    Item::Space(" "),
    Item::Numeric(Numeric::Day, Pad::Zero),
    Item::Space(" "),
    Item::Fixed(Fixed::ShortMonthName),
    Item::Space(" "),
    Item::Numeric(Numeric::Year, Pad::Zero),
    Item::Space(" "),
    Item::Numeric(Numeric::Hour, Pad::Zero),
    Item::Literal(":"),
    Item::Numeric(Numeric::Minute, Pad::Zero),
    Item::Literal(":"),
    Item::Numeric(Numeric::Second, Pad::Zero),
    Item::Space(" "),
    Item::Literal("GMT"),
    Item::Space(""),
];

/// Definition of the obsolete RFC850 date format..
const RFC850_DATE: &[Item<'static>] = &[
    Item::Space(""),
    Item::Fixed(Fixed::LongWeekdayName),
    Item::Space(""),
    Item::Literal(","),
    Item::Space(" "),
    Item::Numeric(Numeric::Day, Pad::Zero),
    Item::Literal("-"),
    Item::Fixed(Fixed::ShortMonthName),
    Item::Literal("-"),
    Item::Numeric(Numeric::YearMod100, Pad::Zero),
    Item::Space(" "),
    Item::Numeric(Numeric::Hour, Pad::Zero),
    Item::Literal(":"),
    Item::Numeric(Numeric::Minute, Pad::Zero),
    Item::Literal(":"),
    Item::Numeric(Numeric::Second, Pad::Zero),
    Item::Space(" "),
    Item::Literal("GMT"),
    Item::Space(""),
];

/// Definition of the obsolete asctime date format.
const ASCTIME_DATE: &[Item<'static>] = &[
    Item::Space(""),
    Item::Fixed(Fixed::ShortWeekdayName),
    Item::Space(" "),
    Item::Fixed(Fixed::ShortMonthName),
    Item::Space(" "),
    Item::Numeric(Numeric::Day, Pad::Space),
    Item::Space(" "),
    Item::Numeric(Numeric::Hour, Pad::Zero),
    Item::Literal(":"),
    Item::Numeric(Numeric::Minute, Pad::Zero),
    Item::Literal(":"),
    Item::Numeric(Numeric::Second, Pad::Zero),
    Item::Space(" "),
    Item::Numeric(Numeric::Year, Pad::Zero),
    Item::Space(""),
];

/// Parses an HTTP date.
///
/// Since all date format allow ASCII characters only, this expects a str.
/// If it cannot parse the date, it simply returns `None`.
#[allow(clippy::question_mark)] // False positive.
pub fn parse_http_date(date: &str) -> Option<DateTime<Utc>> {
    use chrono::format::{Parsed, parse};

    let mut parsed = Parsed::new();
    if parse(&mut parsed, date, IMF_FIXDATE.iter()).is_err() {
        parsed = Parsed::new();
        if parse(&mut parsed, date, RFC850_DATE.iter()).is_err() {
            parsed = Parsed::new();
            if parse(&mut parsed, date, ASCTIME_DATE.iter()).is_err() {
                return None
            }
        }
    }
    parsed.to_datetime_with_timezone(&Utc).ok()
}

pub fn format_http_date(date: DateTime<Utc>) -> String {
    date.format_with_items(IMF_FIXDATE.iter()).to_string()
}

