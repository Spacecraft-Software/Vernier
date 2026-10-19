// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Timestamps (Steelbore Standard §14, CLI Standard §1 rule 1).

/// The current time as ISO 8601 UTC with second precision and a `Z` suffix,
/// e.g. `2026-10-19T08:30:00Z`.
#[must_use]
pub fn now() -> String {
    // `Timestamp::strftime` formats in UTC by construction.
    jiff::Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn timestamp_is_iso8601_utc() {
        let stamp = super::now();
        let bytes = stamp.as_bytes();
        assert_eq!(stamp.len(), 20, "{stamp}");
        assert_eq!(bytes[4], b'-');
        assert_eq!(bytes[10], b'T');
        assert_eq!(bytes[19], b'Z');
    }
}
