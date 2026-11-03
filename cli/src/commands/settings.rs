//! The account-wide UI settings that live in the Home doc's `settings`
//! map (`spec/data-model.md` "WorkspaceSettings": `show_list_counts`
//! is read from the Home doc alone and gates every workspace's
//! sidebar). IPC-only: no CLI verb maps here, since a `-w`-taking verb
//! would suggest the flag is per workspace. The caller is the macOS
//! app's Settings window (`spec/mac-window-plan.md` "Settings"), whose
//! request names no workspace and so lands on Home.

use std::io::Write;

use monoplan_core::Doc;
use serde::{Deserialize, Serialize};

use super::items::write_json;

/// What the frame reports, after applying any change it carried.
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct SettingsJson {
    pub show_list_counts: bool,
}

pub fn run(doc: &Doc, show_list_counts: Option<bool>, out: &mut dyn Write) -> anyhow::Result<()> {
    if let Some(on) = show_list_counts {
        doc.set_show_list_counts(on)?;
    }
    write_json(
        out,
        &SettingsJson {
            show_list_counts: doc.get_settings().show_list_counts,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::DocCmd;

    fn run_json(doc: &Doc, set: Option<bool>) -> SettingsJson {
        let mut out = Vec::new();
        run(doc, set, &mut out).unwrap();
        serde_json::from_slice(&out).unwrap()
    }

    #[test]
    fn reads_sets_and_reports_back() {
        let doc = Doc::new().unwrap();
        assert_eq!(
            run_json(&doc, None),
            SettingsJson {
                show_list_counts: true
            }
        );
        assert_eq!(
            run_json(&doc, Some(false)),
            SettingsJson {
                show_list_counts: false
            }
        );
        assert!(!doc.get_settings().show_list_counts);
        assert_eq!(
            run_json(&doc, None),
            SettingsJson {
                show_list_counts: false
            }
        );
    }

    /// The wire shapes the macOS app encodes by hand
    /// (`DaemonClient.swift` `Request.settings` / `.setShowListCounts`).
    #[test]
    fn wire_shape() {
        let read = DocCmd::Settings {
            show_list_counts: None,
        };
        assert_eq!(
            serde_json::to_value(&read).unwrap(),
            serde_json::json!({"Settings": {}})
        );
        let set = DocCmd::Settings {
            show_list_counts: Some(true),
        };
        assert_eq!(
            serde_json::to_value(&set).unwrap(),
            serde_json::json!({"Settings": {"show_list_counts": true}})
        );
        let back: DocCmd = serde_json::from_value(serde_json::json!({"Settings": {}})).unwrap();
        assert!(matches!(
            back,
            DocCmd::Settings {
                show_list_counts: None
            }
        ));
    }
}
