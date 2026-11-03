//! `[[trigger]]` rules from the config, compiled.
//!
//! A trigger does up to three things with output that matches it, and they happen in different
//! places. Highlighting is done while drawing, over the rows on screen (`terminal::render`), so it
//! follows the text as it scrolls and needs nothing from the session. Notifying and badging need
//! to see every line as it arrives, scrolled past or not, so those triggers' patterns go to each
//! session as one [`RegexSet`] its reader thread matches lines against; a match comes back as
//! `SessionEvent::Triggered`, handled by `App::triggered`.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use iced::Color;
use macaroni_config::Trigger;
use macaroni_core::SessionId;
use regex::{Regex, RegexSet};

use crate::theme;

/// How long after a trigger notifies for a terminal before it can notify for it again, so a
/// stream of matching lines makes one notification rather than dozens.
const NOTIFY_COOLDOWN: Duration = Duration::from_secs(10);

/// The highlight for a color name the palette doesn't have.
const DEFAULT_HIGHLIGHT: &str = "amber";

/// How opaque a highlight is over the text's background.
const HIGHLIGHT_ALPHA: f32 = 0.35;

/// The config's triggers, ready to use.
#[derive(Debug, Default)]
pub struct Triggers {
    /// Each trigger as written, in config order.
    rules: Vec<Trigger>,
    /// Each rule's pattern compiled, in the same order, for the badge's capture groups.
    regexes: Vec<Regex>,
    /// The patterns of the rules that notify or set a badge, for the sessions to match.
    watched: Option<RegexSet>,
    /// For each pattern in `watched`, its rule's index.
    watched_rules: Vec<usize>,
    /// The highlighting rules' patterns and colors, for drawing.
    highlights: Vec<(Regex, Color)>,
    /// When each rule last notified for each terminal, for [`NOTIFY_COOLDOWN`].
    notified: HashMap<(SessionId, usize), Instant>,
}

/// What a match asks the app to do.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Fired {
    /// Notify with this title (`None` uses the tab's) and the line as the text.
    pub notify: Option<Option<String>>,
    /// Set the tab's badge to this.
    pub badge: Option<String>,
}

impl Triggers {
    /// Compiles `rules`. The config has already checked that every pattern compiles; one that
    /// somehow doesn't is skipped.
    pub fn new(rules: &[Trigger]) -> Self {
        let rules: Vec<Trigger> = rules
            .iter()
            .filter(|r| Regex::new(&r.pattern).is_ok())
            .cloned()
            .collect();
        let regexes: Vec<Regex> = rules
            .iter()
            .filter_map(|r| Regex::new(&r.pattern).ok())
            .collect();
        let watched_rules: Vec<usize> = rules
            .iter()
            .enumerate()
            .filter(|(_, r)| r.notify || r.badge.is_some())
            .map(|(i, _)| i)
            .collect();
        let watched = (!watched_rules.is_empty())
            .then(|| RegexSet::new(watched_rules.iter().map(|&i| &rules[i].pattern)).ok())
            .flatten();
        let highlights = rules
            .iter()
            .zip(&regexes)
            .filter_map(|(rule, regex)| {
                let name = rule.highlight.as_deref()?;
                let color = theme::workspace_color(Some(name))
                    .or_else(|| theme::workspace_color(Some(DEFAULT_HIGHLIGHT)))?;
                Some((
                    regex.clone(),
                    Color {
                        a: HIGHLIGHT_ALPHA,
                        ..color
                    },
                ))
            })
            .collect();
        Self {
            rules,
            regexes,
            watched,
            watched_rules,
            highlights,
            notified: HashMap::new(),
        }
    }

    /// Whether these are the same rules as `rules`, so nothing needs redoing.
    pub fn same_rules(&self, rules: &[Trigger]) -> bool {
        self.rules == rules
    }

    /// The patterns a session should match its output against, or `None` when no trigger
    /// needs to see output.
    pub fn watched(&self) -> Option<RegexSet> {
        self.watched.clone()
    }

    /// The highlighting rules' patterns and colors.
    pub fn highlights(&self) -> &[(Regex, Color)] {
        &self.highlights
    }

    /// What to do about `line` from `session` matching the `index`th pattern of
    /// [`Triggers::watched`]. A rule that notified for this session less than
    /// [`NOTIFY_COOLDOWN`] ago doesn't notify again.
    pub fn fire(&mut self, session: SessionId, index: usize, line: &str) -> Fired {
        let Some(&rule) = self.watched_rules.get(index) else {
            return Fired::default();
        };
        let (trigger, regex) = (&self.rules[rule], &self.regexes[rule]);
        let badge = trigger.badge.as_deref().and_then(|template| {
            let captures = regex.captures(line)?;
            let mut badge = String::new();
            captures.expand(template, &mut badge);
            Some(badge)
        });
        let now = Instant::now();
        let notify = trigger.notify
            && self
                .notified
                .get(&(session, rule))
                .is_none_or(|at| now.duration_since(*at) >= NOTIFY_COOLDOWN);
        if notify {
            self.notified.insert((session, rule), now);
        }
        Fired {
            notify: notify.then(|| (!trigger.name.is_empty()).then(|| trigger.name.clone())),
            badge,
        }
    }

    /// Forgets a closed terminal's cooldowns.
    pub fn forget(&mut self, session: SessionId) {
        self.notified.retain(|(s, _), _| *s != session);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trigger matching `pattern` that notifies and sets `badge`.
    fn rule(pattern: &str, badge: Option<&str>) -> Trigger {
        Trigger {
            pattern: pattern.to_owned(),
            notify: true,
            badge: badge.map(str::to_owned),
            ..Trigger::default()
        }
    }

    #[test]
    fn only_notifying_and_badging_rules_are_watched() {
        let highlight = Trigger {
            pattern: "warn".to_owned(),
            highlight: Some("no-such-color".to_owned()),
            ..Trigger::default()
        };
        let triggers = Triggers::new(&[highlight, rule("error", None)]);
        let watched = triggers.watched().unwrap();
        assert_eq!(watched.patterns(), ["error"]);
        assert_eq!(triggers.highlights().len(), 1);
        assert_eq!(triggers.highlights()[0].1.a, HIGHLIGHT_ALPHA);
        assert!(Triggers::new(&[]).watched().is_none());
    }

    #[test]
    fn badges_fill_groups_and_notifying_cools_down() {
        let mut triggers = Triggers::new(&[rule(r"FAILED: (\w+)", Some("$1 failed"))]);
        let session = SessionId(1);
        let first = triggers.fire(session, 0, "FAILED: build");
        assert_eq!(first.badge.as_deref(), Some("build failed"));
        assert_eq!(first.notify, Some(None));
        let again = triggers.fire(session, 0, "FAILED: lint");
        assert_eq!(again.badge.as_deref(), Some("lint failed"));
        assert_eq!(again.notify, None);
        assert_eq!(
            triggers.fire(SessionId(2), 0, "FAILED: x").notify,
            Some(None)
        );
        triggers.forget(session);
        assert_eq!(triggers.fire(session, 0, "FAILED: x").notify, Some(None));
    }
}
