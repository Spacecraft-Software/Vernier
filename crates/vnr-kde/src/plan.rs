// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Output priorities for `kde_output_configuration_v2.set_priority`.

/// Assigns Plasma priorities: 1 for the primary output, then the other
/// enabled outputs in their current order, then newly enabled ones; disabled
/// outputs get 0, as the protocol requires.
///
/// `outputs` lists every output with whether it will be enabled; `order` is
/// the current priority order; `primary` the output that should come first,
/// or `None` to keep the current first one.
#[must_use]
pub fn priorities<'a>(
    order: &[String],
    outputs: &[(&'a str, bool)],
    primary: Option<&'a str>,
) -> Vec<(&'a str, u32)> {
    let enabled = |name: &str| outputs.iter().any(|(n, on)| *n == name && *on);
    let mut ranked: Vec<&'a str> = Vec::new();
    if let Some(primary) = primary.filter(|p| enabled(p)) {
        ranked.push(primary);
    }
    for name in order {
        if let Some((n, _)) = outputs.iter().find(|(n, on)| n == name && *on)
            && !ranked.contains(n)
        {
            ranked.push(n);
        }
    }
    for (name, on) in outputs {
        if *on && !ranked.contains(name) {
            ranked.push(name);
        }
    }
    let mut result: Vec<(&'a str, u32)> = ranked
        .iter()
        .zip(1u32..)
        .map(|(name, priority)| (*name, priority))
        .collect();
    result.extend(
        outputs
            .iter()
            .filter(|(_, on)| !on)
            .map(|(name, _)| (*name, 0)),
    );
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    #[test]
    fn primary_goes_first_and_the_rest_keep_their_order() {
        let outputs = [("eDP-1", true), ("DP-1", true), ("DP-2", true)];
        let got = priorities(&order(&["eDP-1", "DP-1", "DP-2"]), &outputs, Some("DP-2"));
        assert_eq!(got, [("DP-2", 1), ("eDP-1", 2), ("DP-1", 3)]);
    }

    #[test]
    fn disabled_outputs_get_zero_and_new_ones_go_last() {
        let outputs = [("eDP-1", false), ("DP-1", true), ("HDMI-A-1", true)];
        let got = priorities(&order(&["eDP-1", "DP-1"]), &outputs, None);
        assert_eq!(got, [("DP-1", 1), ("HDMI-A-1", 2), ("eDP-1", 0)]);
    }

    #[test]
    fn a_disabled_primary_is_ignored() {
        let outputs = [("eDP-1", true), ("DP-1", false)];
        let got = priorities(&order(&["DP-1", "eDP-1"]), &outputs, Some("DP-1"));
        assert_eq!(got, [("eDP-1", 1), ("DP-1", 0)]);
    }
}
