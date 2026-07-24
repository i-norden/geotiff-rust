//! Content-Range parsing and validation shared by the blocking and async
//! HTTP range sources.

use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ParsedContentRange {
    pub start: u64,
    pub end: u64,
    pub total: Option<u64>,
}

pub(crate) fn parse_content_range(content_range: &str) -> Option<ParsedContentRange> {
    let (unit, range_and_total) = content_range.trim().split_once(' ')?;
    if !unit.eq_ignore_ascii_case("bytes") {
        return None;
    }
    let (range, total) = range_and_total.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let start = start.parse().ok()?;
    let end = end.parse().ok()?;
    if start > end {
        return None;
    }
    let total = if total == "*" {
        None
    } else {
        let total = total.parse().ok()?;
        if end >= total {
            return None;
        }
        Some(total)
    };
    Some(ParsedContentRange { start, end, total })
}

pub(crate) fn validate_content_range_header(
    content_range: Option<&str>,
    url: &str,
    expected_start: u64,
    expected_end: u64,
    expected_total: Option<u64>,
) -> Result<()> {
    let content_range = content_range
        .ok_or_else(|| Error::Other(format!("missing Content-Range header for {url}")))?;
    let parsed = parse_content_range(content_range).ok_or_else(|| {
        Error::Other(format!(
            "unable to parse Content-Range for {url}: {content_range}"
        ))
    })?;
    if parsed.start != expected_start || parsed.end != expected_end {
        return Err(Error::Other(format!(
            "unexpected Content-Range for {url}: expected bytes {expected_start}-{expected_end}, got bytes {}-{}",
            parsed.start, parsed.end
        )));
    }
    if let (Some(actual_total), Some(expected_total)) = (parsed.total, expected_total) {
        if actual_total != expected_total {
            return Err(Error::Other(format!(
                "unexpected Content-Range total for {url}: expected {expected_total}, got {actual_total}"
            )));
        }
    }
    Ok(())
}

/// Parse an object length out of a `bytes 0-0/<total>` probe response.
pub(crate) fn probe_total_from_content_range(content_range: &str, url: &str) -> Result<u64> {
    let parsed = parse_content_range(content_range).ok_or_else(|| {
        Error::Other(format!(
            "unable to parse object size from Content-Range: {content_range}"
        ))
    })?;
    if parsed.start != 0 || parsed.end != 0 {
        return Err(Error::Other(format!(
            "unexpected Content-Range for {url}: expected bytes 0-0, got bytes {}-{}",
            parsed.start, parsed.end
        )));
    }
    parsed.total.ok_or_else(|| {
        Error::Other(format!(
            "missing object size in Content-Range for {url}: bytes {}-{}/*",
            parsed.start, parsed.end
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_content_range, probe_total_from_content_range};

    #[test]
    fn parses_content_range_start_end_and_total() {
        let parsed = parse_content_range("bytes 12-34/100").unwrap();
        assert_eq!(parsed.start, 12);
        assert_eq!(parsed.end, 34);
        assert_eq!(parsed.total, Some(100));
        assert_eq!(parse_content_range("bytes 34-12/100"), None);
        assert_eq!(parse_content_range("items 12-34/100"), None);
    }

    #[test]
    fn probes_total_length_from_content_range() {
        assert_eq!(
            probe_total_from_content_range("bytes 0-0/12345", "http://example").unwrap(),
            12345
        );
        assert!(probe_total_from_content_range("bytes 1-1/5", "http://example").is_err());
    }
}
